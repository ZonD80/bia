package main

import (
	"cmp"
	"context"
	"errors"
	"fmt"
	"net/url"
	"sort"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"go.mau.fi/util/exhttp"
	"go.mau.fi/util/jsontime"
	"go.mau.fi/util/ptr"

	"go.mau.fi/mautrix-meta/pkg/instameow"
	"go.mau.fi/mautrix-meta/pkg/instameow/slidetypes"
	"go.mau.fi/mautrix-meta/pkg/messagix/cookies"
	"go.mau.fi/mautrix-meta/pkg/messagix/dgw"
	"go.mau.fi/mautrix-meta/pkg/messagix/httpclient"
	"go.mau.fi/mautrix-meta/pkg/messagix/methods"
	"go.mau.fi/mautrix-meta/pkg/messagix/types"
)

// Instagram chat ids are the thread key: for a DM that is the other person's id,
// for a group the group's id. Requests want the thread's short or long id instead;
// both are looked up once and kept (Chat.WAJID short, Chat.IGThread long).
type instagram struct {
	st      *Store
	rootCtx context.Context

	mu      sync.Mutex
	sess    *Session
	cli     *instameow.Client
	cancel  context.CancelFunc
	state   string
	errText string
	fbid    int64
	gen     uint64
	ready   chan struct{}

	retrying atomic.Bool
}

func newInstagram(ctx context.Context, st *Store) *instagram {
	ig := &instagram{st: st, rootCtx: ctx, state: "logged_out", ready: make(chan struct{})}
	if sess, _ := st.LoadSession(); sess != nil {
		ig.sess = sess
		ig.fbid, _ = strconv.ParseInt(sess.ID, 10, 64)
		ig.state = "disconnected"
	}
	return ig
}

func (ig *instagram) setState(state, errText string) {
	ig.mu.Lock()
	changed := ig.state != state || ig.errText != errText
	ig.state, ig.errText = state, errText
	ig.mu.Unlock()
	if changed {
		emit("status", map[string]any{"state": state, "error": errText})
	}
}

func (ig *instagram) Status() map[string]any {
	ig.mu.Lock()
	defer ig.mu.Unlock()
	st := map[string]any{"state": ig.state}
	if ig.errText != "" {
		st["error"] = ig.errText
	}
	if ig.sess != nil {
		st["id"], st["name"], st["username"] = ig.sess.ID, ig.sess.Name, ig.sess.Username
	}
	return st
}

func igCookies(values map[string]string) *cookies.Cookies {
	c := &cookies.Cookies{Platform: types.Instagram}
	vals := make(map[cookies.MetaCookieName]string, len(values))
	for k, v := range values {
		vals[cookies.MetaCookieName(k)] = v
	}
	c.UpdateValues(vals)
	return c
}

func (ig *instagram) newClient(c *cookies.Cookies, gen uint64) *instameow.Client {
	return instameow.NewClient(instameow.ClientParams{
		Cookies:  c,
		Log:      log.With().Str("component", "instameow").Logger(),
		Settings: exhttp.SensibleClientSettings,
		EventHandler: func(ctx context.Context, evt slidetypes.ClientEvent) error {
			return ig.onEvent(gen, evt)
		},
	})
}

func igLoginError(err error) error {
	switch {
	case errors.Is(err, httpclient.ErrTokenInvalidated), errors.Is(err, httpclient.ErrUserIDIsZero):
		return fmt.Errorf("Instagram did not accept the session (logged out or the cookies are old): log in again in the browser, then call login again")
	case errors.Is(err, httpclient.ErrCheckpointRequired), errors.Is(err, httpclient.ErrChallengeRequired):
		return fmt.Errorf("Instagram wants a security check first: open https://www.instagram.com in the browser, finish it, then call login again (%s)", httpclient.GetErrorRedirectURL(err))
	case errors.Is(err, httpclient.ErrConsentRequired):
		return fmt.Errorf("Instagram asks for consent first: open https://www.instagram.com in the browser, accept, then call login again")
	case errors.Is(err, httpclient.ErrAccountSuspended):
		return fmt.Errorf("this Instagram account is suspended: %s", httpclient.GetErrorRedirectURL(err))
	case errors.Is(err, httpclient.ErrRateLimited):
		return fmt.Errorf("Instagram is rate limiting this account; wait a while and try again")
	}
	return err
}

func (ig *instagram) Login(ctx context.Context, p LoginParams) (map[string]any, error) {
	c := igCookies(p.Cookies)
	if missing := c.GetMissingCookieNames(); len(missing) > 0 {
		return nil, fmt.Errorf("missing cookies %v: log in to Instagram in the browser first", missing)
	}
	// stop the old connection first: stop() moves the generation on, and start()
	// ignores a client made for an older one (it was never connected)
	ig.stop()
	ig.mu.Lock()
	ig.gen++
	gen := ig.gen
	ig.mu.Unlock()
	cli := ig.newClient(c, gen)
	viewer, mailbox, err := cli.LoadIndex(ctx)
	if err == nil && cli.GetOwnFBID() == 0 {
		err = errors.New("Instagram did not say who is logged in")
	}
	if err != nil {
		ig.reconnectSaved()
		return nil, igLoginError(err)
	}
	fbid := cli.GetOwnFBID()
	old := ig.sess
	sess := &Session{
		Platform: "instagram", Cookies: p.Cookies, ID: strconv.FormatInt(fbid, 10), IGID: viewer.ID,
		Name: viewer.GetName(), Username: viewer.GetUsername(), Since: time.Now().Unix(),
	}
	if old != nil && old.ID != sess.ID {
		_ = ig.st.Wipe(ctx)
	}
	if err = ig.st.SaveSession(sess); err != nil {
		return nil, err
	}
	ig.mu.Lock()
	ig.sess, ig.fbid = sess, fbid
	ig.mu.Unlock()
	_ = ig.st.PutUser(ctx, &User{ID: sess.ID, Name: sess.Name, Username: sess.Username, IGID: viewer.ID})
	ig.start(cli, gen, mailbox)
	// report the connection, not just the accepted cookies
	select {
	case <-ig.readyCh():
	case <-time.After(30 * time.Second):
	case <-ctx.Done():
	}
	st := ig.Status()
	return map[string]any{"id": sess.ID, "name": sess.Name, "username": sess.Username, "state": st["state"], "error": st["error"]}, nil
}

// reconnectSaved brings a saved session back after a failed new login.
func (ig *instagram) reconnectSaved() {
	ig.mu.Lock()
	saved := ig.sess != nil
	ig.mu.Unlock()
	if !saved {
		return
	}
	go func() {
		ctx, cancel := context.WithTimeout(ig.rootCtx, 2*time.Minute)
		defer cancel()
		ig.connectSaved(ctx)
	}()
}

func (ig *instagram) Restore(ctx context.Context) (map[string]any, error) {
	ig.mu.Lock()
	sess, cli, state := ig.sess, ig.cli, ig.state
	ig.mu.Unlock()
	if sess == nil {
		return ig.Status(), nil
	}
	if cli == nil || state == "error" || state == "disconnected" {
		ig.connectSaved(ctx)
	}
	select {
	case <-ig.readyCh():
	case <-time.After(20 * time.Second):
	case <-ctx.Done():
	}
	return ig.Status(), nil
}

func (ig *instagram) readyCh() chan struct{} {
	ig.mu.Lock()
	defer ig.mu.Unlock()
	return ig.ready
}

