package main

import (
	"context"
	"errors"
	"fmt"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/coder/websocket"
	"github.com/google/uuid"
	"go.mau.fi/util/exhttp"
	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/store"
	"go.mau.fi/whatsmeow/store/sqlstore"
	waTypes "go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	waLog "go.mau.fi/whatsmeow/util/log"

	"go.mau.fi/mautrix-meta/pkg/messagix"
	"go.mau.fi/mautrix-meta/pkg/messagix/cookies"
	"go.mau.fi/mautrix-meta/pkg/messagix/dgw"
	"go.mau.fi/mautrix-meta/pkg/messagix/httpclient"
	"go.mau.fi/mautrix-meta/pkg/messagix/table"
	"go.mau.fi/mautrix-meta/pkg/messagix/types"
)

// Messenger chat ids (the MCP's chat_id, channel messenger:<chat_id>):
//
//	person:          their Facebook id, e.g. 100012345678 (encrypted or not)
//	classic group:   t<thread key>, e.g. t7712345678901234
//	encrypted group: <id>@g.us
type messenger struct {
	st      *Store
	rootCtx context.Context

	container *sqlstore.Container

	mu      sync.Mutex
	sess    *Session
	cli     *messagix.Client
	e2ee    *whatsmeow.Client
	device  *store.Device
	cancel  context.CancelFunc
	state   string
	errText string
	e2eeSt  string
	fbid    int64
	// bumped on every (re)connect so callbacks of an old client do nothing
	gen uint64

	ready     chan struct{}
	e2eeReady chan struct{}

	e2eeConnecting atomic.Bool
	retrying       atomic.Bool

	// history requests waiting for the thread's messages (thread key → channel)
	collectMu  sync.Mutex
	collectors map[int64]chan []*table.WrappedMessage
}

func newMessenger(ctx context.Context, st *Store) *messenger {
	m := &messenger{
		st:         st,
		rootCtx:    ctx,
		state:      "logged_out",
		ready:      make(chan struct{}),
		e2eeReady:  make(chan struct{}),
		collectors: map[int64]chan []*table.WrappedMessage{},
	}
	m.container = sqlstore.NewWithDB(st.db, "sqlite", waLog.Zerolog(log.With().Str("component", "whatsmeow-store").Logger()))
	if err := m.container.Upgrade(ctx); err != nil {
		log.Error().Err(err).Msg("whatsmeow store upgrade failed")
	}
	if sess, _ := st.LoadSession(); sess != nil {
		m.sess = sess
		m.fbid, _ = strconv.ParseInt(sess.ID, 10, 64)
		m.state = "disconnected"
	}
	return m
}

func (m *messenger) setState(state, errText string) {
	m.mu.Lock()
	changed := m.state != state || m.errText != errText
	m.state, m.errText = state, errText
	m.mu.Unlock()
	if changed {
		emit("status", map[string]any{"state": state, "error": errText})
	}
}

func (m *messenger) Status() map[string]any {
	m.mu.Lock()
	defer m.mu.Unlock()
	st := map[string]any{"state": m.state, "e2ee": m.e2eeSt}
	if m.errText != "" {
		st["error"] = m.errText
	}
	if m.sess != nil {
		st["id"], st["name"], st["username"], st["variant"] = m.sess.ID, m.sess.Name, m.sess.Username, m.sess.Variant
	}
	return st
}

func platformOf(variant string) types.Platform {
	if variant == "messenger" {
		return types.Messenger
	}
	return types.Facebook
}

func newCookies(variant string, values map[string]string) *cookies.Cookies {
	c := &cookies.Cookies{Platform: platformOf(variant)}
	vals := make(map[cookies.MetaCookieName]string, len(values))
	for k, v := range values {
		vals[cookies.MetaCookieName(k)] = v
	}
	c.UpdateValues(vals)
	return c
}

func (m *messenger) newClient(c *cookies.Cookies) *messagix.Client {
	return messagix.NewClient(c, log.With().Str("component", "messagix").Logger(), &messagix.Config{ClientSettings: exhttp.SensibleClientSettings})
}

