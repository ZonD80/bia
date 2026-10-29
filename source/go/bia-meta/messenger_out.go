package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"image"
	"image/color"
	"image/draw"
	_ "image/gif"
	_ "image/jpeg"
	"image/png"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/google/uuid"
	"go.mau.fi/util/ptr"
	"go.mau.fi/whatsmeow"
	armadillo "go.mau.fi/whatsmeow/proto"
	"go.mau.fi/whatsmeow/proto/waCommon"
	"go.mau.fi/whatsmeow/proto/waConsumerApplication"
	"go.mau.fi/whatsmeow/proto/waMediaTransport"
	"go.mau.fi/whatsmeow/proto/waMsgApplication"
	waTypes "go.mau.fi/whatsmeow/types"
	_ "golang.org/x/image/webp"
	"google.golang.org/protobuf/proto"

	"go.mau.fi/mautrix-meta/pkg/messagix/graphql"
	"go.mau.fi/mautrix-meta/pkg/messagix/httpclient"
	"go.mau.fi/mautrix-meta/pkg/messagix/methods"
	"go.mau.fi/mautrix-meta/pkg/messagix/socket"
	"go.mau.fi/mautrix-meta/pkg/messagix/table"
)

// --- sending ---

func (m *messenger) Send(ctx context.Context, p SendParams) (map[string]any, error) {
	if strings.TrimSpace(p.Text) == "" && len(p.Files) == 0 {
		return nil, errors.New("text or files required")
	}
	t, err := m.target(ctx, p.Chat)
	if err != nil {
		return nil, err
	}
	var reply *MsgRef
	if p.ReplyTo != "" {
		reply, _ = m.st.GetMsg(ctx, p.ReplyTo)
		if reply == nil {
			reply = &MsgRef{ID: p.ReplyTo, Chat: t.key}
		}
	}
	if t.e2ee {
		return m.sendE2EE(ctx, t, p, reply)
	}
	return m.sendClassic(ctx, t, p, reply)
}

// ensureDM makes sure an encrypted DM exists with someone bia has not written to yet.
func (m *messenger) ensureDM(ctx context.Context, t *target) {
	if t.chat != nil || t.group {
		return
	}
	cli, err := m.readyClient(ctx)
	if err != nil {
		return
	}
	fbid, _ := strconv.ParseInt(t.key, 10, 64)
	_, _ = cli.ExecuteTasks(ctx, &socket.CreateThreadTask{ThreadFBID: fbid, SyncGroup: 1})
	resp, err := cli.ExecuteTasks(ctx, &socket.CreateWhatsAppThreadTask{
		WAJID: fbid, OfflineThreadKey: methods.GenerateEpochID(), ThreadType: table.ENCRYPTED_OVER_WA_ONE_TO_ONE,
		FolderType: table.INBOX, BumpTimestampMS: time.Now().UnixMilli(),
	})
	if err == nil && resp != nil && len(resp.LSIssueNewTask) > 0 {
		tasks := make([]socket.Task, len(resp.LSIssueNewTask))
		for i, task := range resp.LSIssueNewTask {
			tasks[i] = task
		}
		_, _ = cli.ExecuteTasks(ctx, tasks...)
	}
	_ = m.st.PutChat(ctx, &Chat{ID: t.key, Kind: "dm", E2EE: true, Peer: t.key, FBKey: fbid,
		WAJID: t.jid.String(), LastTS: time.Now().Unix()})
}

func (m *messenger) waKey(t *target, ref *MsgRef) *waCommon.MessageKey {
	key := &waCommon.MessageKey{RemoteJID: ptr.Ptr(t.jid.String()), ID: ptr.Ptr(ref.ID)}
	if ref.Me {
		key.FromMe = ptr.Ptr(true)
	}
	if t.group && ref.Sender != "" {
		key.Participant = ptr.Ptr(waTypes.NewJID(ref.Sender, waTypes.MessengerServer).String())
	}
	return key
}

func consumerContent(c *waConsumerApplication.ConsumerApplication_Content) *waConsumerApplication.ConsumerApplication {
	return &waConsumerApplication.ConsumerApplication{
		Payload: &waConsumerApplication.ConsumerApplication_Payload{
			Payload: &waConsumerApplication.ConsumerApplication_Payload_Content{Content: c},
		},
	}
}

func (m *messenger) sendE2EE(ctx context.Context, t *target, p SendParams, reply *MsgRef) (map[string]any, error) {
	m.ensureDM(ctx, t)
	e2ee, err := m.encrypted(ctx)
	if err != nil {
		return nil, err
	}
	var meta waMsgApplication.MessageApplication_Metadata
	if reply != nil {
		meta.QuotedMessage = &waMsgApplication.MessageApplication_Metadata_QuotedMessage{StanzaID: ptr.Ptr(reply.ID)}
		if reply.Sender != "" {
			meta.QuotedMessage.Participant = ptr.Ptr(waTypes.NewJID(reply.Sender, waTypes.MessengerServer).String())
		}
	}
	var ids []string
	send := func(msg armadillo.RealMessageApplicationSub, shown string) error {
		otid := strconv.FormatInt(methods.GenerateEpochID(), 10)
		resp, err := e2ee.SendFBMessage(ctx, t.jid, msg, &meta, whatsmeow.SendRequestExtra{ID: otid})
		if err != nil {
			return err
		}
		id := orStr(string(resp.ID), otid)
		ids = append(ids, id)
		_ = m.st.PutMsg(ctx, &MsgRef{ID: id, Chat: t.key, Sender: m.ownID(), Me: true, E2EE: true, TS: time.Now().UnixMilli(), Text: shown})
		meta.QuotedMessage = nil
		return nil
	}
	text := p.Text
	for i, f := range p.Files {
		content, shown, err := m.e2eeMedia(ctx, e2ee, f, text)
		if err != nil {
			return sentSoFar(ids, fmt.Errorf("%s: %w", filepath.Base(f.Path), err))
		}
		if err = send(consumerContent(content), shown); err != nil {
			return sentSoFar(ids, fmt.Errorf("send %s: %w", filepath.Base(f.Path), err))
		}
		if i == 0 && hasCaption(content) {
			text = ""
		}
	}
	if strings.TrimSpace(text) != "" {
		content := &waConsumerApplication.ConsumerApplication_Content{
			Content: &waConsumerApplication.ConsumerApplication_Content_MessageText{MessageText: &waCommon.MessageText{Text: ptr.Ptr(text)}},
		}
		if err = send(consumerContent(content), text); err != nil {
			return sentSoFar(ids, err)
		}
	}
	_ = m.st.PutChat(ctx, &Chat{ID: t.key, Kind: kindStr(t.group), E2EE: true, LastTS: time.Now().Unix()})
	return map[string]any{"ids": ids, "e2ee": true}, nil
}