func (ig *instagram) connectSaved(ctx context.Context) {
	ig.mu.Lock()
	sess := ig.sess
	ig.gen++
	gen := ig.gen
	ig.mu.Unlock()
	if sess == nil {
		return
	}
	ig.setState("connecting", "")
	cli := ig.newClient(igCookies(sess.Cookies), gen)
	_, mailbox, err := cli.LoadIndex(ctx)
	if err != nil {
		log.Warn().Err(err).Msg("load index")
		if isLoggedOut(err) {
			ig.loggedOut("Instagram ended the session (logged out elsewhere or the cookies expired)")
			return
		}
		ig.setState("error", igLoginError(err).Error())
		ig.retryLater()
		return
	}
	ig.start(cli, gen, mailbox)
}

func (ig *instagram) retryLater() {
	if !ig.retrying.CompareAndSwap(false, true) {
		return
	}
	go func() {
		defer ig.retrying.Store(false)
		delay := 15 * time.Second
		for i := 0; i < 40; i++ {
			select {
			case <-time.After(delay):
			case <-ig.rootCtx.Done():
				return
			}
			st := ig.Status()["state"]
			if st == "ready" || st == "logged_out" {
				return
			}
			ctx, cancel := context.WithTimeout(ig.rootCtx, 2*time.Minute)
			ig.connectSaved(ctx)
			cancel()
			if ig.Status()["state"] == "ready" {
				return
			}
			delay = min(delay*2, 10*time.Minute)
		}
	}()
}

func (ig *instagram) start(cli *instameow.Client, gen uint64, mailbox *slidetypes.Mailbox) {
	ig.mu.Lock()
	if ig.gen != gen {
		ig.mu.Unlock()
		return
	}
	if ig.cancel != nil {
		ig.cancel()
	}
	if ig.cli != nil && ig.cli != cli {
		old := ig.cli
		go old.Disconnect()
	}
	ctx, cancel := context.WithCancel(ig.rootCtx)
	ig.cancel, ig.cli = cancel, cli
	ig.fbid = cli.GetOwnFBID()
	ig.ready = make(chan struct{})
	ig.mu.Unlock()
	ig.setState("connecting", "")
	if mailbox != nil {
		ig.handleMailbox(ctx, mailbox)
	}
	go cli.Connect(ctx)
	go ig.saveCookiesLoop(ctx, gen)
}

func (ig *instagram) current(gen uint64) bool {
	ig.mu.Lock()
	defer ig.mu.Unlock()
	return ig.gen == gen
}

func (ig *instagram) saveCookiesLoop(ctx context.Context, gen uint64) {
	t := time.NewTicker(10 * time.Minute)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-t.C:
			ig.saveCookies(gen)
		}
	}
}

func (ig *instagram) saveCookies(gen uint64) {
	ig.mu.Lock()
	if ig.gen != gen || ig.cli == nil || ig.sess == nil {
		ig.mu.Unlock()
		return
	}
	all := ig.cli.GetCookies().GetAll()
	sess := *ig.sess
	ig.mu.Unlock()
	vals := make(map[string]string, len(all))
	for k, v := range all {
		vals[string(k)] = v
	}
	if vals["sessionid"] == "" {
		return
	}
	sess.Cookies = vals
	if ig.st.SaveSession(&sess) == nil {
		ig.mu.Lock()
		if ig.gen == gen {
			ig.sess = &sess
		}
		ig.mu.Unlock()
	}
}

func (ig *instagram) onEvent(gen uint64, raw slidetypes.ClientEvent) error {
	if !ig.current(gen) {
		return nil
	}
	ctx := ig.rootCtx
	switch evt := raw.(type) {
	case *slidetypes.Connected:
		ig.setState("ready", "")
		ig.mu.Lock()
		select {
		case <-ig.ready:
		default:
			close(ig.ready)
		}
		ig.mu.Unlock()
		ig.saveCookies(gen)
		go reads.flush(ctx)
	case *slidetypes.Disconnected:
		if dgw.IsUnauthorized(evt.Error) || errors.Is(evt.Error, instameow.ErrUserIDMissing) {
			ig.loggedOut("Instagram ended the session")
			return fmt.Errorf("unauthorized")
		}
		ig.setState("reconnecting", "")
		if evt.FailureCount > 5 && errors.Is(evt.Error, instameow.ErrMainStreamClosed) {
			ig.setState("error", evt.Error.Error())
			go ig.fullReconnect(gen)
			return fmt.Errorf("main stream closed")
		}
	case *slidetypes.AuthError:
		if isLoggedOut(evt.Error) {
			ig.loggedOut("Instagram ended the session")
		} else {
			log.Warn().Err(evt.Error).Msg("instagram auth error")
		}
	case *slidetypes.ResnapshotRequired:
		go ig.fullReconnect(gen)
	case *slidetypes.Delta:
		ig.handleDelta(ctx, evt)
	}
	return nil
}

func (ig *instagram) fullReconnect(gen uint64) {
	if !ig.current(gen) {
		return
	}
	ctx, cancel := context.WithTimeout(ig.rootCtx, 2*time.Minute)
	defer cancel()
	ig.connectSaved(ctx)
}

func (ig *instagram) SessionGone(reason string) {
	if ig.Status()["state"] != "logged_out" {
		ig.loggedOut(reason)
	}
}

func (ig *instagram) loggedOut(reason string) {
	ig.stop()
	ig.mu.Lock()
	ig.sess = nil
	ig.mu.Unlock()
	_ = ig.st.DeleteSession()
	ig.setState("logged_out", reason)
	emit("logged_out", map[string]any{"reason": reason})
}

func (ig *instagram) stop() {
	ig.mu.Lock()
	ig.gen++
	cli, cancel := ig.cli, ig.cancel
	ig.cli, ig.cancel = nil, nil
	ig.mu.Unlock()
	if cancel != nil {
		cancel()
	}
	if cli != nil {
		cli.Disconnect()
	}
}

func (ig *instagram) Close() {
	ig.stop()
}

func (ig *instagram) Logout(ctx context.Context) error {
	ig.stop()
	ig.mu.Lock()
	ig.sess, ig.fbid = nil, 0
	ig.mu.Unlock()
	if err := ig.st.Wipe(ctx); err != nil {
		return err
	}
	ig.setState("logged_out", "")
	return nil
}

func (ig *instagram) client() (*instameow.Client, error) {
	ig.mu.Lock()
	cli, sess := ig.cli, ig.sess
	ig.mu.Unlock()
	if sess == nil {
		return nil, ErrNotLoggedIn
	}
	if cli == nil {
		return nil, fmt.Errorf("Instagram is not connected (state %s; it reconnects by itself): try again in a minute. If instagram_status keeps showing it down, open https://www.instagram.com/direct/inbox/ in bia's Chrome (a security check may wait there), then call instagram_login again", ig.Status()["state"])
	}
	return cli, nil
}

func (ig *instagram) readyClient(ctx context.Context) (*instameow.Client, error) {
	if _, err := ig.client(); err != nil {
		return nil, err
	}
	select {
	case <-ig.readyCh():
	case <-time.After(time.Minute):
		return nil, errors.New("Instagram is still connecting: try again in a minute. If instagram_status keeps showing it down, open https://www.instagram.com/direct/inbox/ in bia's Chrome, then call instagram_login again")
	case <-ctx.Done():
		return nil, ctx.Err()
	}
	return ig.client()
}

func (ig *instagram) ownID() string {
	ig.mu.Lock()
	defer ig.mu.Unlock()
	return strconv.FormatInt(ig.fbid, 10)
}

