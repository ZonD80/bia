package main

import (
	"context"
	"fmt"
	"net/url"
	"strconv"
	"strings"
	"sync"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waArmadilloApplication"
	"go.mau.fi/whatsmeow/proto/waArmadilloXMA"
	"go.mau.fi/whatsmeow/proto/waCommon"
	"go.mau.fi/whatsmeow/proto/waConsumerApplication"
	"go.mau.fi/whatsmeow/proto/waMediaTransport"
	waTypes "go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"

	"go.mau.fi/mautrix-meta/pkg/messagix/socket"
	"go.mau.fi/mautrix-meta/pkg/messagix/table"
)

// The thumbs-up "stickers" Messenger sends for the like button (three sizes).
const (
	thumbsUpSmall  = 369239263222822
	thumbsUpMedium = 369239343222814
	thumbsUpLarge  = 369239383222810
)

// tableThreads is what one table says about its threads.
type tableThreads struct {
	types map[int64]table.ThreadType
	// Facebook thread key → WhatsApp JID user of its encrypted twin
	wa map[int64]int64
}

func isGroupType(tt table.ThreadType) bool {
	return tt != table.UNKNOWN_THREAD_TYPE && !tt.IsOneToOne() && tt != table.FOLDER
}

// chatOf maps a Facebook thread key to the chat id (and whether it is a group).
func (m *messenger) chatOf(ctx context.Context, tt *tableThreads, key int64) (string, bool, bool) {
	typ := tt.types[key]
	if waUser, ok := tt.wa[key]; ok {
		if typ == table.ENCRYPTED_OVER_WA_GROUP || isGroupType(typ) {
			return strconv.FormatInt(waUser, 10) + "@g.us", true, true
		}
		return strconv.FormatInt(waUser, 10), false, true
	}
	if c, _ := m.st.ChatByFBKey(ctx, key); c != nil {
		return c.ID, c.Kind == "group", c.E2EE
	}
	if typ == table.ENCRYPTED_OVER_WA_GROUP {
		return strconv.FormatInt(key, 10) + "@g.us", true, true
	}
	if isGroupType(typ) {
		return "t" + strconv.FormatInt(key, 10), true, false
	}
	return strconv.FormatInt(key, 10), false, typ.IsWhatsApp()
}