func hasCaption(c *waConsumerApplication.ConsumerApplication_Content) bool {
	switch v := c.GetContent().(type) {
	case *waConsumerApplication.ConsumerApplication_Content_ImageMessage:
		return v.ImageMessage.GetCaption().GetText() != ""
	case *waConsumerApplication.ConsumerApplication_Content_VideoMessage:
		return v.VideoMessage.GetCaption().GetText() != ""
	}
	return false
}

// sentSoFar reports a failure after some parts already went out.
func sentSoFar(ids []string, err error) (map[string]any, error) {
	if len(ids) == 0 {
		return nil, err
	}
	return nil, fmt.Errorf("%w (already sent: %s)", err, strings.Join(ids, ", "))
}

// toM4A converts audio for Messenger voice clips (AAC in MP4); without ffmpeg the file goes as it is.
func toM4A(ctx context.Context, data []byte, mimeType string) ([]byte, string) {
	if strings.HasPrefix(mimeType, "audio/mp4") || strings.HasPrefix(mimeType, "audio/aac") {
		return data, mimeType
	}
	if _, err := exec.LookPath("ffmpeg"); err != nil {
		return data, mimeType
	}
	dir, err := os.MkdirTemp("", "bia-meta-voice-")
	if err != nil {
		return data, mimeType
	}
	defer os.RemoveAll(dir)
	in, out := filepath.Join(dir, "in"), filepath.Join(dir, "out.m4a")
	if os.WriteFile(in, data, 0o600) != nil {
		return data, mimeType
	}
	cmd := exec.CommandContext(ctx, "ffmpeg", "-y", "-loglevel", "error", "-i", in, "-vn", "-c:a", "aac", "-b:a", "64k", out)
	if err = cmd.Run(); err != nil {
		log.Warn().Err(err).Msg("ffmpeg voice conversion")
		return data, mimeType
	}
	conv, err := os.ReadFile(out)
	if err != nil || len(conv) == 0 {
		return data, mimeType
	}
	return conv, "audio/mp4"
}

func clamp400(w, h int) (int, int) {
	if w > 400 {
		h, w = h*400/w, 400
	}
	if h > 400 {
		w, h = w*400/h, 400
	}
	return w, h
}

// fixPNG re-encodes PNGs Messenger refuses (it wants NRGBA).
func fixPNG(data []byte) []byte {
	cfg, err := png.DecodeConfig(bytes.NewReader(data))
	if err != nil || cfg.ColorModel == color.NRGBAModel {
		return data
	}
	img, err := png.Decode(bytes.NewReader(data))
	if err != nil {
		return data
	}
	b := img.Bounds()
	nrgba := image.NewNRGBA(image.Rect(0, 0, b.Dx(), b.Dy()))
	draw.Draw(nrgba, nrgba.Bounds(), img, b.Min, draw.Src)
	var buf bytes.Buffer
	if png.Encode(&buf, nrgba) != nil {
		return data
	}
	return buf.Bytes()
}

func (m *messenger) e2eeMedia(ctx context.Context, e2ee *whatsmeow.Client, f FileParam, text string) (*waConsumerApplication.ConsumerApplication_Content, string, error) {
	data, mimeType, name, err := readFile(f)
	if err != nil {
		return nil, "", err
	}
	kind := kindOf(mimeType)
	voice := f.Kind == "voice"
	if voice {
		data, mimeType = toM4A(ctx, data, mimeType)
		kind = "audio"
	}
	var w, h int
	if kind == "image" {
		if mimeType == "image/png" {
			data = fixPNG(data)
		}
		if cfg, _, err := image.DecodeConfig(bytes.NewReader(data)); err == nil {
			w, h = cfg.Width, cfg.Height
		}
	}
	mediaType := whatsmeow.MediaDocument
	switch kind {
	case "image":
		mediaType = whatsmeow.MediaImage
	case "gif", "video":
		mediaType = whatsmeow.MediaVideo
	case "audio":
		mediaType = whatsmeow.MediaAudio
	}
	up, err := e2ee.Upload(ctx, data, mediaType)
	if err != nil {
		return nil, "", fmt.Errorf("upload: %w", err)
	}
	tw, th := clamp400(w, h)
	if tw == 0 && kind == "image" {
		tw, th = 400, 400
	}
	transport := &waMediaTransport.WAMediaTransport{
		Integral: &waMediaTransport.WAMediaTransport_Integral{
			FileSHA256: up.FileSHA256, MediaKey: up.MediaKey, FileEncSHA256: up.FileEncSHA256,
			DirectPath: &up.DirectPath, MediaKeyTimestamp: proto.Int64(time.Now().Unix()),
		},
		Ancillary: &waMediaTransport.WAMediaTransport_Ancillary{
			FileLength: proto.Uint64(uint64(len(data))), Mimetype: &mimeType,
			// Messenger's apps do not show media without a thumbnail size
			Thumbnail: &waMediaTransport.WAMediaTransport_Ancillary_Thumbnail{ThumbnailWidth: proto.Uint32(uint32(tw)), ThumbnailHeight: proto.Uint32(uint32(th))},
			ObjectID:  &up.ObjectID,
		},
	}
	var capt *waCommon.MessageText
	if text != "" {
		capt = &waCommon.MessageText{Text: ptr.Ptr(text)}
	}
	out := &waConsumerApplication.ConsumerApplication_Content{}
	switch kind {
	case "image":
		msg := &waConsumerApplication.ConsumerApplication_ImageMessage{Caption: capt}
		err = msg.Set(&waMediaTransport.ImageTransport{
			Integral:  &waMediaTransport.ImageTransport_Integral{Transport: transport},
			Ancillary: &waMediaTransport.ImageTransport_Ancillary{Width: proto.Uint32(uint32(w)), Height: proto.Uint32(uint32(h))},
		})
		out.Content = &waConsumerApplication.ConsumerApplication_Content_ImageMessage{ImageMessage: msg}
		return out, labelled("image", text, name), err
	case "gif", "video":
		msg := &waConsumerApplication.ConsumerApplication_VideoMessage{Caption: capt}
		anc := &waMediaTransport.VideoTransport_Ancillary{}
		if kind == "gif" {
			anc.GifPlayback = proto.Bool(true)
		}
		err = msg.Set(&waMediaTransport.VideoTransport{Integral: &waMediaTransport.VideoTransport_Integral{Transport: transport}, Ancillary: anc})
		out.Content = &waConsumerApplication.ConsumerApplication_Content_VideoMessage{VideoMessage: msg}
		return out, labelled("video", text, name), err
	case "audio":
		msg := &waConsumerApplication.ConsumerApplication_AudioMessage{PTT: proto.Bool(voice)}
		err = msg.Set(&waMediaTransport.AudioTransport{
			Integral:  &waMediaTransport.AudioTransport_Integral{Transport: transport},
			Ancillary: &waMediaTransport.AudioTransport_Ancillary{Seconds: proto.Uint32(uint32(f.Seconds + 0.5))},
		})
		out.Content = &waConsumerApplication.ConsumerApplication_Content_AudioMessage{AudioMessage: msg}
		label := "audio"
		if voice {
			label = "voice"
		}
		return out, labelled(label, "", name), err
	default:
		msg := &waConsumerApplication.ConsumerApplication_DocumentMessage{FileName: &name}
		err = msg.Set(&waMediaTransport.DocumentTransport{
			Integral:  &waMediaTransport.DocumentTransport_Integral{Transport: transport},
			Ancillary: &waMediaTransport.DocumentTransport_Ancillary{},
		})
		out.Content = &waConsumerApplication.ConsumerApplication_Content_DocumentMessage{DocumentMessage: msg}
		return out, labelled("document", "", name), err
	}
}