// --- incoming ---

func (ig *instagram) putUser(ctx context.Context, u *slidetypes.User) {
	if u == nil || u.InteropMessagingUserFBID == 0 {
		return
	}
	_ = ig.st.PutUser(ctx, &User{ID: strconv.FormatInt(u.InteropMessagingUserFBID, 10), Name: u.FullName, Username: u.Username, IGID: cmp.Or(u.ID, u.PK)})
}

// putThread saves a thread and returns its chat id.
func (ig *instagram) putThread(ctx context.Context, t *slidetypes.ThreadInfo) string {
	if t == nil || t.ThreadKey == 0 {
		return ""
	}
	own := ig.ownID()
	for _, u := range t.Users {
		ig.putUser(ctx, u)
	}
	id := strconv.FormatInt(t.ThreadKey, 10)
	c := &Chat{ID: id, Kind: kindStr(t.IsGroup), FBKey: t.ThreadKey, WAJID: t.ID, IGThread: t.ThreadID,
		Title: t.ThreadTitle, LastTS: t.LastActivityTimestampMS.Unix(), Folder: t.Folder,
		Request: t.Folder == "pending" || t.SystemFolder == "pending"}
	if !t.IsGroup {
		c.Peer = id
		for _, u := range t.Users {
			if u.InteropMessagingUserFBID != 0 && strconv.FormatInt(u.InteropMessagingUserFBID, 10) != own {
				c.Peer = strconv.FormatInt(u.InteropMessagingUserFBID, 10)
			}
		}
	}
	_ = ig.st.PutChat(ctx, c)
	if t.IsGroup {
		for _, u := range t.Users {
			if u.InteropMessagingUserFBID != 0 {
				uid := strconv.FormatInt(u.InteropMessagingUserFBID, 10)
				role := "member"
				for _, a := range t.AdminUserIDs {
					if a == u.ID || a == u.PK {
						role = "admin"
					}
				}
				ig.st.PutKV(ctx, "member:"+id+":"+uid, role)
			}
		}
	}
	title := t.ThreadTitle
	if !t.IsGroup {
		title = cmp.Or(ig.st.UserName(ctx, c.Peer), title)
	}
	emit("chat", map[string]any{"chat": id, "title": title, "group": t.IsGroup, "request": c.Request})
	return id
}

func (ig *instagram) handleMailbox(ctx context.Context, mb *slidetypes.Mailbox) {
	threads := mb.PinnedThreadsV2
	for _, e := range mb.ThreadsByFolder.Edges {
		threads = append(threads, e.Node)
	}
	own := ig.ownID()
	var batch []map[string]any
	for _, wt := range threads {
		t := wt.AsIGDirectThread
		chat := ig.putThread(ctx, t)
		if chat == "" || t.SlideMessages == nil {
			continue
		}
		for _, e := range t.SlideMessages.Edges {
			if ev := ig.message(ctx, chat, t.IsGroup, t.ThreadTitle, e.Node, own); ev != nil {
				batch = append(batch, ev)
			}
		}
	}
	if len(batch) > 0 {
		emit("backlog", map[string]any{"messages": batch, "initial": true})
	}
}

// chatForShort finds (or fetches) the chat of a thread's short id.
func (ig *instagram) chatForShort(ctx context.Context, short string) (*Chat, error) {
	if short == "" {
		return nil, errors.New("no thread id")
	}
	if c, err := ig.st.ChatByWAJID(ctx, short); err != nil || c != nil {
		return c, err
	}
	cli, err := ig.client()
	if err != nil {
		return nil, err
	}
	resp, err := cli.GetThread(ctx, slidetypes.MakeGetThreadInfoRequest(short))
	if err != nil {
		return nil, err
	}
	if resp.ThreadInfo.AsIGDirectThread == nil {
		return nil, fmt.Errorf("thread %s not found", short)
	}
	id := ig.putThread(ctx, resp.ThreadInfo.AsIGDirectThread)
	return ig.st.GetChat(ctx, id)
}

func (ig *instagram) handleDelta(ctx context.Context, d *slidetypes.Delta) {
	defer func() {
		if v := recover(); v != nil {
			log.Error().Interface("panic", v).Str("delta", d.TypeName).Msg("panic in delta")
		}
	}()
	if nm, ok := d.Data.(*slidetypes.NewMessageEvent); ok && d.ThreadIGID == "" && nm.Message != nil {
		d.ThreadIGID = nm.Message.ThreadFBID
	}
	switch d.Data.(type) {
	case slidetypes.UnknownEvent, slidetypes.IgnoredEvent, *slidetypes.MarkReadEvent, *slidetypes.MarkUnreadEvent, *slidetypes.ReadReceiptEvent:
		return
	}
	c, err := ig.chatForShort(ctx, d.ThreadIGID)
	if err != nil || c == nil {
		log.Debug().Err(err).Str("thread", d.ThreadIGID).Str("delta", d.TypeName).Msg("delta for unknown thread")
		return
	}
	own := ig.ownID()
	switch evt := d.Data.(type) {
	case *slidetypes.NewMessageEvent:
		if ev := ig.message(ctx, c.ID, c.Kind == "group", c.Title, evt.Message, own); ev != nil {
			emit("message", ev)
		}
	case *slidetypes.AdminMessageEvent:
		if ev := ig.message(ctx, c.ID, c.Kind == "group", c.Title, evt.Message, own); ev != nil {
			ev["admin"] = true
			emit("message", ev)
		}
	case *slidetypes.EditMessageEvent:
		ref, _ := ig.st.GetMsg(ctx, evt.MessageID)
		ev := map[string]any{"chat": c.ID, "id": evt.MessageID, "text": evt.TextBody, "ts": evt.SlideEditHistoryEntry.TimestampMS.Unix()}
		if ref != nil {
			ev["sender"], ev["me"], ev["sender_name"] = ref.Sender, ref.Me, ig.st.UserName(ctx, ref.Sender)
			ev["old"], ev["sent_ts"] = ref.Text, ref.TS/1000
		}
		ig.st.SetMsgText(ctx, evt.MessageID, evt.TextBody)
		emit("edit", ev)
	case *slidetypes.DeleteMessageEvent:
		ref, _ := ig.st.GetMsg(ctx, evt.MessageID)
		ev := map[string]any{"chat": c.ID, "id": evt.MessageID}
		if ref != nil {
			ev["sender"], ev["me"] = ref.Sender, ref.Me
		}
		ig.st.DeleteMsg(ctx, evt.MessageID)
		emit("delete", ev)
	case *slidetypes.CreateReactionEvent:
		s := strconv.FormatInt(evt.Reaction.SenderFBID, 10)
		emit("reaction", map[string]any{"chat": c.ID, "id": evt.MessageID, "sender": s, "sender_name": ig.st.UserName(ctx, s),
			"me": s == own, "emoji": evt.Reaction.Reaction})
	case *slidetypes.DeleteReactionEvent:
		s := strconv.FormatInt(evt.Reaction.SenderFBID, 10)
		emit("reaction", map[string]any{"chat": c.ID, "id": evt.MessageID, "sender": s, "sender_name": ig.st.UserName(ctx, s),
			"me": s == own, "emoji": ""})
	case *slidetypes.UpdateThreadNameEvent:
		_ = ig.st.PutChat(ctx, &Chat{ID: c.ID, Kind: c.Kind, Title: evt.ThreadName})
		emit("chat", map[string]any{"chat": c.ID, "title": evt.ThreadName, "group": c.Kind == "group"})
	case *slidetypes.DeleteThreadEvent:
		_ = ig.st.DeleteChat(ctx, c.ID)
		emit("chat_deleted", map[string]any{"chat": c.ID})
	case *slidetypes.ParticipantJoinEvent:
		if evt.Thread.AsIGDirectThread != nil {
			for _, u := range evt.Thread.AsIGDirectThread.Users {
				ig.putUser(ctx, u)
				if u.InteropMessagingUserFBID != 0 {
					ig.st.PutKV(ctx, "member:"+c.ID+":"+strconv.FormatInt(u.InteropMessagingUserFBID, 10), "member")
				}
			}
		}
	case *slidetypes.ParticipantLeaveEvent:
		ig.st.DelKV(ctx, "member:"+c.ID+":"+strconv.FormatInt(evt.LeftParticipantFBID, 10))
	case *slidetypes.UpdateThreadFolderEvent:
		_ = ig.st.PutChat(ctx, &Chat{ID: c.ID, Kind: c.Kind, Folder: evt.Folder, Request: evt.Folder == "pending"})
	}
}