func (m *messenger) handleTable(ctx context.Context, tbl *table.LSTable, initial bool) {
	own := m.ownID()
	for _, c := range tbl.LSDeleteThenInsertContact {
		_ = m.st.PutUser(ctx, &User{ID: strconv.FormatInt(c.GetFBID(), 10), Name: c.GetName(), Username: c.GetUsername()})
	}
	for _, c := range tbl.LSVerifyContactRowExists {
		_ = m.st.PutUser(ctx, &User{ID: strconv.FormatInt(c.GetFBID(), 10), Name: c.GetName(), Username: c.GetUsername()})
	}
	tt := &tableThreads{types: map[int64]table.ThreadType{}, wa: map[int64]int64{}}
	for _, v := range tbl.LSVerifyThreadExists {
		tt.types[v.ThreadKey] = v.ThreadType
	}
	for _, t := range tbl.LSDeleteThenInsertThread {
		tt.types[t.ThreadKey] = t.ThreadType
	}
	for _, t := range tbl.LSUpdateOrInsertThread {
		tt.types[t.ThreadKey] = t.ThreadType
	}
	for _, j := range tbl.LSUpdateThreadAuthorityAndMappingWithOTIDFromJID {
		tt.wa[j.ThreadKey] = j.ThreadJID
	}
	for _, j := range tbl.LSVerifyHybridThreadExists {
		tt.wa[j.ThreadKey] = j.ThreadJID
		if _, ok := tt.types[j.ThreadKey]; !ok || j.ThreadType != table.UNKNOWN_THREAD_TYPE {
			tt.types[j.ThreadKey] = j.ThreadType
		}
		key, group, _ := m.chatOf(ctx, tt, j.ThreadKey)
		c := &Chat{ID: key, Kind: kindStr(group), E2EE: true, FBKey: j.ThreadKey, LastTS: j.LastActivityTimestampMS / 1000}
		if group {
			c.WAJID = waTypes.NewJID(strconv.FormatInt(j.ThreadJID, 10), waTypes.GroupServer).String()
		} else {
			c.Peer = strconv.FormatInt(j.ThreadJID, 10)
			c.WAJID = waTypes.NewJID(c.Peer, waTypes.MessengerServer).String()
		}
		_ = m.st.PutChat(ctx, c)
	}
	putThread := func(key int64, name string, typ table.ThreadType, last int64, folder string) {
		id, group, e2ee := m.chatOf(ctx, tt, key)
		c := &Chat{ID: id, Kind: kindStr(group), E2EE: e2ee, FBKey: key, LastTS: last / 1000, Folder: folder,
			Request: folder == "pending" || folder == "other"}
		if group {
			c.Title = name
		} else {
			c.Peer = strings.TrimSuffix(id, "@g.us")
		}
		_ = m.st.PutChat(ctx, c)
		title := name
		if !group {
			title = m.st.UserName(ctx, c.Peer)
		}
		emit("chat", map[string]any{"chat": id, "title": title, "group": group, "e2ee": e2ee, "request": c.Request})
	}
	for _, t := range tbl.LSDeleteThenInsertThread {
		putThread(t.ThreadKey, t.ThreadName, t.ThreadType, t.LastActivityTimestampMs, t.FolderName)
	}
	for _, t := range tbl.LSUpdateOrInsertThread {
		putThread(t.ThreadKey, t.ThreadName, t.ThreadType, t.LastActivityTimestampMs, t.FolderName)
	}
	for _, t := range tbl.LSSyncUpdateThreadName {
		id, group, _ := m.chatOf(ctx, tt, t.ThreadKey)
		if group {
			_ = m.st.PutChat(ctx, &Chat{ID: id, Kind: "group", Title: t.ThreadName, FBKey: t.ThreadKey})
			emit("chat", map[string]any{"chat": id, "title": t.ThreadName, "group": true})
		}
	}
	for _, mv := range tbl.LSMoveThreadToE2EECutoverFolder {
		// Meta moved a classic chat to end-to-end encryption: send that way from now on
		id, group, _ := m.chatOf(ctx, tt, mv.ThreadKey)
		_ = m.st.PutChat(ctx, &Chat{ID: id, Kind: kindStr(group), E2EE: true, FBKey: mv.ThreadKey})
	}
	for _, p := range tbl.LSAddParticipantIdToGroupThread {
		uid := strconv.FormatInt(p.ContactId, 10)
		if p.Nickname != "" {
			_ = m.st.PutUser(ctx, &User{ID: uid, Name: p.Nickname})
		}
		id, group, _ := m.chatOf(ctx, tt, p.ThreadKey)
		if group {
			m.st.PutKV(ctx, "member:"+id+":"+uid, b2s(p.IsAdmin))
		}
	}
	for _, p := range tbl.LSRemoveParticipantFromThread {
		id, group, _ := m.chatOf(ctx, tt, p.ThreadKey)
		if group {
			m.st.DelKV(ctx, "member:"+id+":"+strconv.FormatInt(p.ParticipantId, 10))
		}
	}

	upsert, insert := tbl.WrapMessages()
	for key, um := range upsert {
		if m.collect(key, um.Messages) {
			continue
		}
		// pages of older messages (the initial inbox, or what the server sends on its own)
		var batch []map[string]any
		for _, wm := range um.Messages {
			if ev := m.classicMessage(ctx, tt, wm, own); ev != nil {
				batch = append(batch, ev)
			}
		}
		if len(batch) > 0 {
			emit("backlog", map[string]any{"messages": batch, "initial": initial})
		}
	}
	for _, wm := range insert {
		if ev := m.classicMessage(ctx, tt, wm, own); ev != nil {
			emit("message", ev)
		}
	}
	for _, e := range tbl.LSEditMessage {
		ref, _ := m.st.GetMsg(ctx, e.MessageID)
		if ref == nil {
			continue
		}
		m.st.SetMsgText(ctx, e.MessageID, e.Text)
		emit("edit", map[string]any{"chat": ref.Chat, "id": e.MessageID, "sender": ref.Sender, "sender_name": m.st.UserName(ctx, ref.Sender),
			"me": ref.Me, "text": e.Text, "old": ref.Text, "sent_ts": ref.TS / 1000, "ts": nowMS() / 1000})
	}
	unsend := func(threadKey int64, id string) {
		chat, _, _ := m.chatOf(ctx, tt, threadKey)
		ref, _ := m.st.GetMsg(ctx, id)
		ev := map[string]any{"chat": chat, "id": id}
		if ref != nil {
			ev["sender"], ev["me"] = ref.Sender, ref.Me
		}
		m.st.DeleteMsg(ctx, id)
		emit("delete", ev)
	}
	for _, d := range tbl.LSDeleteMessage {
		unsend(d.ThreadKey, d.MessageId)
	}
	for _, d := range tbl.LSDeleteThenInsertMessage {
		if d.IsUnsent {
			unsend(d.ThreadKey, d.MessageId)
		}
	}
	react := func(threadKey int64, msgID string, actor int64, emoji string) {
		chat, _, _ := m.chatOf(ctx, tt, threadKey)
		sender := strconv.FormatInt(actor, 10)
		emit("reaction", map[string]any{"chat": chat, "id": msgID, "sender": sender, "sender_name": m.st.UserName(ctx, sender),
			"me": sender == own, "emoji": emoji})
	}
	if !initial {
		for _, r := range tbl.LSUpsertReaction {
			react(r.ThreadKey, r.MessageId, r.ActorId, r.Reaction)
		}
		for _, r := range tbl.LSDeleteReaction {
			react(r.ThreadKey, r.MessageId, r.ActorId, "")
		}
	}
	for _, d := range tbl.LSDeleteThread {
		chat, _, _ := m.chatOf(ctx, tt, d.ThreadKey)
		if _, active := tt.types[d.ThreadKey]; !active {
			_ = m.st.DeleteChat(ctx, chat)
			emit("chat_deleted", map[string]any{"chat": chat})
		}
	}
	if !initial {
		if cli, err := m.client(); err == nil {
			cli.PostHandlePublishResponse(tbl)
		}
	}
	m.st.Prune(ctx)
}