func labelled(kind, text, name string) string {
	if text != "" {
		return "[" + kind + "] " + text
	}
	return "[" + kind + "] " + name
}

func (m *messenger) sendClassic(ctx context.Context, t *target, p SendParams, reply *MsgRef) (map[string]any, error) {
	cli, err := m.readyClient(ctx)
	if err != nil {
		return nil, err
	}
	if t.fbKey == 0 {
		return nil, fmt.Errorf("chat %s has no Facebook thread", t.key)
	}
	var ids []string
	sendTask := func(task *socket.SendMessageTask, shown string) error {
		task.ThreadId, task.Source, task.InitiatingSource, task.SyncGroup = t.fbKey, table.MESSENGER_INBOX_IN_THREAD, table.FACEBOOK_INBOX, 1
		if t.chat != nil && t.chat.Request {
			task.Source = table.MESSENGER_INBOX_PENDING_REQUESTS
		}
		if reply != nil {
			sender, _ := strconv.ParseInt(reply.Sender, 10, 64)
			task.ReplyMetaData = &socket.ReplyMetaData{ReplyMessageId: reply.ID, ReplySourceType: 1, ReplyType: 0, ReplySender: sender}
		}
		task.Otid = methods.GenerateEpochID()
		read := &socket.ThreadMarkReadTask{ThreadId: t.fbKey, SyncGroup: 1, LastReadWatermarkTs: time.Now().UnixMilli()}
		var resp *table.LSTable
		var err error
		for try := 0; try < 3; try++ {
			if err = cli.WaitUntilCanSendMessages(ctx, 15*time.Second); err != nil {
				continue
			}
			if resp, err = cli.ExecuteTasks(ctx, task, read); err == nil {
				break
			}
		}
		if err != nil {
			return err
		}
		id, err := sentID(resp, task.Otid)
		if err != nil {
			return err
		}
		ids = append(ids, id)
		_ = m.st.PutMsg(ctx, &MsgRef{ID: id, Chat: t.key, Sender: m.ownID(), Me: true, TS: time.Now().UnixMilli(), Text: shown})
		reply = nil
		return nil
	}
	text := p.Text
	for i, f := range p.Files {
		data, mimeType, name, err := readFile(f)
		if err != nil {
			return sentSoFar(ids, err)
		}
		voice := f.Kind == "voice"
		if voice {
			data, mimeType = toM4A(ctx, data, mimeType)
			name = strings.TrimSuffix(name, filepath.Ext(name)) + ".m4a"
		} else if mimeType == "image/png" {
			data = fixPNG(data)
		}
		up, err := cli.GetHTTP().SendMercuryUploadRequest(ctx, t.fbKey, &httpclient.MercuryUploadMedia{
			Filename: name, MimeType: mimeType, MediaData: data, IsVoiceClip: voice,
		})
		if err != nil {
			return sentSoFar(ids, fmt.Errorf("upload %s: %w", name, err))
		}
		fbid := up.Payload.RealMetadata.GetFbId()
		if fbid == 0 {
			return sentSoFar(ids, fmt.Errorf("upload %s: Messenger returned no id", name))
		}
		task := &socket.SendMessageTask{SendType: table.MEDIA, AttachmentFBIds: []int64{fbid}}
		kind := kindOf(mimeType)
		if voice {
			kind = "voice"
		}
		shown := labelled(kind, "", name)
		if i == 0 && text != "" && !voice {
			task.Text, shown, text = text, labelled(kind, text, name), ""
		}
		if err = sendTask(task, shown); err != nil {
			return sentSoFar(ids, err)
		}
	}
	if strings.TrimSpace(text) != "" {
		if err = sendTask(&socket.SendMessageTask{SendType: table.TEXT, Text: text}, text); err != nil {
			return sentSoFar(ids, err)
		}
	}
	return map[string]any{"ids": ids, "e2ee": false}, nil
}

func sentID(resp *table.LSTable, otid int64) (string, error) {
	if resp == nil {
		return "", errors.New("no answer from Messenger")
	}
	want := strconv.FormatInt(otid, 10)
	for _, r := range resp.LSReplaceOptimsiticMessage {
		if r.OfflineThreadingId == want {
			return r.MessageId, nil
		}
	}
	for _, f := range resp.LSMarkOptimisticMessageFailed {
		return "", fmt.Errorf("Messenger refused the message: %s", f.Message)
	}
	for _, f := range resp.LSHandleFailedTask {
		return "", fmt.Errorf("Messenger refused the message: %s", f.Message)
	}
	return "", errors.New("Messenger did not confirm the message")
}

// msgRef finds a message bia knows, defaulting to the given chat.
func (m *messenger) msgRef(ctx context.Context, t *target, id string) *MsgRef {
	ref, _ := m.st.GetMsg(ctx, id)
	if ref == nil {
		ref = &MsgRef{ID: id, Chat: t.key, E2EE: t.e2ee}
	}
	return ref
}