func (ig *instagram) attachment(a *slidetypes.Attachment, kind string) Attachment {
	if a == nil {
		return Attachment{Kind: kind, Note: "no download link"}
	}
	u := cmp.Or(a.AttachmentCDNURL, a.PreviewCDNURL)
	if u == "" {
		return Attachment{Kind: kind, Note: "no download link"}
	}
	return Attachment{Kind: kind, Ref: urlRef(u, "")}
}

// message turns an Instagram message into an event map; nil skips it.
func (ig *instagram) message(ctx context.Context, chat string, group bool, title string, msg *slidetypes.Message, own string) map[string]any {
	if msg == nil {
		return nil
	}
	if msg.Sender != nil {
		ig.putUser(ctx, &msg.Sender.UserDict)
		if msg.Sender.Name != "" && msg.SenderFBID != 0 {
			_ = ig.st.PutUser(ctx, &User{ID: strconv.FormatInt(msg.SenderFBID, 10), Name: msg.Sender.Name, IGID: msg.Sender.IGID})
		}
	}
	sender := strconv.FormatInt(msg.SenderFBID, 10)
	text := msg.TextBody
	var atts []Attachment
	switch c := msg.Content.Content.(type) {
	case *slidetypes.MessageContentText:
		text = cmp.Or(c.TextBody, text)
	case *slidetypes.MessageContentAdminText:
		var parts []string
		for _, f := range c.TextFragments {
			parts = append(parts, f.Plaintext)
		}
		text = cmp.Or(strings.Join(parts, ""), msg.IGDSnippet)
	case *slidetypes.MessageContentImage:
		for _, a := range c.Attachments {
			atts = append(atts, ig.attachment(a, "image"))
		}
	case *slidetypes.MessageContentVideo:
		for _, a := range c.Videos {
			atts = append(atts, ig.attachment(a, "video"))
		}
	case *slidetypes.MessageContentMultiMedia:
		for _, a := range c.Attachments {
			kind := "image"
			if a != nil && a.DashManifest != "" {
				kind = "video"
			}
			atts = append(atts, ig.attachment(a, kind))
		}
	case *slidetypes.MessageContentAudio:
		for _, a := range c.AudioAttachments {
			if a == nil || a.AttachmentCDNURL == "" {
				atts = append(atts, Attachment{Kind: "voice", Note: "no download link"})
				continue
			}
			atts = append(atts, Attachment{Kind: "voice", Ref: urlRef(a.AttachmentCDNURL, ""), Seconds: float64(a.PlayableDurationMS) / 1000})
		}
	case *slidetypes.MessageContentAnimatedMedia:
		for _, a := range c.AnimatedMedia {
			if a == nil {
				continue
			}
			switch {
			case a.AttachmentWebpURL != "" && (a.IsSticker || a.AttachmentMP4URL == ""):
				atts = append(atts, Attachment{Kind: "sticker", Name: a.AltText, Ref: urlRef(a.AttachmentWebpURL, "image/webp")})
			case a.AttachmentMP4URL != "":
				atts = append(atts, Attachment{Kind: "gif", Name: a.AltText, Ref: urlRef(a.AttachmentMP4URL, "video/mp4")})
			case a.PreviewCDNURL != "":
				atts = append(atts, Attachment{Kind: "image", Ref: urlRef(a.PreviewCDNURL, "")})
			}
		}
	case *slidetypes.MessageContentRavenImage:
		if c.Attachment == nil {
			atts = append(atts, Attachment{Kind: "image", Note: "view-once photo: only viewable in the Instagram app"})
		} else {
			atts = append(atts, ig.attachment(c.Attachment, "image"))
		}
	case *slidetypes.MessageContentRavenVideo:
		if c.Attachment == nil {
			atts = append(atts, Attachment{Kind: "video", Note: "view-once video: only viewable in the Instagram app"})
		} else {
			atts = append(atts, ig.attachment(c.Attachment, "video"))
		}
	case *slidetypes.MessageContentSticker:
		if c.PreviewURL != "" {
			atts = append(atts, Attachment{Kind: "sticker", Name: c.AltText, Ref: urlRef(c.PreviewURL, "")})
		}
	case *slidetypes.MessageContentMusicSticker:
		text = strings.TrimSpace(fmt.Sprintf("[music] %s — %s", c.AudioTrack.Title, c.AudioTrack.DisplayArtist))
	case *slidetypes.MessageContentXMA:
		text = xmaShare(c)
	case *slidetypes.MessageContentAIRichResponse:
		text = c.UnifiedResponse
	case *slidetypes.MessageContentAISearchResponse:
		text = c.MessageTextBody
	case slidetypes.MessageContentUnknown:
		if text == "" {
			text = "[a message type bia cannot show; open it in the Instagram app]"
		}
	}
	if text == "" && len(atts) == 0 {
		return nil
	}
	ts := msg.TimestampMS.Unix()
	ev := map[string]any{
		"chat": chat, "group": group, "id": msg.ID, "ts": ts, "sender": sender,
		"sender_name": ig.st.UserName(ctx, sender), "me": sender == own, "text": text,
	}
	if group && title != "" {
		ev["chat_title"] = title
	}
	if len(atts) > 0 {
		ev["attachments"] = atts
	}
	if msg.RepliedToMessageID != "" {
		reply := map[string]any{"id": msg.RepliedToMessageID}
		if rm := msg.RepliedToMessage; rm != nil {
			rs := strconv.FormatInt(rm.SenderFBID, 10)
			reply["sender"], reply["sender_name"], reply["me"] = rs, ig.st.UserName(ctx, rs), rs == own
			reply["text"] = cmp.Or(rm.TextBody, rm.IGDSnippet)
		} else if ref, _ := ig.st.GetMsg(ctx, msg.RepliedToMessageID); ref != nil {
			reply["sender"], reply["text"], reply["me"] = ref.Sender, ref.Text, ref.Me
		}
		ev["reply"] = reply
	}
	if len(msg.Mentions) > 0 {
		var names []string
		for _, mt := range msg.Mentions {
			if mt.UserFBID == 0 || mt.ProfileRangeType != slidetypes.ProfileRangeTypeProfile {
				continue
			}
			id := strconv.FormatInt(mt.UserFBID, 10)
			names = append(names, fmt.Sprintf("%s (%s)", ig.st.UserName(ctx, id), id))
		}
		if len(names) > 0 {
			ev["mentions"] = names
		}
	}
	_ = ig.st.PutMsg(ctx, &MsgRef{ID: msg.ID, Chat: chat, Sender: sender, Me: sender == own, TS: msg.TimestampMS.UnixMilli(), Text: shownText(text, atts)})
	_ = ig.st.PutChat(ctx, &Chat{ID: chat, Kind: kindStr(group), LastTS: ts})
	return ev
}