// loginError turns Meta's refusals into something a person can act on.
func loginError(err error) error {
	switch {
	case errors.Is(err, httpclient.ErrTokenInvalidated), errors.Is(err, httpclient.ErrUserIDIsZero):
		return fmt.Errorf("Facebook did not accept the session (logged out or the cookies are old): log in again in the browser, then call login again")
	case errors.Is(err, httpclient.ErrCheckpointRequired):
		return fmt.Errorf("Facebook wants a security check first (checkpoint): open https://www.facebook.com in the browser, finish it, then call login again (%s)", httpclient.GetErrorRedirectURL(err))
	case errors.Is(err, httpclient.ErrChallengeRequired):
		return fmt.Errorf("Facebook wants a challenge solved first: open https://www.facebook.com in the browser, finish it, then call login again (%s)", httpclient.GetErrorRedirectURL(err))
	case errors.Is(err, httpclient.ErrConsentRequired):
		return fmt.Errorf("Facebook asks for consent first: open https://www.facebook.com in the browser, accept, then call login again")
	case errors.Is(err, httpclient.ErrAccountSuspended):
		return fmt.Errorf("this Facebook account is suspended: %s", httpclient.GetErrorRedirectURL(err))
	}
	return err
}

func isLoggedOut(err error) bool {
	return errors.Is(err, httpclient.ErrTokenInvalidated) || errors.Is(err, httpclient.ErrUserIDIsZero)
}

func (m *messenger) Login(ctx context.Context, p LoginParams) (map[string]any, error) {
	c := newCookies(p.Variant, p.Cookies)
	if missing := c.GetMissingCookieNames(); len(missing) > 0 {
		return nil, fmt.Errorf("missing cookies %v: log in to Facebook in the browser first", missing)
	}
	cli := m.newClient(c)
	user, tbl, err := cli.LoadMessagesPage(ctx)
	if err != nil {
		return nil, loginError(err)
	}
	if user.GetFBID() != c.GetUserID() {
		return nil, fmt.Errorf("the account (%d) does not match the cookies (%d)", user.GetFBID(), c.GetUserID())
	}
	m.stop()
	old := m.sess
	sess := &Session{
		Platform: "messenger", Variant: orStr(p.Variant, "facebook"), Cookies: p.Cookies,
		ID: strconv.FormatInt(user.GetFBID(), 10), Name: user.GetName(), Username: user.GetUsername(), Since: time.Now().Unix(),
	}
	if old != nil && old.ID != sess.ID {
		// another account: forget the old one's chats and encryption device
		_ = m.st.Wipe(ctx)
		m.deleteDevices(ctx)
	}
	if err = m.st.SaveSession(sess); err != nil {
		return nil, err
	}
	m.mu.Lock()
	m.sess, m.fbid = sess, user.GetFBID()
	m.mu.Unlock()
	_ = m.st.PutUser(ctx, &User{ID: sess.ID, Name: sess.Name, Username: sess.Username})
	m.start(cli, user, tbl)
	return map[string]any{"id": sess.ID, "name": sess.Name, "username": sess.Username}, nil
}

func (m *messenger) Restore(ctx context.Context) (map[string]any, error) {
	m.mu.Lock()
	sess, cli, state := m.sess, m.cli, m.state
	m.mu.Unlock()
	if sess == nil {
		return m.Status(), nil
	}
	if cli == nil || state == "error" || state == "disconnected" {
		m.connectSaved(ctx)
	}
	// give it a moment so the answer says where it is
	select {
	case <-m.readyCh():
	case <-time.After(20 * time.Second):
	case <-ctx.Done():
	}
	return m.Status(), nil
}

func (m *messenger) readyCh() chan struct{} {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.ready
}

func (m *messenger) e2eeCh() chan struct{} {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.e2eeReady
}

// connectSaved loads the messages page with the saved cookies and connects.
func (m *messenger) connectSaved(ctx context.Context) {
	m.mu.Lock()
	sess := m.sess
	m.mu.Unlock()
	if sess == nil {
		return
	}
	m.setState("connecting", "")
	cli := m.newClient(newCookies(sess.Variant, sess.Cookies))
	user, tbl, err := cli.LoadMessagesPage(ctx)
	if err != nil {
		log.Warn().Err(err).Msg("load messages page")
		if isLoggedOut(err) {
			m.loggedOut("Facebook ended the session (logged out elsewhere or the cookies expired)")
			return
		}
		m.setState("error", loginError(err).Error())
		m.retryLater()
		return
	}
	m.start(cli, user, tbl)
}