func (m *messenger) Edit(ctx context.Context, p EditParams) error {
	if strings.TrimSpace(p.Text) == "" {
		return errors.New("new text is empty")
	}
	t, err := m.target(ctx, p.Chat)
	if err != nil {
		return err
	}
	ref := m.msgRef(ctx, t, p.ID)
	if ref.Sender != "" && !ref.Me {
		return errors.New("only your own messages can be edited")
	}
	if ref.E2EE || t.e2ee && !isClassicID(p.ID) {
		e2ee, err := m.encrypted(ctx)
		if err != nil {
			return err
		}
		ref.Me = true
		edit := consumerContent(&waConsumerApplication.ConsumerApplication_Content{
			Content: &waConsumerApplication.ConsumerApplication_Content_EditMessage{EditMessage: &waConsumerApplication.ConsumerApplication_EditMessage{
				Key: m.waKey(t, ref), Message: &waCommon.MessageText{Text: ptr.Ptr(p.Text)}, TimestampMS: ptr.Ptr(time.Now().UnixMilli()),
			}},
		})
		_, err = e2ee.SendFBMessage(ctx, t.jid, edit, nil, whatsmeow.SendRequestExtra{ID: strconv.FormatInt(methods.GenerateEpochID(), 10)})
		if err == nil {
			m.st.SetMsgText(ctx, p.ID, p.Text)
		}
		return err
	}
	cli, err := m.readyClient(ctx)
	if err != nil {
		return err
	}
	resp, err := cli.ExecuteTasks(ctx, &socket.EditMessageTask{MessageID: p.ID, Text: p.Text})
	if err != nil {
		return err
	}
	if resp != nil {
		for _, e := range resp.LSEditMessage {
			if e.MessageID == p.ID && e.Text != p.Text {
				// Messenger sometimes answers with the old text first and confirms later
				time.Sleep(2 * time.Second)
			}
		}
	}
	m.st.SetMsgText(ctx, p.ID, p.Text)
	return nil
}

// isClassicID: classic Messenger message ids look like mid.$… ; encrypted ones do not.
func isClassicID(id string) bool {
	return strings.HasPrefix(id, "mid.")
}

func (m *messenger) React(ctx context.Context, p ReactParams) error {
	t, err := m.target(ctx, p.Chat)
	if err != nil {
		return err
	}
	ref := m.msgRef(ctx, t, p.ID)
	emoji := strings.TrimSpace(p.Emoji)
	if ref.E2EE || t.e2ee && !isClassicID(p.ID) {
		e2ee, err := m.encrypted(ctx)
		if err != nil {
			return err
		}
		msg := consumerContent(&waConsumerApplication.ConsumerApplication_Content{
			Content: &waConsumerApplication.ConsumerApplication_Content_ReactionMessage{ReactionMessage: &waConsumerApplication.ConsumerApplication_ReactionMessage{
				Key: m.waKey(t, ref), Text: ptr.Ptr(emoji), SenderTimestampMS: ptr.Ptr(time.Now().UnixMilli()),
			}},
		})
		_, err = e2ee.SendFBMessage(ctx, t.jid, msg, nil, whatsmeow.SendRequestExtra{ID: strconv.FormatInt(methods.GenerateEpochID(), 10)})
		return err
	}
	cli, err := m.readyClient(ctx)
	if err != nil {
		return err
	}
	fbid, _ := strconv.ParseInt(m.ownID(), 10, 64)
	_, err = cli.ExecuteTasks(ctx, &socket.SendReactionTask{
		ThreadKey: t.fbKey, TimestampMs: time.Now().UnixMilli(), MessageID: p.ID, Reaction: emoji,
		ActorID: fbid, SyncGroup: 1, SendAttribution: table.MESSENGER_INBOX_IN_THREAD,
	})
	return err
}

func (m *messenger) Delete(ctx context.Context, p DeleteParams) (map[string]any, error) {
	t, err := m.target(ctx, p.Chat)
	if err != nil {
		return nil, err
	}
	var done, failed []string
	for _, id := range p.IDs {
		if err := m.deleteOne(ctx, t, id, p.Everyone); err != nil {
			failed = append(failed, id+": "+err.Error())
		} else {
			done = append(done, id)
			m.st.DeleteMsg(ctx, id)
		}
	}
	if len(done) == 0 && len(failed) > 0 {
		return nil, errors.New(strings.Join(failed, "; "))
	}
	return map[string]any{"deleted": done, "failed": failed}, nil
}

func (m *messenger) deleteOne(ctx context.Context, t *target, id string, everyone bool) error {
	ref := m.msgRef(ctx, t, id)
	if ref.E2EE || t.e2ee && !isClassicID(id) {
		if !everyone {
			// encrypted chats have no "remove for me" for linked devices: it stays on the phone
			return nil
		}
		if ref.Sender != "" && !ref.Me {
			return errors.New("only your own messages can be deleted for everyone")
		}
		ref.Me = true
		e2ee, err := m.encrypted(ctx)
		if err != nil {
			return err
		}
		msg := &waConsumerApplication.ConsumerApplication{
			Payload: &waConsumerApplication.ConsumerApplication_Payload{
				Payload: &waConsumerApplication.ConsumerApplication_Payload_ApplicationData{
					ApplicationData: &waConsumerApplication.ConsumerApplication_ApplicationData{
						ApplicationContent: &waConsumerApplication.ConsumerApplication_ApplicationData_Revoke{
							Revoke: &waConsumerApplication.ConsumerApplication_RevokeMessage{Key: m.waKey(t, ref)},
						},
					},
				},
			},
		}
		_, err = e2ee.SendFBMessage(ctx, t.jid, msg, nil, whatsmeow.SendRequestExtra{ID: strconv.FormatInt(methods.GenerateEpochID(), 10)})
		return err
	}
	cli, err := m.readyClient(ctx)
	if err != nil {
		return err
	}
	if everyone {
		_, err = cli.ExecuteTasks(ctx, &socket.DeleteMessageTask{MessageId: id})
	} else {
		_, err = cli.ExecuteTasks(ctx, &socket.DeleteMessageMeOnlyTask{ThreadKey: t.fbKey, MessageId: id})
	}
	return err
}