// xmaShare describes a shared post, reel, story or link.
func xmaShare(c *slidetypes.MessageContentXMA) string {
	x := c.XMA
	body := c.XMATextBody
	if x == nil {
		return body
	}
	var parts []string
	for _, s := range []string{x.HeaderTitleText, cmp.Or(x.TitleText, x.XMATitle), x.CaptionBodyText} {
		if s = strings.TrimSpace(s); s != "" {
			parts = append(parts, s)
		}
	}
	link := x.TargetURL
	if link == "" {
		for _, b := range x.CTAButtons {
			if strings.HasPrefix(b.ActionURL, "http") {
				link = b.ActionURL
				break
			}
		}
	}
	if link != "" {
		link = removeLPHP(link)
		parts = append(parts, link)
	}
	shared := ""
	if len(parts) > 0 {
		label := "shared"
		if x.EyebrowText != "" {
			label = "reply: " + x.EyebrowText
		}
		shared = "[" + label + "] " + strings.Join(parts, " — ")
	}
	if body == "" {
		return shared
	}
	if shared == "" || (link != "" && strings.Contains(body, link)) {
		return body
	}
	return body + "\n" + shared
}

// --- targets ---

type igTarget struct {
	key   int64
	id    string
	chat  *Chat
	group bool
	short string
	long  string
}

func (ig *instagram) target(ctx context.Context, raw string) (*igTarget, error) {
	id := strings.TrimSpace(strings.TrimPrefix(strings.TrimSpace(raw), "instagram:"))
	if id == "" {
		return nil, errors.New("chat required")
	}
	if !isDigits(id) {
		r, err := ig.Resolve(ctx, ResolveParams{Query: id})
		if err != nil {
			return nil, err
		}
		id = fmt.Sprint(r["chat"])
	}
	c, err := ig.st.GetChat(ctx, id)
	if err != nil {
		return nil, err
	}
	key, _ := strconv.ParseInt(id, 10, 64)
	t := &igTarget{key: key, id: id, chat: c}
	if c != nil {
		t.group, t.short, t.long = c.Kind == "group", c.WAJID, c.IGThread
	}
	return t, nil
}

// ids makes sure the short and long thread ids are known.
func (ig *instagram) ids(ctx context.Context, t *igTarget) error {
	if t.short != "" && t.long != "" {
		return nil
	}
	cli, err := ig.client()
	if err != nil {
		return err
	}
	res, err := cli.FetchThreadID(ctx, t.key)
	if err != nil {
		return err
	}
	t.short, t.long = res.ShortID, res.LongID
	kind := "dm"
	if t.group {
		kind = "group"
	}
	_ = ig.st.PutChat(ctx, &Chat{ID: t.id, Kind: kind, FBKey: t.key, WAJID: t.short, IGThread: t.long})
	return nil
}

// --- sending ---

func (ig *instagram) Send(ctx context.Context, p SendParams) (map[string]any, error) {
	if strings.TrimSpace(p.Text) == "" && len(p.Files) == 0 {
		return nil, errors.New("text or files required")
	}
	cli, err := ig.readyClient(ctx)
	if err != nil {
		return nil, err
	}
	t, err := ig.target(ctx, p.Chat)
	if err != nil {
		return nil, err
	}
	err = ig.ids(ctx, t)
	newDM := errors.Is(err, instameow.ErrThreadNotFound) && !t.group
	if err != nil && !newDM {
		return nil, err
	}
	var reply *string
	if p.ReplyTo != "" {
		reply = ptr.Ptr(p.ReplyTo)
	}
	var ids []string
	own := ig.ownID()
	record := func(msg slidetypes.SentMessage, shown string) {
		id := cmp.Or(msg.ID, msg.MessageID)
		ids = append(ids, id)
		_ = ig.st.PutMsg(ctx, &MsgRef{ID: id, Chat: t.id, Sender: own, Me: true, TS: time.Now().UnixMilli(), Text: shown})
		reply = nil
	}
	sendText := func(text string) error {
		req := &slidetypes.SendTextRequest{
			OfflineThreadingID: strconv.FormatInt(methods.GenerateEpochID(), 10), ReplyToMessageID: reply,
			Text: slidetypes.SensitiveString{Value: text}, SendAttribution: ptr.Ptr("igd_web_chat_tab:in_thread"),
		}
		if t.short != "" {
			req.IGThreadIGID = &t.short
		} else {
			req.RecipientIGIDs = []string{t.id}
		}
		resp, err := cli.SendMessage(ctx, req)
		if err != nil {
			return err
		}
		record(resp.GetMessage(), text)
		return nil
	}
	text := p.Text
	if len(p.Files) > 0 && newDM {
		// media needs an existing thread: say hello with the text first
		if strings.TrimSpace(text) == "" {
			return nil, errors.New("this person has no chat with you yet: send a text first, then the files")
		}
		if err = sendText(text); err != nil {
			return nil, err
		}
		text = ""
		time.Sleep(2 * time.Second)
		if err = ig.ids(ctx, t); err != nil {
			return sentSoFar(ids, fmt.Errorf("the chat was not created yet: %w", err))
		}
	}
	for _, f := range p.Files {
		data, mimeType, name, err := readFile(f)
		if err != nil {
			return sentSoFar(ids, err)
		}
		voice := f.Kind == "voice"
		if voice {
			data, mimeType = toM4A(ctx, data, mimeType)
			name = strings.TrimSuffix(name, ".ogg") + ".m4a"
		}
		up, err := cli.GetHTTP().SendMercuryUploadRequest(ctx, t.key, &httpclient.MercuryUploadMedia{
			Filename: name, MimeType: mimeType, MediaData: data, IsVoiceClip: voice,
		})
		if err != nil {
			return sentSoFar(ids, fmt.Errorf("upload %s: %w", name, err))
		}
		fbid := up.Payload.RealMetadata.GetFbId()
		if fbid == 0 {
			return sentSoFar(ids, fmt.Errorf("upload %s: Instagram returned no id (videos sometimes cannot be sent from the web)", name))
		}
		resp, err := cli.SendMedia(ctx, &slidetypes.SendMediaRequest{
			AttachmentFBID: strconv.FormatInt(fbid, 10), ThreadID: t.short,
			OfflineThreadingID: strconv.FormatInt(methods.GenerateEpochID(), 10), ReplyToMessageID: reply,
		})
		if err != nil {
			return sentSoFar(ids, fmt.Errorf("send %s: %w", name, err))
		}
		kind := kindOf(mimeType)
		if voice {
			kind = "voice"
		}
		record(resp.GetMessage(), labelled(kind, "", name))
	}
	if strings.TrimSpace(text) != "" {
		if err = sendText(text); err != nil {
			return sentSoFar(ids, err)
		}
	}
	_ = ig.st.PutChat(ctx, &Chat{ID: t.id, Kind: kindStr(t.group), FBKey: t.key, LastTS: time.Now().Unix()})
	return map[string]any{"ids": ids}, nil
}