// retryLater keeps trying to connect with backoff while the session is still valid.
func (m *messenger) retryLater() {
	if !m.retrying.CompareAndSwap(false, true) {
		return
	}
	go func() {
		defer m.retrying.Store(false)
		delay := 15 * time.Second
		for i := 0; i < 40; i++ {
			select {
			case <-time.After(delay):
			case <-m.rootCtx.Done():
				return
			}
			m.mu.Lock()
			sess, state := m.sess, m.state
			m.mu.Unlock()
			if sess == nil || state == "ready" || state == "logged_out" {
				return
			}
			ctx, cancel := context.WithTimeout(m.rootCtx, 2*time.Minute)
			m.connectSaved(ctx)
			cancel()
			if m.Status()["state"] == "ready" {
				return
			}
			delay = min(delay*2, 10*time.Minute)
		}
	}()
}

func (m *messenger) start(cli *messagix.Client, user types.UserInfo, tbl *table.LSTable) {
	m.mu.Lock()
	if m.cancel != nil {
		m.cancel()
	}
	if m.cli != nil && m.cli != cli {
		old := m.cli
		go old.Disconnect()
	}
	if m.e2ee != nil {
		old := m.e2ee
		go old.Disconnect()
		m.e2ee = nil
	}
	m.gen++
	gen := m.gen
	ctx, cancel := context.WithCancel(m.rootCtx)
	m.cancel = cancel
	m.cli = cli
	m.fbid = user.GetFBID()
	m.ready = make(chan struct{})
	m.e2eeReady = make(chan struct{})
	m.e2eeSt = "connecting"
	m.mu.Unlock()
	m.setState("connecting", "")
	cli.SetEventHandler(func(_ context.Context, evt any) { m.onEvent(gen, evt) })
	if tbl != nil {
		m.handleTable(ctx, tbl, true)
	}
	go func() {
		if err := cli.Connect(ctx); err != nil {
			log.Warn().Err(err).Msg("socket connect")
			if m.current(gen) {
				m.setState("error", err.Error())
				m.retryLater()
			}
		}
	}()
	go m.saveCookiesLoop(ctx, gen)
}

func (m *messenger) current(gen uint64) bool {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.gen == gen
}

// saveCookiesLoop keeps session.json up to date: Meta rotates some cookies.
func (m *messenger) saveCookiesLoop(ctx context.Context, gen uint64) {
	t := time.NewTicker(10 * time.Minute)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-t.C:
			m.saveCookies(gen)
		}
	}
}

func (m *messenger) saveCookies(gen uint64) {
	m.mu.Lock()
	if m.gen != gen || m.cli == nil || m.sess == nil {
		m.mu.Unlock()
		return
	}
	all := m.cli.GetCookies().GetAll()
	sess := *m.sess
	m.mu.Unlock()
	if len(all) == 0 {
		return
	}
	vals := make(map[string]string, len(all))
	for k, v := range all {
		vals[string(k)] = v
	}
	if vals["xs"] == "" {
		return
	}
	sess.Cookies = vals
	if err := m.st.SaveSession(&sess); err == nil {
		m.mu.Lock()
		if m.gen == gen {
			m.sess = &sess
		}
		m.mu.Unlock()
	}
}

func (m *messenger) onEvent(gen uint64, raw any) {
	if !m.current(gen) {
		return
	}
	ctx := m.rootCtx
	switch evt := raw.(type) {
	case *table.LSTable:
		m.handleTable(ctx, evt, false)
	case *messagix.ConnectedEvent, *messagix.ReconnectedEvent:
		m.setState("ready", "")
		m.mu.Lock()
		select {
		case <-m.ready:
		default:
			close(m.ready)
		}
		m.mu.Unlock()
		m.saveCookies(gen)
		go m.connectE2EE(gen, false)
	case *messagix.TransientDisconnectEvent:
		m.setState("reconnecting", "")
	case *messagix.PermanentErrorEvent:
		log.Warn().Err(evt.Err).Msg("permanent socket error")
		if websocket.CloseStatus(evt.Err) == dgw.CloseStatusUnauthorized {
			m.loggedOut("Facebook ended the session")
			return
		}
		m.setState("error", evt.Err.Error())
		m.retryLater()
	}
}