func (m *messenger) Typing(ctx context.Context, p TypingParams) error {
	t, err := m.target(ctx, p.Chat)
	if err != nil {
		return err
	}
	if t.e2ee {
		e2ee, err := m.encryptedNow()
		if err != nil {
			return nil
		}
		state := waTypes.ChatPresencePaused
		if p.On {
			state = waTypes.ChatPresenceComposing
			_ = e2ee.SendPresence(ctx, waTypes.PresenceAvailable)
		}
		return e2ee.SendChatPresence(ctx, t.jid, state, waTypes.ChatPresenceMediaText)
	}
	cli, err := m.client()
	if err != nil {
		return nil
	}
	typ := table.ONE_TO_ONE
	grp := int64(0)
	if t.group {
		typ, grp = table.GROUP_THREAD, 1
	}
	return cli.ExecuteStatelessTask(ctx, &socket.UpdatePresenceTask{
		ThreadKey: t.fbKey, IsGroupThread: grp, IsTyping: int64(b2i(p.On)), SyncGroup: 1, ThreadType: int64(typ),
	})
}

func (m *messenger) MarkRead(ctx context.Context, p ReadParams) error {
	t, err := m.target(ctx, p.Chat)
	if err != nil {
		return err
	}
	ref := m.msgRef(ctx, t, p.ID)
	if ref.E2EE || t.e2ee && !isClassicID(p.ID) {
		if _, err := m.client(); err != nil {
			return notConnected(err)
		}
		e2ee, err := m.encryptedNow()
		if err != nil {
			return notConnected(err)
		}
		var sender waTypes.JID
		if t.group && ref.Sender != "" {
			sender = waTypes.NewJID(ref.Sender, waTypes.MessengerServer)
		}
		return e2ee.MarkRead(ctx, []waTypes.MessageID{p.ID}, time.Now(), t.jid, sender)
	}
	cli, err := m.client()
	if err != nil {
		return notConnected(err)
	}
	ts := ref.TS
	if ts == 0 {
		ts = time.Now().UnixMilli()
	}
	_, err = cli.ExecuteTasks(ctx, &socket.ThreadMarkReadTask{ThreadId: t.fbKey, LastReadWatermarkTs: ts, SyncGroup: 1})
	return err
}

// --- chats, people, history ---

func (m *messenger) chatMap(ctx context.Context, c *Chat) map[string]any {
	title := c.Title
	if c.Kind == "dm" {
		title = orStr(m.st.UserName(ctx, c.Peer), title)
	}
	out := map[string]any{"chat": c.ID, "title": title, "group": c.Kind == "group", "e2ee": c.E2EE, "last_ts": c.LastTS}
	if c.Request {
		out["request"] = true
	}
	return out
}

func (m *messenger) Chats(ctx context.Context, p ChatsParams) ([]map[string]any, error) {
	if _, err := m.client(); errors.Is(err, ErrNotLoggedIn) {
		return nil, err
	}
	chats, err := m.st.Chats(ctx, p.Limit)
	if err != nil {
		return nil, err
	}
	out := make([]map[string]any, 0, len(chats))
	for _, c := range chats {
		if c.Kind == "dm" && c.Peer == m.ownID() {
			continue
		}
		out = append(out, m.chatMap(ctx, c))
	}
	return out, nil
}

func (m *messenger) Search(ctx context.Context, p SearchParams) ([]map[string]any, error) {
	q := strings.TrimSpace(p.Query)
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
		id, _ := r["chat"].(string)
		if id == "" || seen[id] || len(out) >= limit {
			return
		}
		seen[id] = true
		out = append(out, r)
	}
	if local, err := m.st.FindChats(ctx, q, limit); err == nil {
		for _, c := range local {
			add(m.chatMap(ctx, c))
		}
	}
	cli, err := m.readyClient(ctx)
	if err != nil {
		if len(out) > 0 {
			return out, nil
		}
		return nil, err
	}
	task := &socket.SearchUserTask{
		Query: q,
		SupportedTypes: []table.SearchType{table.SearchTypeContact, table.SearchTypeGroup, table.SearchTypePage, table.SearchTypeNonContact,
			table.SearchTypeCommunityMessagingThread},
		SurfaceType: 5,
	}
	second := *task
	second.Secondary = true
	go func() {
		time.Sleep(10 * time.Millisecond)
		_, _ = cli.ExecuteTasks(context.WithoutCancel(ctx), &second)
	}()
	resp, err := cli.ExecuteTasks(ctx, task)
	if err != nil {
		if len(out) > 0 {
			return out, nil
		}
		return nil, fmt.Errorf("search: %w", err)
	}
	for _, r := range resp.LSInsertSearchResult {
		if r.ThreadType.IsOneToOne() {
			if r.GetFBID() == 0 {
				continue
			}
			id := strconv.FormatInt(r.GetFBID(), 10)
			_ = m.st.PutUser(ctx, &User{ID: id, Name: r.DisplayName})
			res := map[string]any{"chat": id, "title": r.DisplayName, "group": false}
			if r.ContextLine != "" {
				res["about"] = r.ContextLine
			}
			if !r.CanViewerMessage {
				res["cannot_message"] = true
			}
			add(res)
		} else {
			key, _ := strconv.ParseInt(r.ResultId, 10, 64)
			if key == 0 {
				continue
			}
			id := "t" + r.ResultId
			if c, _ := m.st.ChatByFBKey(ctx, key); c != nil {
				id = c.ID
			}
			add(map[string]any{"chat": id, "title": r.DisplayName, "group": true})
		}
	}
	return out, nil
}

// Resolve turns an id, a profile link or a name into a person.
func (m *messenger) Resolve(ctx context.Context, p ResolveParams) (map[string]any, error) {
	q := strings.TrimSpace(normChat(p.Query))
	if q == "" {
		return nil, errors.New("query required")
	}
	if c, _ := m.st.GetChat(ctx, q); c != nil {
		return m.chatMap(ctx, c), nil
	}
	if isDigits(q) {
		name := m.st.UserName(ctx, q)
		if name == "" {
			m.fetchName(q)
			name = m.st.UserName(ctx, q)
		}
		return map[string]any{"chat": q, "title": name, "group": false}, nil
	}
	if strings.Contains(q, "facebook.com") || strings.Contains(q, "m.me") || strings.Contains(q, "messenger.com") {
		if !strings.Contains(q, "://") {
			q = "https://" + q
		}
		if u, err := url.Parse(q); err == nil {
			if id := u.Query().Get("id"); isDigits(id) {
				return m.Resolve(ctx, ResolveParams{Query: id})
			}
			parts := strings.Split(strings.Trim(u.Path, "/"), "/")
			if len(parts) > 0 && parts[len(parts)-1] != "" {
				last := parts[len(parts)-1]
				if isDigits(last) {
					return m.Resolve(ctx, ResolveParams{Query: last})
				}
				q = last
			}
		}
	}
	if u, _ := m.st.UserByName(ctx, q); u != nil {
		return map[string]any{"chat": u.ID, "title": orStr(u.Name, u.Username), "group": false}, nil
	}
	hits, err := m.Search(ctx, SearchParams{Query: q, Limit: 5})
	if err != nil {
		return nil, err
	}
	if len(hits) == 0 {
		return nil, fmt.Errorf("nobody found for %q on Messenger", q)
	}
	if len(hits) > 1 {
		var names []string
		for _, h := range hits {
			names = append(names, fmt.Sprintf("%v (%v)", h["title"], h["chat"]))
		}
		if !strings.EqualFold(fmt.Sprint(hits[0]["title"]), q) {
			return nil, fmt.Errorf("several matches for %q: %s", q, strings.Join(names, "; "))
		}
	}
	return hits[0], nil
}