func (ig *instagram) Edit(ctx context.Context, p EditParams) error {
	if strings.TrimSpace(p.Text) == "" {
		return errors.New("new text is empty")
	}
	cli, err := ig.readyClient(ctx)
	if err != nil {
		return err
	}
	t, err := ig.target(ctx, p.Chat)
	if err != nil {
		return err
	}
	if ref, _ := ig.st.GetMsg(ctx, p.ID); ref != nil && !ref.Me {
		return errors.New("only your own messages can be edited")
	}
	if err = ig.ids(ctx, t); err != nil {
		return err
	}
	_, err = cli.EditMessage(ctx, &slidetypes.EditMessageRequest{
		ThreadID: t.short, Body: slidetypes.SensitiveString{Value: p.Text},
		OfflineThreadingID: strconv.FormatInt(methods.GenerateEpochID(), 10), TargetMessageID: p.ID,
	})
	if err == nil {
		ig.st.SetMsgText(ctx, p.ID, p.Text)
	}
	return err
}

func (ig *instagram) React(ctx context.Context, p ReactParams) error {
	cli, err := ig.readyClient(ctx)
	if err != nil {
		return err
	}
	t, err := ig.target(ctx, p.Chat)
	if err != nil {
		return err
	}
	if err = ig.ids(ctx, t); err != nil {
		return err
	}
	in := slidetypes.ReactionInput{Emoji: strings.TrimSpace(p.Emoji), MessageID: p.ID, ReactionStatus: slidetypes.ReactionStatusCreated, ThreadID: t.short}
	if in.Emoji == "" {
		// removing names the emoji that is there (one per person): the one bia sent, else a heart
		in.ReactionStatus, in.Emoji = slidetypes.ReactionStatusDeleted, cmp.Or(ig.st.GetKV(ctx, "react:"+p.ID), "❤️")
	}
	if _, err = cli.SendReaction(ctx, &slidetypes.CreateReactionRequest{Input: in}); err != nil {
		return err
	}
	if in.ReactionStatus == slidetypes.ReactionStatusCreated {
		ig.st.PutKV(ctx, "react:"+p.ID, in.Emoji)
	} else {
		ig.st.DelKV(ctx, "react:"+p.ID)
	}
	return nil
}

func (ig *instagram) Delete(ctx context.Context, p DeleteParams) (map[string]any, error) {
	if !p.Everyone {
		return nil, errors.New("Instagram can only unsend messages for everyone")
	}
	cli, err := ig.readyClient(ctx)
	if err != nil {
		return nil, err
	}
	t, err := ig.target(ctx, p.Chat)
	if err != nil {
		return nil, err
	}
	if err = ig.ids(ctx, t); err != nil {
		return nil, err
	}
	var done, failed []string
	for _, id := range p.IDs {
		if ref, _ := ig.st.GetMsg(ctx, id); ref != nil && !ref.Me {
			failed = append(failed, id+": only your own messages can be unsent")
			continue
		}
		_, err := cli.UnsendMessage(ctx, &slidetypes.UnsendMessageRequest{MessageID: id, SendData: slidetypes.SendData{ThreadID: t.long}})
		if err != nil {
			failed = append(failed, id+": "+err.Error())
		} else {
			done = append(done, id)
			ig.st.DeleteMsg(ctx, id)
		}
	}
	if len(done) == 0 && len(failed) > 0 {
		return nil, errors.New(strings.Join(failed, "; "))
	}
	return map[string]any{"deleted": done, "failed": failed}, nil
}

func (ig *instagram) Typing(ctx context.Context, p TypingParams) error {
	cli, err := ig.client()
	if err != nil {
		return nil
	}
	t, err := ig.target(ctx, p.Chat)
	if err != nil || t.short == "" {
		return nil
	}
	return cli.SetTyping(ctx, t.short, p.On)
}

func (ig *instagram) MarkRead(ctx context.Context, p ReadParams) error {
	cli, err := ig.client()
	if err != nil {
		return notConnected(err)
	}
	t, err := ig.target(ctx, p.Chat)
	if err != nil {
		return err
	}
	if err = ig.ids(ctx, t); err != nil {
		return err
	}
	ts := time.Now()
	if ref, _ := ig.st.GetMsg(ctx, p.ID); ref != nil && ref.TS > 0 {
		ts = time.UnixMilli(ref.TS)
	}
	if _, err = cli.MarkRead(ctx, &slidetypes.MarkReadRequest{
		Metadata: slidetypes.MarkReadMetadata{IGThreadIGID: t.long},
		Data:     slidetypes.MarkReadData{MessageID: p.ID, ItemID: ptr.Ptr("")},
	}); err != nil {
		return err
	}
	_, err = cli.MarkReadValidation(ctx, &slidetypes.MarkReadRequest{
		Metadata: slidetypes.MarkReadMetadata{IGThreadIGID: t.long},
		Data:     slidetypes.MarkReadData{MessageID: p.ID, MessageTimestampMS: jsontime.UnixMilliString{Time: ts}},
	})
	return err
}

// --- chats, people, history ---

func (ig *instagram) chatMap(ctx context.Context, c *Chat) map[string]any {
	title := c.Title
	if c.Kind == "dm" {
		title = cmp.Or(ig.st.UserName(ctx, c.Peer), title)
	}
	out := map[string]any{"chat": c.ID, "title": title, "group": c.Kind == "group", "last_ts": c.LastTS}
	if u, _ := ig.st.GetUser(ctx, c.Peer); c.Kind == "dm" && u != nil && u.Username != "" {
		out["username"] = u.Username
	}
	if c.Request {
		out["request"] = true
	}
	return out
}

func (ig *instagram) Chats(ctx context.Context, p ChatsParams) ([]map[string]any, error) {
	if _, err := ig.client(); errors.Is(err, ErrNotLoggedIn) {
		return nil, err
	}
	if p.Refresh {
		if cli, err := ig.readyClient(ctx); err == nil {
			if resp, err := cli.GetMailbox(ctx); err == nil && resp.GetMailbox() != nil {
				ig.handleMailbox(ctx, resp.GetMailbox())
			}
		}
	}
	chats, err := ig.st.Chats(ctx, p.Limit)
	if err != nil {
		return nil, err
	}
	out := make([]map[string]any, 0, len(chats))
	for _, c := range chats {
		out = append(out, ig.chatMap(ctx, c))
	}
	return out, nil
}

func (ig *instagram) Search(ctx context.Context, p SearchParams) ([]map[string]any, error) {
	q := strings.TrimSpace(strings.TrimPrefix(strings.TrimSpace(p.Query), "@"))
	if q == "" {
		return nil, errors.New("query required")
	}
	limit := p.Limit
	if limit <= 0 {
		limit = 20
	}
	var out []map[string]any
	seen := map[string]bool{}
	add := func(r map[string]any) {
		id := fmt.Sprint(r["chat"])
		if seen[id] || len(out) >= limit {
			return
		}
		seen[id] = true
		out = append(out, r)
	}
	if local, err := ig.st.FindChats(ctx, q, limit); err == nil {
		for _, c := range local {
			add(ig.chatMap(ctx, c))
		}
	}
	cli, err := ig.readyClient(ctx)
	if err != nil {
		if len(out) > 0 {
			return out, nil
		}
		return nil, err
	}
	resp, err := cli.SearchUsers(ctx, q)
	if err != nil {
		if len(out) > 0 {
			return out, nil
		}
		return nil, fmt.Errorf("search: %w", err)
	}
	for _, r := range resp.Data.Results {
		if r == nil || r.InteropMessagingUserFBID == 0 {
			continue
		}
		id := strconv.FormatInt(r.InteropMessagingUserFBID, 10)
		_ = ig.st.PutUser(ctx, &User{ID: id, Name: r.FullName, Username: r.Username, IGID: r.PK})
		add(map[string]any{"chat": id, "title": cmp.Or(r.FullName, r.Username), "username": r.Username, "group": false, "verified": r.IsVerified})
	}
	return out, nil
}