func kindStr(group bool) string {
	if group {
		return "group"
	}
	return "dm"
}

func b2s(b bool) string {
	if b {
		return "admin"
	}
	return "member"
}

// collect hands a page of history to a waiting history request.
func (m *messenger) collect(threadKey int64, msgs []*table.WrappedMessage) bool {
	m.collectMu.Lock()
	ch := m.collectors[threadKey]
	if ch != nil {
		delete(m.collectors, threadKey)
	}
	m.collectMu.Unlock()
	if ch == nil {
		return false
	}
	ch <- msgs
	return true
}

func removeLPHP(addr string) string {
	parsed, _ := url.Parse(addr)
	if parsed != nil && parsed.Path == "/l.php" {
		if u := parsed.Query().Get("u"); u != "" {
			return u
		}
	}
	return addr
}

// classicMessage turns a (non-encrypted) message into an event map; nil skips it.
func (m *messenger) classicMessage(ctx context.Context, tt *tableThreads, wm *table.WrappedMessage, own string) map[string]any {
	if wm == nil || wm.LSInsertMessage == nil || wm.IsUnsent {
		return nil
	}
	chat, group, e2ee := m.chatOf(ctx, tt, wm.ThreadKey)
	sender := strconv.FormatInt(wm.SenderId, 10)
	text := wm.Text
	switch wm.StickerId {
	case thumbsUpSmall, thumbsUpMedium, thumbsUpLarge:
		if len(wm.Stickers) <= 1 {
			text, wm.Stickers = "👍", nil
		}
	}
	var atts []Attachment
	seen := map[string]bool{}
	for _, b := range wm.BlobAttachments {
		if b.AttachmentFbid != "" && seen[b.AttachmentFbid] {
			continue
		}
		seen[b.AttachmentFbid] = true
		atts = append(atts, blobAttachment(b.AttachmentType, b.PlayableUrl, b.PlayableUrlMimeType, b.AttachmentMimeType,
			b.PreviewUrl, b.PreviewUrlMimeType, b.Filename, b.Filesize, b.PlayableDurationMs))
	}
	for _, a := range wm.Attachments {
		if a.EphemeralMediaViewMode == table.EphemeralMediaViewOnce || a.EphemeralMediaViewMode == table.EphemeralMediaReplayable {
			atts = append(atts, Attachment{Kind: "image", Note: "view-once media: only viewable in the Messenger app"})
			continue
		}
		atts = append(atts, blobAttachment(a.AttachmentType, a.PlayableUrl, a.PlayableUrlMimeType, a.AttachmentMimeType,
			a.PreviewUrl, a.PreviewUrlMimeType, a.Filename, a.Filesize, a.PlayableDurationMs))
	}
	for _, s := range wm.Stickers {
		u := orStr(s.PlayableUrl, orStr(s.PreviewUrl, s.ImageUrl))
		mt := orStr(s.PlayableUrlMimeType, orStr(s.PreviewUrlMimeType, s.ImageUrlMimeType))
		a := Attachment{Kind: "sticker", Mime: mt, Name: s.AccessibilitySummaryText}
		if u != "" {
			a.Ref = urlRef(u, mt)
		}
		atts = append(atts, a)
	}
	var extra []string
	for _, x := range wm.XMAAttachments {
		extra = append(extra, xmaText(x))
	}
	if len(extra) > 0 {
		t := strings.TrimSpace(strings.Join(extra, "\n"))
		if text == "" {
			text = t
		} else if t != "" && !strings.Contains(text, t) {
			text += "\n" + t
		}
	}
	if text == "" && len(atts) == 0 {
		return nil
	}
	mentions, _ := (&socket.MentionData{MentionIDs: wm.MentionIds, MentionOffsets: wm.MentionOffsets,
		MentionLengths: wm.MentionLengths, MentionTypes: wm.MentionTypes}).Parse()
	ev := map[string]any{
		"chat": chat, "group": group, "e2ee": e2ee, "id": wm.MessageId, "ts": wm.TimestampMs / 1000,
		"sender": sender, "sender_name": m.st.UserName(ctx, sender), "me": sender == own,
		"text": text, "admin": wm.IsAdminMessage,
	}
	if len(mentions) > 0 {
		ev["mentions"] = m.mentionNames(ctx, text, mentions)
	}
	if len(atts) > 0 {
		ev["attachments"] = atts
	}
	if wm.ReplySourceId != "" {
		reply := map[string]any{"id": wm.ReplySourceId, "text": orStr(wm.ReplyMessageText, wm.ReplySnippet)}
		if wm.ReplyToUserId != 0 {
			rs := strconv.FormatInt(wm.ReplyToUserId, 10)
			reply["sender"], reply["sender_name"], reply["me"] = rs, m.st.UserName(ctx, rs), rs == own
		}
		ev["reply"] = reply
	}
	if group {
		if c, _ := m.st.GetChat(ctx, chat); c != nil {
			ev["chat_title"] = c.Title
		}
	}
	_ = m.st.PutMsg(ctx, &MsgRef{ID: wm.MessageId, Chat: chat, Sender: sender, Me: sender == own, E2EE: false, TS: wm.TimestampMs, Text: shownText(text, atts)})
	_ = m.st.PutChat(ctx, &Chat{ID: chat, Kind: kindStr(group), FBKey: wm.ThreadKey, LastTS: wm.TimestampMs / 1000})
	return ev
}