func (m *messenger) Members(ctx context.Context, p ChatParams) (map[string]any, error) {
	t, err := m.target(ctx, p.Chat)
	if err != nil {
		return nil, err
	}
	if !t.group {
		return map[string]any{"members": []map[string]any{{"id": t.key, "name": m.st.UserName(ctx, t.key)}}}, nil
	}
	var members []map[string]any
	title := ""
	if t.chat != nil {
		title = t.chat.Title
	}
	if t.e2ee {
		e2ee, err := m.encrypted(ctx)
		if err != nil {
			return nil, err
		}
		gi, err := e2ee.GetGroupInfo(ctx, t.jid)
		if err != nil {
			return nil, err
		}
		title = orStr(gi.Name, title)
		for _, pc := range gi.Participants {
			id := pc.JID.User
			members = append(members, map[string]any{"id": id, "name": m.st.UserName(ctx, id), "admin": pc.IsAdmin || pc.IsSuperAdmin})
		}
	} else {
		for uid, role := range m.st.KVPrefix(ctx, "member:"+t.key+":") {
			members = append(members, map[string]any{"id": uid, "name": m.st.UserName(ctx, uid), "admin": role == "admin"})
		}
	}
	sort.Slice(members, func(i, j int) bool { return fmt.Sprint(members[i]["name"]) < fmt.Sprint(members[j]["name"]) })
	return map[string]any{"title": title, "members": members}, nil
}

func (m *messenger) History(ctx context.Context, p HistoryParams) ([]map[string]any, error) {
	if _, err := m.client(); errors.Is(err, ErrNotLoggedIn) {
		return nil, err
	}
	t, err := m.target(ctx, p.Chat)
	if err != nil {
		return nil, err
	}
	if t.e2ee && (t.chat == nil || t.chat.FBKey == 0 || t.group) {
		return nil, errors.New("encrypted chats have no history on the server for linked devices; only messages seen while logged in are kept")
	}
	cli, err := m.readyClient(ctx)
	if err != nil {
		return nil, err
	}
	refTS, refID := time.Now().UnixMilli(), ""
	if p.Before != "" {
		if ref, _ := m.st.GetMsg(ctx, p.Before); ref != nil {
			refTS, refID = ref.TS, ref.ID
		}
	} else if latest, _ := m.st.LatestMsgs(ctx, t.key, 1); len(latest) == 1 && !latest[0].E2EE {
		refTS, refID = latest[0].TS+1, ""
	}
	ch := make(chan []*table.WrappedMessage, 1)
	m.collectMu.Lock()
	m.collectors[t.fbKey] = ch
	m.collectMu.Unlock()
	defer func() {
		m.collectMu.Lock()
		if m.collectors[t.fbKey] == ch {
			delete(m.collectors, t.fbKey)
		}
		m.collectMu.Unlock()
	}()
	resp, err := cli.ExecuteTasks(ctx, &socket.FetchMessagesTask{
		ThreadKey: t.fbKey, Direction: 0, ReferenceTimestampMs: refTS, ReferenceMessageId: refID, SyncGroup: 1, Cursor: cli.GetCursor(1),
	})
	if err != nil {
		return nil, err
	}
	var msgs []*table.WrappedMessage
	if resp != nil {
		up, ins := resp.WrapMessages()
		if um := up[t.fbKey]; um != nil {
			msgs = um.Messages
		}
		for _, wm := range ins {
			if wm.ThreadKey == t.fbKey {
				msgs = append(msgs, wm)
			}
		}
	}
	if len(msgs) == 0 {
		select {
		case msgs = <-ch:
		case <-time.After(20 * time.Second):
		case <-ctx.Done():
			return nil, ctx.Err()
		}
	}
	tt := &tableThreads{types: map[int64]table.ThreadType{}, wa: map[int64]int64{}}
	own := m.ownID()
	var out []map[string]any
	for _, wm := range msgs {
		if ev := m.classicMessage(ctx, tt, wm, own); ev != nil {
			ev["chat"] = t.key
			out = append(out, ev)
		}
	}
	sort.Slice(out, func(i, j int) bool { return out[i]["ts"].(int64) > out[j]["ts"].(int64) })
	if p.Limit > 0 && len(out) > p.Limit {
		out = out[:p.Limit]
	}
	return out, nil
}

// --- groups and chat settings ---

// userIDs turns names, @usernames, links and ids into Facebook ids.
func (m *messenger) userIDs(ctx context.Context, members []string) ([]int64, error) {
	var ids []int64
	for _, mem := range members {
		r, err := m.Resolve(ctx, ResolveParams{Query: mem})
		if err != nil {
			return nil, fmt.Errorf("%s: %w", mem, err)
		}
		if g, _ := r["group"].(bool); g {
			return nil, fmt.Errorf("%s is a group, not a person", mem)
		}
		id, _ := strconv.ParseInt(fmt.Sprint(r["chat"]), 10, 64)
		if id == 0 {
			return nil, fmt.Errorf("%s: no Facebook id", mem)
		}
		ids = append(ids, id)
	}
	return ids, nil
}