// Resolve turns @username, a profile or chat link, or an id into a chat.
func (ig *instagram) Resolve(ctx context.Context, p ResolveParams) (map[string]any, error) {
	q := strings.TrimSpace(strings.TrimPrefix(strings.TrimSpace(p.Query), "instagram:"))
	if q == "" {
		return nil, errors.New("query required")
	}
	if isDigits(q) {
		if c, _ := ig.st.GetChat(ctx, q); c != nil {
			return ig.chatMap(ctx, c), nil
		}
		return map[string]any{"chat": q, "title": ig.st.UserName(ctx, q), "group": false}, nil
	}
	if strings.Contains(q, "instagram.com") || strings.Contains(q, "ig.me") {
		if !strings.Contains(q, "://") {
			q = "https://" + q
		}
		if u, err := url.Parse(q); err == nil {
			parts := strings.Split(strings.Trim(u.Path, "/"), "/")
			if len(parts) >= 3 && parts[0] == "direct" && parts[1] == "t" {
				c, err := ig.chatForShort(ctx, parts[2])
				if err != nil {
					return nil, err
				}
				return ig.chatMap(ctx, c), nil
			}
			if len(parts) > 0 && parts[0] != "" {
				q = parts[len(parts)-1]
				if parts[0] == "m" && len(parts) > 1 {
					q = parts[1]
				}
			}
		}
	}
	name := strings.ToLower(strings.TrimPrefix(q, "@"))
	if u, _ := ig.st.UserByName(ctx, name); u != nil && strings.EqualFold(u.Username, name) {
		return map[string]any{"chat": u.ID, "title": cmp.Or(u.Name, u.Username), "username": u.Username, "group": false}, nil
	}
	hits, err := ig.Search(ctx, SearchParams{Query: name, Limit: 10})
	if err != nil {
		return nil, err
	}
	for _, h := range hits {
		if strings.EqualFold(fmt.Sprint(h["username"]), name) {
			return h, nil
		}
	}
	if len(hits) == 1 {
		return hits[0], nil
	}
	if len(hits) == 0 {
		return nil, fmt.Errorf("nobody found for %q on Instagram", q)
	}
	var names []string
	for _, h := range hits {
		names = append(names, fmt.Sprintf("%v @%v (%v)", h["title"], h["username"], h["chat"]))
	}
	return nil, fmt.Errorf("several matches for %q: %s", q, strings.Join(names, "; "))
}

func (ig *instagram) Members(ctx context.Context, p ChatParams) (map[string]any, error) {
	t, err := ig.target(ctx, p.Chat)
	if err != nil {
		return nil, err
	}
	if !t.group {
		return map[string]any{"members": []map[string]any{{"id": t.id, "name": ig.st.UserName(ctx, t.id)}}}, nil
	}
	if err = ig.ids(ctx, t); err == nil {
		if cli, err := ig.client(); err == nil {
			if resp, err := cli.GetThread(ctx, slidetypes.MakeGetThreadInfoRequest(t.short)); err == nil && resp.ThreadInfo.AsIGDirectThread != nil {
				ig.putThread(ctx, resp.ThreadInfo.AsIGDirectThread)
			}
		}
	}
	var members []map[string]any
	for uid, role := range ig.st.KVPrefix(ctx, "member:"+t.id+":") {
		m := map[string]any{"id": uid, "name": ig.st.UserName(ctx, uid), "admin": role == "admin"}
		if u, _ := ig.st.GetUser(ctx, uid); u != nil && u.Username != "" {
			m["username"] = u.Username
		}
		members = append(members, m)
	}
	sort.Slice(members, func(i, j int) bool { return fmt.Sprint(members[i]["name"]) < fmt.Sprint(members[j]["name"]) })
	title := ""
	if t.chat != nil {
		title = t.chat.Title
	}
	return map[string]any{"title": title, "members": members}, nil
}

func (ig *instagram) History(ctx context.Context, p HistoryParams) ([]map[string]any, error) {
	cli, err := ig.readyClient(ctx)
	if err != nil {
		return nil, err
	}
	t, err := ig.target(ctx, p.Chat)
	if err != nil {
		return nil, err
	}
	if err = ig.ids(ctx, t); err != nil {
		return nil, err
	}
	limit := p.Limit
	if limit <= 0 || limit > 100 {
		limit = 30
	}
	req := &slidetypes.PaginateMessagesRequest{ThreadID: t.short, FirstN: limit, InitialMessagePageCount: 20}
	if p.Before != "" {
		req.OlderThanMessageID = ptr.Ptr(p.Before)
	}
	resp, err := cli.PaginateMessages(ctx, req)
	if err != nil {
		return nil, err
	}
	own := ig.ownID()
	var out []map[string]any
	if th := resp.ThreadInfo.AsIGDirectThread; th != nil && th.Messages != nil {
		title := ""
		if t.chat != nil {
			title = t.chat.Title
		}
		for _, e := range th.Messages.Edges {
			if ev := ig.message(ctx, t.id, t.group, title, e.Node, own); ev != nil {
				out = append(out, ev)
			}
		}
	}
	sort.Slice(out, func(i, j int) bool { return out[i]["ts"].(int64) > out[j]["ts"].(int64) })
	if len(out) > limit {
		out = out[:limit]
	}
	return out, nil
}

// --- groups and chat settings ---

// people turns @usernames, names, links and ids into Instagram users (FBID, IG id).
func (ig *instagram) people(ctx context.Context, members []string) ([]*User, error) {
	var out []*User
	for _, mem := range members {
		r, err := ig.Resolve(ctx, ResolveParams{Query: mem})
		if err != nil {
			return nil, fmt.Errorf("%s: %w", mem, err)
		}
		if g, _ := r["group"].(bool); g {
			return nil, fmt.Errorf("%s is a group, not a person", mem)
		}
		id := fmt.Sprint(r["chat"])
		u, _ := ig.st.GetUser(ctx, id)
		if u == nil || u.IGID == "" {
			u = &User{ID: id}
			fbid, _ := strconv.ParseInt(id, 10, 64)
			if cli, err := ig.client(); err == nil && fbid != 0 {
				if info, err := cli.GetUserForNewDM(ctx, fbid); err == nil && info.Data != nil {
					ig.putUser(ctx, info.Data)
					u.IGID, u.Name, u.Username = cmp.Or(info.Data.ID, info.Data.PK), info.Data.FullName, info.Data.Username
				}
			}
		}
		out = append(out, u)
	}
	return out, nil
}