// mentionNames lists who was mentioned: "Name (id)".
func (m *messenger) mentionNames(ctx context.Context, text string, mentions socket.Mentions) []string {
	var out []string
	for _, mt := range mentions {
		if mt.ID == 0 {
			continue
		}
		id := strconv.FormatInt(mt.ID, 10)
		name := m.st.UserName(ctx, id)
		if name == "" {
			runes := []rune(text)
			if mt.Offset >= 0 && mt.Offset+mt.Length <= len(runes) {
				name = strings.TrimPrefix(string(runes[mt.Offset:mt.Offset+mt.Length]), "@")
			}
		}
		out = append(out, fmt.Sprintf("%s (%s)", name, id))
	}
	return out
}

func blobAttachment(typ table.AttachmentType, playURL, playMime, attMime, prevURL, prevMime, name string, size, durMS int64) Attachment {
	u, mt := playURL, orStr(playMime, attMime)
	if u == "" {
		u, mt = prevURL, orStr(prevMime, attMime)
	}
	var kind string
	switch typ {
	case table.AttachmentTypeImage:
		kind = "image"
	case table.AttachmentTypeAnimatedImage:
		kind = "gif"
	case table.AttachmentTypeVideo:
		kind = "video"
	case table.AttachmentTypeAudio, table.AttachmentTypeSoundBite:
		kind = "audio"
	case table.AttachmentTypeFile:
		kind = "document"
	case table.AttachmentTypeSticker, table.AttachmentTypeSelfieSticker, table.AttachmentTypeThirdPartySticker:
		kind = "sticker"
	case table.AttachmentTypeEphemeralImage, table.AttachmentTypeEphemeralVideo:
		return Attachment{Kind: "image", Note: "view-once media: only viewable in the Messenger app"}
	default:
		kind = kindOf(mt)
	}
	a := Attachment{Kind: kind, Mime: mt, Name: name, Size: size, Seconds: float64(durMS) / 1000}
	if u == "" {
		a.Note = "no download link"
		return a
	}
	a.Ref = urlRef(u, mt)
	return a
}

// xmaText describes a shared link, location or post as text.
func xmaText(x *table.WrappedXMA) string {
	var parts []string
	if x.CTA != nil && x.CTA.Type_ == "xma_live_location_sharing" {
		return strings.TrimSpace("[live location] " + x.TitleText + " " + x.SubtitleText)
	}
	if x.CTA != nil && strings.HasPrefix(x.CTA.Type_, "xma_poll_") {
		return strings.TrimSpace("[poll] " + x.TitleText)
	}
	if x.CTA != nil && strings.HasPrefix(x.CTA.NativeUrl, "geo:") {
		ll := strings.TrimPrefix(x.CTA.NativeUrl, "geo:")
		return strings.TrimSpace(fmt.Sprintf("[location] %s %s — %s https://maps.google.com/?q=%s", x.TitleText, x.SubtitleText, ll, ll))
	}
	if x.TitleText != "" {
		parts = append(parts, x.TitleText)
	}
	if x.SubtitleText != "" && x.SubtitleText != x.TitleText {
		parts = append(parts, x.SubtitleText)
	}
	if x.CTA != nil && x.CTA.ActionUrl != "" {
		u := removeLPHP(x.CTA.ActionUrl)
		if strings.HasPrefix(u, "/") {
			u = "https://www.facebook.com" + u
		}
		parts = append(parts, u)
	}
	if len(parts) == 0 {
		return ""
	}
	return "[shared] " + strings.Join(parts, " — ")
}