func (m *messenger) CreateGroup(ctx context.Context, p CreateGroupParams) (map[string]any, error) {
	if len(p.Members) == 0 {
		return nil, errors.New("a group needs at least one other person")
	}
	cli, err := m.readyClient(ctx)
	if err != nil {
		return nil, err
	}
	ids, err := m.userIDs(ctx, p.Members)
	if err != nil {
		return nil, err
	}
	threadID, otid := methods.GenerateEpochID(), methods.GenerateEpochID()
	resp, err := cli.ExecuteTasks(ctx, &socket.CreateGroupTask{
		Participants: ids,
		SendPayload:  socket.CreateGroupPayload{ThreadID: threadID, OTID: strconv.FormatInt(otid, 10), Source: 0, SendType: 8},
	})
	if err != nil {
		return nil, err
	}
	if len(resp.LSIssueNewError) > 0 {
		e := resp.LSIssueNewError[0]
		return nil, fmt.Errorf("Messenger refused the group: %s", orStr(e.ErrorMessage, orStr(e.ErrorTitle, "unknown error")))
	}
	if len(resp.LSReplaceOptimisticThread) == 0 {
		return nil, errors.New("Messenger did not confirm the group")
	}
	real := resp.LSReplaceOptimisticThread[0].ThreadKey2
	chat := "t" + strconv.FormatInt(real, 10)
	_ = m.st.PutChat(ctx, &Chat{ID: chat, Kind: "group", FBKey: real, Title: p.Title, LastTS: time.Now().Unix()})
	if strings.TrimSpace(p.Title) != "" {
		if _, err = cli.ExecuteTasks(ctx, &socket.RenameThreadTask{ThreadKey: real, ThreadName: p.Title, SyncGroup: 1}); err != nil {
			return map[string]any{"chat": chat, "warning": "created, but naming it failed: " + err.Error()}, nil
		}
	}
	return map[string]any{"chat": chat}, nil
}

func (m *messenger) AddMembers(ctx context.Context, p MembersParams) (map[string]any, error) {
	t, err := m.target(ctx, p.Chat)
	if err != nil {
		return nil, err
	}
	if !t.group {
		return nil, errors.New("not a group")
	}
	ids, err := m.userIDs(ctx, p.Members)
	if err != nil {
		return nil, err
	}
	if t.e2ee {
		e2ee, err := m.encrypted(ctx)
		if err != nil {
			return nil, err
		}
		jids := make([]waTypes.JID, len(ids))
		for i, id := range ids {
			jids[i] = waTypes.NewJID(strconv.FormatInt(id, 10), waTypes.MessengerServer)
		}
		res, err := e2ee.UpdateGroupParticipants(ctx, t.jid, jids, whatsmeow.ParticipantChangeAdd)
		if err != nil {
			return nil, err
		}
		var failed []string
		for _, r := range res {
			if r.Error != 0 {
				failed = append(failed, fmt.Sprintf("%s (code %d)", r.JID.User, r.Error))
			}
		}
		return map[string]any{"failed": failed}, nil
	}
	cli, err := m.readyClient(ctx)
	if err != nil {
		return nil, err
	}
	_, err = cli.ExecuteTasks(ctx, &socket.AddParticipantsTask{ThreadKey: t.fbKey, ContactIDs: ids, SyncGroup: 1})
	return map[string]any{}, err
}

func (m *messenger) RemoveMember(ctx context.Context, p MemberParams) error {
	t, err := m.target(ctx, p.Chat)
	if err != nil {
		return err
	}
	if !t.group {
		return errors.New("not a group")
	}
	ids, err := m.userIDs(ctx, []string{p.Member})
	if err != nil {
		return err
	}
	return m.removeFrom(ctx, t, ids[0])
}

func (m *messenger) removeFrom(ctx context.Context, t *target, id int64) error {
	if t.e2ee {
		e2ee, err := m.encrypted(ctx)
		if err != nil {
			return err
		}
		if strconv.FormatInt(id, 10) == m.ownID() {
			return e2ee.LeaveGroup(ctx, t.jid)
		}
		res, err := e2ee.UpdateGroupParticipants(ctx, t.jid, []waTypes.JID{waTypes.NewJID(strconv.FormatInt(id, 10), waTypes.MessengerServer)}, whatsmeow.ParticipantChangeRemove)
		if err == nil && len(res) > 0 && res[0].Error != 0 {
			err = fmt.Errorf("refused (code %d)", res[0].Error)
		}
		return err
	}
	cli, err := m.readyClient(ctx)
	if err != nil {
		return err
	}
	_, err = cli.ExecuteTasks(ctx, &socket.RemoveParticipantTask{ThreadID: t.fbKey, ContactID: id})
	return err
}

func (m *messenger) Leave(ctx context.Context, p ChatParams) error {
	t, err := m.target(ctx, p.Chat)
	if err != nil {
		return err
	}
	if !t.group {
		return errors.New("not a group")
	}
	own, _ := strconv.ParseInt(m.ownID(), 10, 64)
	return m.removeFrom(ctx, t, own)
}

func (m *messenger) Rename(ctx context.Context, p RenameParams) error {
	t, err := m.target(ctx, p.Chat)
	if err != nil {
		return err
	}
	if !t.group {
		return errors.New("only groups have names")
	}
	if t.e2ee {
		e2ee, err := m.encrypted(ctx)
		if err != nil {
			return err
		}
		err = e2ee.SetGroupName(ctx, t.jid, p.Title)
		if err == nil {
			_ = m.st.PutChat(ctx, &Chat{ID: t.key, Kind: "group", E2EE: true, Title: p.Title})
		}
		return err
	}
	cli, err := m.readyClient(ctx)
	if err != nil {
		return err
	}
	_, err = cli.ExecuteTasks(ctx, &socket.RenameThreadTask{ThreadKey: t.fbKey, ThreadName: p.Title, SyncGroup: 1})
	if err == nil {
		_ = m.st.PutChat(ctx, &Chat{ID: t.key, Kind: "group", Title: p.Title, FBKey: t.fbKey})
	}
	return err
}

func (m *messenger) SetGroupPhoto(ctx context.Context, p PhotoParams) error {
	t, err := m.target(ctx, p.Chat)
	if err != nil {
		return err
	}
	if !t.group {
		return errors.New("only groups have a photo")
	}
	data, mimeType, name, err := readFile(FileParam{Path: p.Path})
	if err != nil {
		return err
	}
	if t.e2ee {
		e2ee, err := m.encrypted(ctx)
		if err != nil {
			return err
		}
		if mimeType != "image/jpeg" {
			return errors.New("the group photo of an encrypted group must be a JPEG")
		}
		_, err = e2ee.SetGroupPhoto(ctx, t.jid, data)
		return err
	}
	cli, err := m.readyClient(ctx)
	if err != nil {
		return err
	}
	up, err := cli.GetHTTP().SendMercuryUploadRequest(ctx, t.fbKey, &httpclient.MercuryUploadMedia{Filename: name, MimeType: mimeType, MediaData: data})
	if err != nil {
		return err
	}
	imageID := up.Payload.RealMetadata.GetFbId()
	if imageID == 0 {
		return errors.New("upload returned no id")
	}
	_, err = cli.ExecuteTasks(ctx, &socket.SetThreadImageTask{ThreadKey: t.fbKey, ImageID: imageID, SyncGroup: 1})
	return err
}