func (ig *instagram) CreateGroup(ctx context.Context, p CreateGroupParams) (map[string]any, error) {
	if len(p.Members) < 2 {
		return nil, errors.New("an Instagram group needs at least two other people")
	}
	cli, err := ig.readyClient(ctx)
	if err != nil {
		return nil, err
	}
	users, err := ig.people(ctx, p.Members)
	if err != nil {
		return nil, err
	}
	ids := make([]string, len(users))
	for i, u := range users {
		ids[i] = u.ID
	}
	resp, err := cli.CreateGroup(ctx, &slidetypes.CreateGroupRequest{RecipientFBIDs: ids, OfflineThreadingID: strconv.FormatInt(methods.GenerateEpochID(), 10)})
	if err != nil {
		return nil, err
	}
	key, err := strconv.ParseInt(resp.Data.ID, 10, 64)
	if err != nil {
		return nil, fmt.Errorf("Instagram returned an odd group id %q", resp.Data.ID)
	}
	chat := strconv.FormatInt(key, 10)
	c := &Chat{ID: chat, Kind: "group", FBKey: key, Title: p.Title, LastTS: time.Now().Unix()}
	if resp.Data.Thread != nil {
		c.WAJID = resp.Data.Thread.ID
	}
	_ = ig.st.PutChat(ctx, c)
	if strings.TrimSpace(p.Title) != "" {
		if err = cli.EditGroupTitle(ctx, chat, p.Title); err != nil {
			return map[string]any{"chat": chat, "warning": "created, but naming it failed: " + err.Error()}, nil
		}
	}
	return map[string]any{"chat": chat}, nil
}

func (ig *instagram) AddMembers(ctx context.Context, p MembersParams) (map[string]any, error) {
	cli, err := ig.readyClient(ctx)
	if err != nil {
		return nil, err
	}
	t, err := ig.target(ctx, p.Chat)
	if err != nil {
		return nil, err
	}
	if !t.group {
		return nil, errors.New("not a group")
	}
	users, err := ig.people(ctx, p.Members)
	if err != nil {
		return nil, err
	}
	var igids []string
	for _, u := range users {
		if u.IGID == "" {
			return nil, fmt.Errorf("no Instagram id for %s", cmp.Or(u.Name, u.ID))
		}
		igids = append(igids, u.IGID)
	}
	_, err = cli.AddMembers(ctx, slidetypes.MakeAddMembersRequest(t.id, igids...))
	return map[string]any{}, err
}

func (ig *instagram) RemoveMember(ctx context.Context, p MemberParams) error {
	cli, err := ig.readyClient(ctx)
	if err != nil {
		return err
	}
	t, err := ig.target(ctx, p.Chat)
	if err != nil {
		return err
	}
	if !t.group {
		return errors.New("not a group")
	}
	users, err := ig.people(ctx, []string{p.Member})
	if err != nil {
		return err
	}
	if users[0].IGID == "" {
		return errors.New("no Instagram id for that person")
	}
	_, err = cli.RemoveMember(ctx, &slidetypes.RemoveMemberRequest{ThreadID: t.id, UserIGID: users[0].IGID})
	return err
}

func (ig *instagram) Leave(ctx context.Context, p ChatParams) error {
	cli, err := ig.readyClient(ctx)
	if err != nil {
		return err
	}
	t, err := ig.target(ctx, p.Chat)
	if err != nil {
		return err
	}
	if !t.group {
		return errors.New("not a group")
	}
	_, err = cli.LeaveGroup(ctx, &slidetypes.LeaveThreadRequest{ThreadID: t.id})
	return err
}

func (ig *instagram) Rename(ctx context.Context, p RenameParams) error {
	cli, err := ig.readyClient(ctx)
	if err != nil {
		return err
	}
	t, err := ig.target(ctx, p.Chat)
	if err != nil {
		return err
	}
	if !t.group {
		return errors.New("only groups have names")
	}
	err = cli.EditGroupTitle(ctx, t.id, p.Title)
	if err == nil {
		_ = ig.st.PutChat(ctx, &Chat{ID: t.id, Kind: "group", Title: p.Title})
	}
	return err
}

func (ig *instagram) SetGroupPhoto(ctx context.Context, p PhotoParams) error {
	cli, err := ig.readyClient(ctx)
	if err != nil {
		return err
	}
	t, err := ig.target(ctx, p.Chat)
	if err != nil {
		return err
	}
	if !t.group {
		return errors.New("only groups have a photo")
	}
	data, _, _, err := readFile(FileParam{Path: p.Path})
	if err != nil {
		return err
	}
	return cli.EditGroupAvatar(ctx, t.id, data)
}

func (ig *instagram) Block(ctx context.Context, p BlockParams) error {
	cli, err := ig.readyClient(ctx)
	if err != nil {
		return err
	}
	users, err := ig.people(ctx, []string{p.User})
	if err != nil {
		return err
	}
	if users[0].IGID == "" {
		return errors.New("no Instagram id for that person")
	}
	return cli.SetUserBlocked(ctx, users[0].IGID, p.On)
}

func (ig *instagram) DeleteChat(ctx context.Context, p ChatParams) error {
	cli, err := ig.readyClient(ctx)
	if err != nil {
		return err
	}
	t, err := ig.target(ctx, p.Chat)
	if err != nil {
		return err
	}
	if err = ig.ids(ctx, t); err != nil {
		return err
	}
	_, err = cli.DeleteThread(ctx, &slidetypes.DeleteThreadRequest{ThreadID: t.short})
	if err == nil {
		_ = ig.st.DeleteChat(ctx, t.id)
	}
	return err
}

func (ig *instagram) AcceptRequest(ctx context.Context, p ChatParams) error {
	cli, err := ig.readyClient(ctx)
	if err != nil {
		return err
	}
	t, err := ig.target(ctx, p.Chat)
	if err != nil {
		return err
	}
	if err = ig.ids(ctx, t); err != nil {
		return err
	}
	_, err = cli.AcceptMessageRequest(ctx, &slidetypes.AcceptMessageRequestRequest{
		ThreadID: t.short, OfflineThreadingID: strconv.FormatInt(methods.GenerateEpochID(), 10),
	})
	if err == nil {
		_ = ig.st.PutChat(ctx, &Chat{ID: t.id, Kind: kindStr(t.group), Folder: "inbox"})
	}
	return err
}

func (ig *instagram) Mute(ctx context.Context, p MuteParams) error {
	cli, err := ig.readyClient(ctx)
	if err != nil {
		return err
	}
	t, err := ig.target(ctx, p.Chat)
	if err != nil {
		return err
	}
	if err = ig.ids(ctx, t); err != nil {
		return err
	}
	_, err = cli.MuteThread(ctx, &slidetypes.MuteThreadRequest{
		ThreadID: t.short, MuteSeconds: p.Seconds, OfflineThreadingID: strconv.FormatInt(methods.GenerateEpochID(), 10),
	})
	return err
}

func (ig *instagram) SetProfile(ctx context.Context, p ProfileParams) error {
	return errors.New("bia cannot edit the Instagram profile: change the name, bio or photo at https://www.instagram.com/accounts/edit/ in bia's Chrome (chrome MCP)")
}

func (ig *instagram) Pin(ctx context.Context, p PinParams) error {
	cli, err := ig.readyClient(ctx)
	if err != nil {
		return err
	}
	t, err := ig.target(ctx, p.Chat)
	if err != nil {
		return err
	}
	if err = ig.ids(ctx, t); err != nil {
		return err
	}
	if p.ID == "" {
		_, err = cli.PinThread(ctx, &slidetypes.PinThreadRequest{ThreadID: t.short, Pin: p.On})
		return err
	}
	req := &slidetypes.PinMessageRequest{ThreadID: t.short, MessageID: p.ID}
	if p.On {
		req.ShouldReplaceOldestPin = ptr.Ptr(true)
		_, err = cli.PinMessage(ctx, req)
	} else {
		_, err = cli.UnpinMessage(ctx, req)
	}
	return err
}