// --- encrypted messages ---

func (m *messenger) senderKey(info waTypes.MessageInfo) string {
	return info.Sender.ToNonAD().User
}

func (m *messenger) handleFBMessage(ctx context.Context, evt *events.FBMessage) {
	info := evt.Info
	chat := waChatKey(info.Chat)
	group := info.Chat.Server == waTypes.GroupServer
	sender := m.senderKey(info)
	own := m.ownID()
	me := info.IsFromMe || sender == own
	c := &Chat{ID: chat, Kind: kindStr(group), E2EE: true, WAJID: info.Chat.String(), LastTS: info.Timestamp.Unix()}
	if !group {
		c.Peer = chat
	}
	_ = m.st.PutChat(ctx, c)
	if info.PushName != "" && !me {
		_ = m.st.PutUser(ctx, &User{ID: sender, Name: info.PushName})
	}
	base := map[string]any{
		"chat": chat, "group": group, "e2ee": true, "id": info.ID, "ts": info.Timestamp.Unix(),
		"sender": sender, "sender_name": m.st.UserName(ctx, sender), "me": me,
	}
	if group {
		if ch, _ := m.st.GetChat(ctx, chat); ch != nil && ch.Title != "" {
			base["chat_title"] = ch.Title
		} else if e2ee, _ := m.encryptedNow(); e2ee != nil {
			if gi, err := e2ee.GetGroupInfo(ctx, info.Chat); err == nil && gi.Name != "" {
				_ = m.st.PutChat(ctx, &Chat{ID: chat, Kind: "group", E2EE: true, Title: gi.Name})
				base["chat_title"] = gi.Name
			}
		}
	}
	if name, _ := base["sender_name"].(string); name == "" && !me {
		go m.fetchName(sender)
	}
	switch msg := evt.Message.(type) {
	case *waConsumerApplication.ConsumerApplication:
		m.consumerMessage(ctx, evt, msg, base)
	case *waArmadilloApplication.Armadillo:
		content := msg.GetPayload().GetContent()
		if content == nil {
			return
		}
		text, atts := m.armadilloContent(ctx, evt, content)
		m.emitMessage(ctx, evt, base, text, atts)
	}
}

func (m *messenger) encryptedNow() (*whatsmeow.Client, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.e2ee == nil {
		return nil, fmt.Errorf("not connected")
	}
	return m.e2ee, nil
}

// asked: people whose name was already asked for (once each)
var asked sync.Map

// fetchName asks Messenger for the name of someone it has not told bia about yet.
func (m *messenger) fetchName(id string) {
	if _, done := asked.LoadOrStore(id, true); done {
		return
	}
	cli, err := m.client()
	if err != nil {
		return
	}
	fbid, _ := strconv.ParseInt(id, 10, 64)
	if fbid == 0 {
		return
	}
	ctx, cancel := context.WithTimeout(m.rootCtx, 20*time.Second)
	defer cancel()
	resp, err := cli.ExecuteTasks(ctx, &socket.GetContactsFullTask{ContactID: fbid})
	if err != nil || resp == nil {
		return
	}
	for _, c := range resp.LSDeleteThenInsertContact {
		_ = m.st.PutUser(ctx, &User{ID: strconv.FormatInt(c.GetFBID(), 10), Name: c.GetName(), Username: c.GetUsername()})
	}
}

func (m *messenger) consumerMessage(ctx context.Context, evt *events.FBMessage, msg *waConsumerApplication.ConsumerApplication, base map[string]any) {
	switch payload := msg.GetPayload().GetPayload().(type) {
	case *waConsumerApplication.ConsumerApplication_Payload_Content:
		switch content := payload.Content.GetContent().(type) {
		case *waConsumerApplication.ConsumerApplication_Content_EditMessage:
			id := content.EditMessage.GetKey().GetID()
			text := content.EditMessage.GetMessage().GetText()
			ev := copyMap(base)
			if ref, _ := m.st.GetMsg(ctx, id); ref != nil {
				ev["old"], ev["sent_ts"] = ref.Text, ref.TS/1000
			}
			m.st.SetMsgText(ctx, id, text)
			ev["id"], ev["text"] = id, text
			if ts := content.EditMessage.GetTimestampMS(); ts > 0 {
				ev["ts"] = ts / 1000
			}
			emit("edit", ev)
		case *waConsumerApplication.ConsumerApplication_Content_ReactionMessage:
			ev := copyMap(base)
			ev["id"], ev["emoji"] = content.ReactionMessage.GetKey().GetID(), content.ReactionMessage.GetText()
			emit("reaction", ev)
		default:
			text, atts := m.consumerContent(ctx, evt, payload.Content)
			m.emitMessage(ctx, evt, base, text, atts)
		}
	case *waConsumerApplication.ConsumerApplication_Payload_ApplicationData:
		if rv := payload.ApplicationData.GetRevoke(); rv != nil {
			id := rv.GetKey().GetID()
			m.st.DeleteMsg(ctx, id)
			ev := copyMap(base)
			ev["id"] = id
			emit("delete", ev)
		}
	}
}