func (m *messenger) SessionGone(reason string) {
	if m.Status()["state"] != "logged_out" {
		m.loggedOut(reason)
	}
}

func (m *messenger) loggedOut(reason string) {
	m.stop()
	m.mu.Lock()
	m.sess = nil
	m.mu.Unlock()
	_ = m.st.DeleteSession()
	m.setState("logged_out", reason)
	emit("logged_out", map[string]any{"reason": reason})
}

// stop disconnects without forgetting the session.
func (m *messenger) stop() {
	m.mu.Lock()
	m.gen++
	cli, e2ee, cancel := m.cli, m.e2ee, m.cancel
	m.cli, m.e2ee, m.cancel = nil, nil, nil
	m.e2eeSt = ""
	m.mu.Unlock()
	if cancel != nil {
		cancel()
	}
	if e2ee != nil {
		e2ee.Disconnect()
	}
	if cli != nil {
		cli.SetEventHandler(nil)
		cli.Disconnect()
	}
}

func (m *messenger) Close() {
	m.stop()
}

func (m *messenger) Logout(ctx context.Context) error {
	m.stop()
	m.deleteDevices(ctx)
	m.mu.Lock()
	m.sess, m.fbid = nil, 0
	m.mu.Unlock()
	if err := m.st.Wipe(ctx); err != nil {
		return err
	}
	m.setState("logged_out", "")
	return nil
}

func (m *messenger) deleteDevices(ctx context.Context) {
	devs, err := m.container.GetAllDevices(ctx)
	if err != nil {
		return
	}
	for _, d := range devs {
		_ = d.Delete(ctx)
	}
	m.mu.Lock()
	m.device = nil
	m.mu.Unlock()
}

// --- end-to-end encryption (Messenger's encrypted chats run over WhatsApp's protocol) ---

func (m *messenger) connectE2EE(gen uint64, retry bool) {
	if !m.e2eeConnecting.CompareAndSwap(false, true) {
		return
	}
	defer m.e2eeConnecting.Store(false)
	m.mu.Lock()
	if m.gen != gen || m.cli == nil || m.e2ee != nil {
		m.mu.Unlock()
		return
	}
	cli, fbid := m.cli, m.fbid
	m.mu.Unlock()
	ctx := m.rootCtx
	dev, err := m.loadDevice(ctx, fbid)
	if err != nil {
		m.e2eeFailed(gen, fmt.Errorf("encryption keys: %w", err))
		return
	}
	isNew := dev == nil
	if isNew {
		dev = m.container.NewDevice()
		if suggested := cli.MessengerLite.GetSuggestedDeviceID(); suggested != uuid.Nil {
			dev.FacebookUUID = suggested
		}
	}
	cli.SetDevice(dev)
	if isNew {
		if err = cli.RegisterE2EE(ctx, fbid); err != nil {
			m.e2eeFailed(gen, fmt.Errorf("register encrypted device: %w", err))
			return
		}
		if err = dev.Save(ctx); err != nil {
			m.e2eeFailed(gen, fmt.Errorf("save encrypted device: %w", err))
			return
		}
	}
	e2ee, err := cli.PrepareE2EEClient()
	if err != nil {
		m.e2eeFailed(gen, fmt.Errorf("prepare encrypted client: %w", err))
		return
	}
	e2ee.SynchronousAck = true
	e2ee.AddEventHandler(func(evt any) { m.onE2EE(gen, evt) })
	m.mu.Lock()
	if m.gen != gen {
		m.mu.Unlock()
		return
	}
	m.e2ee, m.device = e2ee, dev
	m.mu.Unlock()
	if err = e2ee.Connect(); err != nil {
		m.mu.Lock()
		if m.e2ee == e2ee {
			m.e2ee = nil
		}
		m.mu.Unlock()
		m.e2eeFailed(gen, fmt.Errorf("connect encrypted chats: %w", err))
	}
}

func (m *messenger) loadDevice(ctx context.Context, fbid int64) (*store.Device, error) {
	m.mu.Lock()
	if m.device != nil && m.device.ID != nil && m.device.ID.User == strconv.FormatInt(fbid, 10) {
		d := m.device
		m.mu.Unlock()
		return d, nil
	}
	m.mu.Unlock()
	devs, err := m.container.GetAllDevices(ctx)
	if err != nil {
		return nil, err
	}
	var found *store.Device
	for _, d := range devs {
		if d.ID != nil && d.ID.User == strconv.FormatInt(fbid, 10) && found == nil {
			found = d
		} else {
			_ = d.Delete(ctx)
		}
	}
	return found, nil
}