func (m *messenger) Block(ctx context.Context, p BlockParams) error {
	cli, err := m.readyClient(ctx)
	if err != nil {
		return err
	}
	ids, err := m.userIDs(ctx, []string{p.User})
	if err != nil {
		return err
	}
	resp, err := cli.ExecuteTasks(ctx, &socket.SetUserBlockStatusTask{BlockeeID: ids[0], RequestID: uuid.NewString(), UserBlockAction: b2i(p.On)})
	if err != nil {
		return err
	}
	for _, r := range resp.LSResetUserBlockStatus {
		if r.ContactID == ids[0] && !r.Success {
			return errors.New("Messenger refused the change")
		}
	}
	return nil
}

func (m *messenger) DeleteChat(ctx context.Context, p ChatParams) error {
	t, err := m.target(ctx, p.Chat)
	if err != nil {
		return err
	}
	cli, err := m.readyClient(ctx)
	if err != nil {
		return err
	}
	key, group := t.fbKey, int64(1)
	if t.e2ee && t.chat != nil && t.chat.FBKey != 0 {
		key, group = t.chat.FBKey, 95
	}
	if key == 0 {
		return errors.New("this chat has no Facebook thread to delete")
	}
	_, err = cli.ExecuteTasks(ctx, &socket.DeleteThreadTask{ThreadKey: key, RemoveType: 0, SyncGroup: group})
	if err == nil {
		_ = m.st.DeleteChat(ctx, t.key)
	}
	return err
}

func (m *messenger) AcceptRequest(ctx context.Context, p ChatParams) error {
	t, err := m.target(ctx, p.Chat)
	if err != nil {
		return err
	}
	cli, err := m.readyClient(ctx)
	if err != nil {
		return err
	}
	if t.fbKey == 0 {
		return errors.New("this chat has no Facebook thread")
	}
	_, err = cli.ExecuteTasks(ctx, &socket.AcceptMessageRequestTask{ThreadKey: t.fbKey, SyncGroup: 95})
	if err == nil {
		_ = m.st.PutChat(ctx, &Chat{ID: t.key, Kind: kindStr(t.group), Folder: "inbox"})
	}
	return err
}

func (m *messenger) Mute(ctx context.Context, p MuteParams) error {
	t, err := m.target(ctx, p.Chat)
	if err != nil {
		return err
	}
	cli, err := m.readyClient(ctx)
	if err != nil {
		return err
	}
	expire := int64(0)
	switch {
	case p.Seconds < 0:
		expire = -1
	case p.Seconds > 0:
		expire = time.Now().Add(time.Duration(p.Seconds) * time.Second).UnixMilli()
	}
	_, err = cli.ExecuteTasks(ctx, &socket.MuteThreadTask{ThreadKey: t.fbKey, MailboxType: 0, MuteExpireTimeMS: expire, SyncGroup: 1})
	return err
}

func (m *messenger) Pin(ctx context.Context, p PinParams) error {
	return errors.New("pinning is not available on Messenger through bia")
}

// The bio mutation of facebook.com's profile page (as in fbchat-v2's changeBio).
const setBioDoc = "ProfileCometSetBioMutation"

func init() {
	graphql.GraphQLDocs[setBioDoc] = graphql.GraphQLDoc{DocID: "6293552847364844", FriendlyName: setBioDoc}
}

// SetProfile changes the Facebook bio (the short intro text on the profile). Facebook
// reviews name changes, and the profile photo goes through the profile page itself.
func (m *messenger) SetProfile(ctx context.Context, p ProfileParams) error {
	if p.Bio == nil {
		return errors.New("only the bio can be changed here; the name and photo are changed at https://www.facebook.com/me in bia's Chrome (chrome MCP)")
	}
	cli, err := m.readyClient(ctx)
	if err != nil {
		return err
	}
	if !cli.GetPlatform().IsViaFacebook() {
		return errors.New("the bio can only be set with a facebook.com login: call messenger_login without variant=messenger")
	}
	bio := strings.TrimSpace(*p.Bio)
	if len([]rune(bio)) > 101 {
		return fmt.Errorf("a Facebook bio is at most 101 characters (this one has %d)", len([]rune(bio)))
	}
	vars := map[string]any{
		"input": map[string]any{
			"bio": bio, "publish_bio_feed_story": false, "actor_id": m.ownID(),
			"client_mutation_id": strconv.Itoa(int(time.Now().UnixNano() % 1024)),
		},
		"hasProfileTileViewID": false, "profileTileViewID": nil, "scale": 1,
	}
	_, body, err := cli.GetHTTP().MakeGraphQLRequest(ctx, setBioDoc, vars)
	if err != nil {
		return fmt.Errorf("set bio: %w", err)
	}
	return checkBio(body, bio)
}

// checkBio reads the mutation's answer: the new bio back, or Facebook's error.
func checkBio(body []byte, bio string) error {
	var resp struct {
		Data struct {
			Set struct {
				Card struct {
					Bio struct {
						Text string `json:"text"`
					} `json:"bio"`
				} `json:"profile_intro_card"`
			} `json:"profile_intro_card_set"`
		} `json:"data"`
		Errors []struct {
			Message string `json:"message"`
		} `json:"errors"`
	}
	// the answer can be several JSON documents, one per line: the first holds the data
	line, _, _ := strings.Cut(string(body), "\n")
	if err := json.Unmarshal([]byte(line), &resp); err != nil {
		return fmt.Errorf("Facebook answered something bia cannot read: %.200s", body)
	}
	if len(resp.Errors) > 0 {
		return fmt.Errorf("Facebook refused the bio: %s (change it at https://www.facebook.com/me in bia's Chrome instead)", resp.Errors[0].Message)
	}
	if resp.Data.Set.Card.Bio.Text != bio {
		return errors.New("Facebook did not confirm the new bio; check it at https://www.facebook.com/me in bia's Chrome")
	}
	return nil
}