func copyMap(in map[string]any) map[string]any {
	out := make(map[string]any, len(in))
	for k, v := range in {
		out[k] = v
	}
	return out
}

func (m *messenger) emitMessage(ctx context.Context, evt *events.FBMessage, base map[string]any, text string, atts []Attachment) {
	if text == "" && len(atts) == 0 {
		return
	}
	ev := copyMap(base)
	ev["text"] = text
	if len(atts) > 0 {
		ev["attachments"] = atts
	}
	if qm := evt.FBApplication.GetMetadata().GetQuotedMessage(); qm != nil && qm.GetStanzaID() != "" {
		reply := map[string]any{"id": qm.GetStanzaID()}
		if ref, _ := m.st.GetMsg(ctx, qm.GetStanzaID()); ref != nil {
			reply["text"], reply["sender"], reply["me"] = ref.Text, ref.Sender, ref.Me
			reply["sender_name"] = m.st.UserName(ctx, ref.Sender)
		} else if p, err := waTypes.ParseJID(qm.GetParticipant()); err == nil {
			reply["sender"] = p.User
			reply["sender_name"] = m.st.UserName(ctx, p.User)
		}
		ev["reply"] = reply
	}
	_ = m.st.PutMsg(ctx, &MsgRef{ID: evt.Info.ID, Chat: base["chat"].(string), Sender: base["sender"].(string),
		Me: base["me"].(bool), E2EE: true, TS: evt.Info.Timestamp.UnixMilli(), Text: shownText(text, atts)})
	emit("message", ev)
}

func (m *messenger) waRef(integral *waMediaTransport.WAMediaTransport_Integral, mediaType whatsmeow.MediaType, mimeType string) string {
	return media.add(func(ctx context.Context) ([]byte, string, error) {
		e2ee, err := m.encrypted(ctx)
		if err != nil {
			return nil, "", err
		}
		data, err := e2ee.DownloadFB(ctx, integral, mediaType)
		return data, mimeType, err
	})
}

func (m *messenger) transportAttachment(t *waMediaTransport.WAMediaTransport, mediaType whatsmeow.MediaType, kind, name string, seconds float64) Attachment {
	mt := t.GetAncillary().GetMimetype()
	a := Attachment{Kind: kind, Mime: mt, Name: name, Size: int64(t.GetAncillary().GetFileLength()), Seconds: seconds}
	if t.GetIntegral().GetDirectPath() == "" {
		a.Note = "no download path"
		return a
	}
	a.Ref = m.waRef(t.GetIntegral(), mediaType, mt)
	return a
}

func caption(t *waCommon.MessageText) string {
	return t.GetText()
}