func (m *messenger) e2eeFailed(gen uint64, err error) {
	log.Warn().Err(err).Msg("encrypted chats")
	m.mu.Lock()
	if m.gen == gen {
		m.e2eeSt = "error: " + err.Error()
	}
	m.mu.Unlock()
	go func() {
		select {
		case <-time.After(2 * time.Minute):
			if m.current(gen) {
				m.connectE2EE(gen, true)
			}
		case <-m.rootCtx.Done():
		}
	}()
}

func (m *messenger) onE2EE(gen uint64, raw any) {
	if !m.current(gen) {
		return
	}
	switch evt := raw.(type) {
	case *events.FBMessage:
		m.handleFBMessage(m.rootCtx, evt)
	case *events.Connected:
		m.mu.Lock()
		m.e2eeSt = "ready"
		select {
		case <-m.e2eeReady:
		default:
			close(m.e2eeReady)
		}
		m.mu.Unlock()
		emit("status", map[string]any{"state": m.Status()["state"], "e2ee": "ready"})
	case *events.Disconnected:
		m.mu.Lock()
		m.e2eeSt = "reconnecting"
		m.mu.Unlock()
	case *events.CATRefreshError:
		log.Warn().Err(evt.Error).Msg("encrypted chats: token refresh failed, reconnecting")
		go m.fullReconnect(gen)
	case *events.LoggedOut:
		// as mautrix-meta: a 401 outside connecting means the device was removed
		log.Warn().Bool("on_connect", evt.OnConnect).Str("reason", evt.Reason.String()).Msg("encrypted device logged out")
		if evt.Reason == events.ConnectFailureLoggedOut && !evt.OnConnect {
			m.resetE2EE(gen, true)
		} else {
			m.e2eeError(gen, "logged out: "+evt.Reason.String())
		}
	case *events.ConnectFailure:
		log.Warn().Str("reason", evt.Reason.String()).Msg("encrypted chats connect failure")
		switch evt.Reason {
		case events.ConnectFailureNotFound, events.ConnectFailureClientUnknown:
			// 415/418: the device is unknown on Meta's side, register a new one
			m.resetE2EE(gen, true)
		case events.ConnectFailureGeneric:
			// 400 after outages: the token is probably stale, a full reconnect gets a new one
			go m.fullReconnect(gen)
		default:
			m.e2eeError(gen, evt.Reason.String())
		}
	case *events.GroupInfo:
		if evt.Name != nil {
			_ = m.st.PutChat(m.rootCtx, &Chat{ID: waChatKey(evt.JID), Kind: "group", E2EE: true, WAJID: evt.JID.String(), Title: evt.Name.Name})
			emit("chat", map[string]any{"chat": waChatKey(evt.JID), "title": evt.Name.Name, "group": true})
		}
	}
}

func (m *messenger) e2eeError(gen uint64, why string) {
	m.mu.Lock()
	if m.gen == gen {
		m.e2eeSt = "error: " + why
	}
	m.mu.Unlock()
}

// resetE2EE drops the encrypted device (optionally forgetting its keys) and starts over.
func (m *messenger) resetE2EE(gen uint64, forget bool) {
	m.mu.Lock()
	if m.gen != gen {
		m.mu.Unlock()
		return
	}
	e2ee, dev := m.e2ee, m.device
	m.e2ee = nil
	if forget {
		m.device = nil
	}
	m.e2eeSt = "reconnecting"
	m.mu.Unlock()
	if e2ee != nil {
		e2ee.Disconnect()
	}
	if forget && dev != nil {
		_ = dev.Delete(m.rootCtx)
	}
	go func() {
		time.Sleep(5 * time.Second)
		m.connectE2EE(gen, true)
	}()
}

func (m *messenger) fullReconnect(gen uint64) {
	if !m.current(gen) {
		return
	}
	ctx, cancel := context.WithTimeout(m.rootCtx, 2*time.Minute)
	defer cancel()
	m.connectSaved(ctx)
}