func (m *messenger) consumerContent(ctx context.Context, evt *events.FBMessage, rc *waConsumerApplication.ConsumerApplication_Content) (string, []Attachment) {
	switch c := rc.GetContent().(type) {
	case *waConsumerApplication.ConsumerApplication_Content_MessageText:
		return c.MessageText.GetText(), nil
	case *waConsumerApplication.ConsumerApplication_Content_ExtendedTextMessage:
		text := c.ExtendedTextMessage.GetText().GetText()
		if u := c.ExtendedTextMessage.GetCanonicalURL(); u != "" && !strings.Contains(text, u) {
			text = strings.TrimSpace(text + "\n" + u)
		}
		return text, nil
	case *waConsumerApplication.ConsumerApplication_Content_ImageMessage:
		return m.imageMessage(c.ImageMessage)
	case *waConsumerApplication.ConsumerApplication_Content_VideoMessage:
		return m.videoMessage(c.VideoMessage)
	case *waConsumerApplication.ConsumerApplication_Content_AudioMessage:
		t, err := c.AudioMessage.Decode()
		if err != nil {
			return "", []Attachment{{Kind: "audio", Note: "could not read: " + err.Error()}}
		}
		// Messenger's own clients do not set the voice flag: every clip is a voice message
		return "", []Attachment{m.transportAttachment(t.GetIntegral().GetTransport(), whatsmeow.MediaAudio, "voice", "", float64(t.GetAncillary().GetSeconds()))}
	case *waConsumerApplication.ConsumerApplication_Content_DocumentMessage:
		t, err := c.DocumentMessage.Decode()
		if err != nil {
			return "", []Attachment{{Kind: "document", Note: "could not read: " + err.Error()}}
		}
		return "", []Attachment{m.transportAttachment(t.GetIntegral().GetTransport(), whatsmeow.MediaDocument, "document", c.DocumentMessage.GetFileName(), 0)}
	case *waConsumerApplication.ConsumerApplication_Content_StickerMessage:
		t, err := c.StickerMessage.Decode()
		if err != nil {
			return "", []Attachment{{Kind: "sticker", Note: "could not read: " + err.Error()}}
		}
		if rf := t.GetAncillary().GetReceiverFetchID(); rf != "" {
			ref := media.add(func(ctx context.Context) ([]byte, string, error) {
				cli, err := m.client()
				if err != nil {
					return nil, "", err
				}
				st, err := cli.FetchReceiverFetchMedia(ctx, rf)
				if err != nil {
					return nil, "", err
				}
				return media.fetchURL(ctx, st.URL, st.MimeType)
			})
			return "", []Attachment{{Kind: "sticker", Ref: ref}}
		}
		return "", []Attachment{m.transportAttachment(t.GetIntegral().GetTransport(), whatsmeow.MediaImage, "sticker", "", 0)}
	case *waConsumerApplication.ConsumerApplication_Content_ViewOnceMessage:
		return "", []Attachment{{Kind: "image", Note: "view-once media: only viewable in the Messenger app"}}
	case *waConsumerApplication.ConsumerApplication_Content_LocationMessage:
		loc := c.LocationMessage.GetLocation()
		return locationText("location", loc.GetDegreesLatitude(), loc.GetDegreesLongitude(), loc.GetName(), c.LocationMessage.GetAddress()), nil
	case *waConsumerApplication.ConsumerApplication_Content_LiveLocationMessage:
		loc := c.LiveLocationMessage.GetLocation()
		return locationText("live location", loc.GetDegreesLatitude(), loc.GetDegreesLongitude(), loc.GetName(), caption(c.LiveLocationMessage.GetCaption())), nil
	case *waConsumerApplication.ConsumerApplication_Content_ContactMessage:
		return contactText(c.ContactMessage), nil
	case *waConsumerApplication.ConsumerApplication_Content_ContactsArrayMessage:
		var lines []string
		for _, ct := range c.ContactsArrayMessage.GetContacts() {
			lines = append(lines, contactText(ct))
		}
		return strings.Join(lines, "\n"), nil
	}
	return "[a message type bia cannot show; open it in the Messenger app]", nil
}

func (m *messenger) imageMessage(im *waConsumerApplication.ConsumerApplication_ImageMessage) (string, []Attachment) {
	t, err := im.Decode()
	if err != nil {
		return caption(im.GetCaption()), []Attachment{{Kind: "image", Note: "could not read: " + err.Error()}}
	}
	return caption(im.GetCaption()), []Attachment{m.transportAttachment(t.GetIntegral().GetTransport(), whatsmeow.MediaImage, "image", "", 0)}
}

func (m *messenger) videoMessage(vm *waConsumerApplication.ConsumerApplication_VideoMessage) (string, []Attachment) {
	t, err := vm.Decode()
	if err != nil {
		return caption(vm.GetCaption()), []Attachment{{Kind: "video", Note: "could not read: " + err.Error()}}
	}
	kind := "video"
	if t.GetAncillary().GetGifPlayback() {
		kind = "gif"
	}
	tr := t.GetIntegral().GetTransport()
	if strings.HasPrefix(tr.GetAncillary().GetMimetype(), "image/") {
		kind = "image"
	}
	return caption(vm.GetCaption()), []Attachment{m.transportAttachment(tr, whatsmeow.MediaVideo, kind, "", float64(t.GetAncillary().GetSeconds()))}
}

func locationText(kind string, lat, lon float64, name, extra string) string {
	place := strings.TrimSpace(strings.Join(nonEmpty(name, extra), ", "))
	if place != "" {
		place += " — "
	}
	return fmt.Sprintf("[%s] %s%.6f, %.6f https://maps.google.com/?q=%.6f,%.6f", kind, place, lat, lon, lat, lon)
}

func nonEmpty(s ...string) []string {
	var out []string
	for _, v := range s {
		if strings.TrimSpace(v) != "" {
			out = append(out, strings.TrimSpace(v))
		}
	}
	return out
}

func contactText(ct *waConsumerApplication.ConsumerApplication_ContactMessage) string {
	t, err := ct.Decode()
	if err != nil {
		return "[contact]"
	}
	name := t.GetAncillary().GetDisplayName()
	vcard := t.GetIntegral().GetVcard()
	var phones []string
	for _, line := range strings.Split(vcard, "\n") {
		line = strings.TrimSpace(line)
		up := strings.ToUpper(line)
		if strings.HasPrefix(up, "TEL") {
			if i := strings.LastIndex(line, ":"); i >= 0 {
				phones = append(phones, strings.TrimSpace(line[i+1:]))
			}
		} else if name == "" && strings.HasPrefix(up, "FN") {
			if i := strings.Index(line, ":"); i >= 0 {
				name = strings.TrimSpace(line[i+1:])
			}
		}
	}
	return strings.TrimSpace("[contact] " + strings.Join(nonEmpty(append([]string{name}, phones...)...), ", "))
}

func (m *messenger) armadilloContent(ctx context.Context, evt *events.FBMessage, content *waArmadilloApplication.Armadillo_Content) (string, []Attachment) {
	switch c := content.GetContent().(type) {
	case *waArmadilloApplication.Armadillo_Content_ExtendedContentMessage:
		return extendedText(c.ExtendedContentMessage), nil
	case *waArmadilloApplication.Armadillo_Content_RavenMessage_:
		return m.ravenMessage(c.RavenMessage)
	case *waArmadilloApplication.Armadillo_Content_RavenMessageMsgr:
		return m.ravenMessage(c.RavenMessageMsgr)
	case *waArmadilloApplication.Armadillo_Content_ImageGalleryMessage_:
		images, err := c.ImageGalleryMessage.Decode()
		if err != nil {
			return "", []Attachment{{Kind: "image", Note: "could not read: " + err.Error()}}
		}
		var atts []Attachment
		for _, img := range images {
			atts = append(atts, m.transportAttachment(img.GetIntegral().GetTransport(), whatsmeow.MediaImage, "image", "", 0))
		}
		return "", atts
	case *waArmadilloApplication.Armadillo_Content_BumpExistingMessage_:
		return "[bumped an earlier message]", nil
	}
	return "[a message type bia cannot show; open it in the Messenger app]", nil
}

func (m *messenger) ravenMessage(r *waArmadilloApplication.Armadillo_Content_RavenMessage) (string, []Attachment) {
	viewOnce := r.GetEphemeralType() == waArmadilloApplication.Armadillo_Content_RavenMessage_VIEW_ONCE ||
		r.GetEphemeralType() == waArmadilloApplication.Armadillo_Content_RavenMessage_ALLOW_REPLAY
	note := ""
	if viewOnce {
		note = "view-once"
	}
	if r.GetImageMessage() != nil {
		img, err := r.DecodeImageMessage()
		if err != nil {
			return "", []Attachment{{Kind: "image", Note: "could not read: " + err.Error()}}
		}
		a := m.transportAttachment(img.GetIntegral().GetTransport(), whatsmeow.MediaImage, "image", "", 0)
		if note != "" {
			a.Name = note
		}
		return "", []Attachment{a}
	}
	if r.GetVideoMessage() != nil {
		vid, err := r.DecodeVideoMessage()
		if err != nil {
			return "", []Attachment{{Kind: "video", Note: "could not read: " + err.Error()}}
		}
		a := m.transportAttachment(vid.GetIntegral().GetTransport(), whatsmeow.MediaVideo, "video", "", float64(vid.GetAncillary().GetSeconds()))
		if note != "" {
			a.Name = note
		}
		return "", []Attachment{a}
	}
	return "[disappearing media; open it in the Messenger app]", nil
}

func extendedText(x *waArmadilloXMA.ExtendedContentMessage) string {
	body := x.GetMessageText()
	for _, cta := range x.GetCtas() {
		parsed, err := url.Parse(cta.GetNativeURL())
		if err == nil && parsed.Scheme == "messenger" && parsed.Host == "location_share" {
			lat, _ := strconv.ParseFloat(parsed.Query().Get("lat"), 64)
			lon, _ := strconv.ParseFloat(parsed.Query().Get("long"), 64)
			return locationText("location", lat, lon, x.GetTitleText(), x.GetSubtitleText())
		}
	}
	var link string
	for _, cta := range x.GetCtas() {
		for _, cand := range []string{cta.GetNativeURL(), cta.GetActionURL()} {
			if p, err := url.Parse(cand); err == nil && (p.Scheme == "https" || p.Scheme == "http") && p.Host != "" {
				link = removeLPHP(cand)
				break
			}
		}
		if link != "" {
			break
		}
	}
	parts := nonEmpty(x.GetTitleText(), x.GetSubtitleText())
	if link != "" {
		parts = append(parts, link)
	}
	shared := ""
	if len(parts) > 0 {
		label := "shared"
		if x.GetTargetType() == waArmadilloXMA.ExtendedContentMessage_FB_STORY_REPLY {
			label = "reply to a story"
		}
		shared = "[" + label + "] " + strings.Join(parts, " — ")
	}
	if body == "" {
		return shared
	}
	if shared == "" || strings.Contains(body, link) && link != "" {
		return body
	}
	return body + "\n" + shared
}