// --- helpers shared by the incoming and outgoing halves ---

func (m *messenger) client() (*messagix.Client, error) {
	m.mu.Lock()
	cli, sess := m.cli, m.sess
	m.mu.Unlock()
	if sess == nil {
		return nil, ErrNotLoggedIn
	}
	if cli == nil {
		return nil, errors.New("Messenger is not connected (it reconnects by itself): try again in a minute. If messenger_status keeps showing it down, open https://www.facebook.com/messages/ in bia's Chrome (a security check may wait there), then call messenger_login again")
	}
	return cli, nil
}

// readyClient waits (up to a minute) until the socket can send.
func (m *messenger) readyClient(ctx context.Context) (*messagix.Client, error) {
	if _, err := m.client(); err != nil {
		return nil, err
	}
	select {
	case <-m.readyCh():
	case <-time.After(time.Minute):
		return nil, errors.New("Messenger is still connecting: try again in a minute. If messenger_status keeps showing it down, open https://www.facebook.com/messages/ in bia's Chrome, then call messenger_login again")
	case <-ctx.Done():
		return nil, ctx.Err()
	}
	return m.client()
}

// encrypted waits (up to a minute) for the encrypted-chat connection.
func (m *messenger) encrypted(ctx context.Context) (*whatsmeow.Client, error) {
	if _, err := m.client(); err != nil {
		return nil, err
	}
	select {
	case <-m.e2eeCh():
	case <-time.After(time.Minute):
		m.mu.Lock()
		st := m.e2eeSt
		m.mu.Unlock()
		return nil, fmt.Errorf("encrypted chats are not connected yet (%s; messenger_status shows it); try again in a minute", orStr(st, "connecting"))
	case <-ctx.Done():
		return nil, ctx.Err()
	}
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.e2ee == nil {
		return nil, errors.New("encrypted chats are reconnecting; try again in a minute")
	}
	return m.e2ee, nil
}

func (m *messenger) ownID() string {
	m.mu.Lock()
	defer m.mu.Unlock()
	return strconv.FormatInt(m.fbid, 10)
}

// waChatKey is the chat id of an encrypted chat.
func waChatKey(jid waTypes.JID) string {
	if jid.Server == waTypes.GroupServer {
		return jid.User + "@g.us"
	}
	return jid.User
}

func isDigits(s string) bool {
	if s == "" {
		return false
	}
	for _, r := range s {
		if r < '0' || r > '9' {
			return false
		}
	}
	return true
}

// target is a resolved chat: where and how to send.
type target struct {
	key   string
	chat  *Chat
	e2ee  bool
	jid   waTypes.JID
	fbKey int64
	group bool
}

func normChat(id string) string {
	id = strings.TrimSpace(id)
	id = strings.TrimPrefix(id, "messenger:")
	return id
}

func (m *messenger) target(ctx context.Context, id string) (*target, error) {
	id = normChat(id)
	if id == "" {
		return nil, errors.New("chat required")
	}
	c, err := m.st.GetChat(ctx, id)
	if err != nil {
		return nil, err
	}
	t := &target{key: id, chat: c}
	switch {
	case strings.HasSuffix(id, "@g.us"):
		user := strings.TrimSuffix(id, "@g.us")
		t.e2ee, t.group = true, true
		t.jid = waTypes.NewJID(user, waTypes.GroupServer)
		if c != nil {
			t.fbKey = c.FBKey
		}
	case strings.HasPrefix(id, "t") && isDigits(id[1:]):
		t.group = true
		t.fbKey, _ = strconv.ParseInt(id[1:], 10, 64)
	case isDigits(id):
		t.jid = waTypes.NewJID(id, waTypes.MessengerServer)
		t.fbKey, _ = strconv.ParseInt(id, 10, 64)
		// new chats are encrypted on Messenger; a known classic one stays classic
		t.e2ee = c == nil || c.E2EE
		if c != nil && c.FBKey != 0 {
			t.fbKey = c.FBKey
		}
	default:
		hits, _ := m.st.FindChats(ctx, id, 2)
		if len(hits) == 1 {
			return m.target(ctx, hits[0].ID)
		}
		return nil, fmt.Errorf("unknown Messenger chat %q: use an id from chats or search", id)
	}
	return t, nil
}
