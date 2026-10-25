// bia-meta: the Messenger and Instagram side of bia's messenger/instagram MCPs.
//
// One process per account, started by the MCP:
//
//	bia-meta --platform messenger|instagram --dir /home/bia/files/<platform>
//
// It speaks JSON lines on stdin/stdout: requests {"id":N,"method":"…","params":{…}}
// are answered with {"id":N,"result":…} or {"id":N,"error":"…"}; everything the
// account receives arrives unasked as {"event":"…",…}. Logs go to stderr.
//
// The protocol work is mautrix-meta's: messagix (Messenger, with whatsmeow for
// end-to-end encrypted chats) and instameow (Instagram DMs).
package main

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"os"
	"os/signal"
	"runtime/debug"
	"strings"
	"sync"
	"syscall"
	"time"

	"github.com/rs/zerolog"
)

var version = "dev"

type request struct {
	ID     int64           `json:"id"`
	Method string          `json:"method"`
	Params json.RawMessage `json:"params"`
}

// out serializes everything written to stdout (responses and events interleave).
type out struct {
	mu  sync.Mutex
	enc *json.Encoder
}

func (o *out) write(v any) {
	o.mu.Lock()
	defer o.mu.Unlock()
	_ = o.enc.Encode(v)
}

var stdout = &out{enc: json.NewEncoder(os.Stdout)}

// emit sends an unsolicited event: {"event": kind, ...fields}.
func emit(kind string, fields map[string]any) {
	m := make(map[string]any, len(fields)+1)
	for k, v := range fields {
		m[k] = v
	}
	m["event"] = kind
	stdout.write(m)
}

// Backend is one platform's account.
type Backend interface {
	// Login checks the cookies, saves the session and connects.
	Login(ctx context.Context, p LoginParams) (map[string]any, error)
	// Restore connects the saved session (no-op when connected).
	Restore(ctx context.Context) (map[string]any, error)
	Status() map[string]any
	Logout(ctx context.Context) error
	Close()

	Chats(ctx context.Context, p ChatsParams) ([]map[string]any, error)
	Search(ctx context.Context, p SearchParams) ([]map[string]any, error)
	Resolve(ctx context.Context, p ResolveParams) (map[string]any, error)
	Members(ctx context.Context, p ChatParams) (map[string]any, error)
	History(ctx context.Context, p HistoryParams) ([]map[string]any, error)

	Send(ctx context.Context, p SendParams) (map[string]any, error)
	Edit(ctx context.Context, p EditParams) error
	React(ctx context.Context, p ReactParams) error
	Delete(ctx context.Context, p DeleteParams) (map[string]any, error)
	Typing(ctx context.Context, p TypingParams) error
	MarkRead(ctx context.Context, p ReadParams) error

	CreateGroup(ctx context.Context, p CreateGroupParams) (map[string]any, error)
	AddMembers(ctx context.Context, p MembersParams) (map[string]any, error)
	RemoveMember(ctx context.Context, p MemberParams) error
	Leave(ctx context.Context, p ChatParams) error
	Rename(ctx context.Context, p RenameParams) error
	SetGroupPhoto(ctx context.Context, p PhotoParams) error
	Block(ctx context.Context, p BlockParams) error
	DeleteChat(ctx context.Context, p ChatParams) error
	AcceptRequest(ctx context.Context, p ChatParams) error
	Mute(ctx context.Context, p MuteParams) error
	Pin(ctx context.Context, p PinParams) error
	SetProfile(ctx context.Context, p ProfileParams) error

	GroupInfo(ctx context.Context, p ChatParams) (map[string]any, error)
	SetAdmin(ctx context.Context, p AdminParams) error
	GroupSettings(ctx context.Context, p GroupSettingsParams) ([]string, error)
	JoinRequests(ctx context.Context, p ChatParams) ([]map[string]any, error)
	AnswerJoin(ctx context.Context, p AnswerJoinParams) error

	// SessionGone: Meta refused the saved session during a call (expired, logged out elsewhere).
	SessionGone(reason string)
}

type AdminParams struct {
	Chat   string `json:"chat"`
	Member string `json:"member"`
	Admin  bool   `json:"admin"`
}

// GroupSettingsParams: only the fields that are set change.
type GroupSettingsParams struct {
	Chat               string  `json:"chat"`
	OnlyAdminsSend     *bool   `json:"only_admins_send"`
	OnlyAdminsEditInfo *bool   `json:"only_admins_edit_info"`
	OnlyAdminsAdd      *bool   `json:"only_admins_add"`
	JoinApproval       *bool   `json:"join_approval"`
	DisappearingSecs   *int    `json:"disappearing_seconds"`
	Description        *string `json:"description"`
}

type AnswerJoinParams struct {
	Chat    string `json:"chat"`
	Member  string `json:"member"`
	Approve bool   `json:"approve"`
}

type ProfileParams struct {
	Bio *string `json:"bio"`
}

type LoginParams struct {
	Cookies map[string]string `json:"cookies"`
	// a Cookie header ("a=1; b=2") or a JSON object pasted by a person
	CookieText string `json:"cookie_text"`
	// Messenger: "facebook" (facebook.com cookies, default) or "messenger" (messenger.com).
	Variant string `json:"variant"`
}

type ChatParams struct {
	Chat string `json:"chat"`
}

type ChatsParams struct {
	Limit int `json:"limit"`
	// true asks the server for the inbox instead of reading what is known locally.
	Refresh bool `json:"refresh"`
}

type SearchParams struct {
	Query string `json:"query"`
	Limit int    `json:"limit"`
}

type ResolveParams struct {
	Query string `json:"query"`
}

type HistoryParams struct {
	Chat   string `json:"chat"`
	Limit  int    `json:"limit"`
	Before string `json:"before"`
}

type FileParam struct {
	Path string `json:"path"`
	Mime string `json:"mime"`
	Name string `json:"name"`
	// "voice" sends an audio file as a voice message.
	Kind    string  `json:"kind"`
	Seconds float64 `json:"seconds"`
}

type SendParams struct {
	Chat    string      `json:"chat"`
	Text    string      `json:"text"`
	Files   []FileParam `json:"files"`
	ReplyTo string      `json:"reply_to"`
}

type EditParams struct {
	Chat string `json:"chat"`
	ID   string `json:"id"`
	Text string `json:"text"`
}

type ReactParams struct {
	Chat  string `json:"chat"`
	ID    string `json:"id"`
	Emoji string `json:"emoji"`
}

type DeleteParams struct {
	Chat     string   `json:"chat"`
	IDs      []string `json:"ids"`
	Everyone bool     `json:"everyone"`
}

type TypingParams struct {
	Chat string `json:"chat"`
	On   bool   `json:"on"`
}

type ReadParams struct {
	Chat string `json:"chat"`
	ID   string `json:"id"`
}

type CreateGroupParams struct {
	Title   string   `json:"title"`
	Members []string `json:"members"`
}

type MembersParams struct {
	Chat    string   `json:"chat"`
	Members []string `json:"members"`
}

type MemberParams struct {
	Chat   string `json:"chat"`
	Member string `json:"member"`
}

type RenameParams struct {
	Chat  string `json:"chat"`
	Title string `json:"title"`
}

type PhotoParams struct {
	Chat string `json:"chat"`
	Path string `json:"path"`
}

type BlockParams struct {
	User string `json:"user"`
	On   bool   `json:"on"`
}

type MuteParams struct {
	Chat string `json:"chat"`
	// seconds muted; -1 forever, 0 unmutes
	Seconds int `json:"seconds"`
}

type PinParams struct {
	Chat string `json:"chat"`
	// a message id pins that message; empty pins the chat
	ID string `json:"id"`
	On bool   `json:"on"`
}

type RecentParams struct {
	Chat   string `json:"chat"`
	Limit  int    `json:"limit"`
	Before string `json:"before"`
}

type DownloadParams struct {
	Ref  string `json:"ref"`
	Path string `json:"path"`
}

func call[P any](ctx context.Context, raw json.RawMessage, fn func(context.Context, P) (any, error)) (any, error) {
	var p P
	if len(raw) > 0 && string(raw) != "null" {
		if err := json.Unmarshal(raw, &p); err != nil {
			return nil, fmt.Errorf("bad params: %w", err)
		}
	}
	return fn(ctx, p)
}

func ok(err error) (any, error) {
	if err != nil {
		return nil, err
	}
	return map[string]any{}, nil
}

func dispatch(ctx context.Context, b Backend, req *request) (any, error) {
	switch req.Method {
	case "hello":
		return map[string]any{"version": version, "protocol": 1}, nil
	case "login":
		return call(ctx, req.Params, func(ctx context.Context, p LoginParams) (any, error) {
			if err := loginCookies(ctx, &p); err != nil {
				return nil, err
			}
			return b.Login(ctx, p)
		})
	case "recent":
		return call(ctx, req.Params, func(ctx context.Context, p RecentParams) (any, error) { return recent(ctx, p) })
	case "restore":
		return b.Restore(ctx)
	case "status":
		return b.Status(), nil
	case "logout":
		return ok(b.Logout(ctx))
	case "chats":
		return call(ctx, req.Params, func(ctx context.Context, p ChatsParams) (any, error) { return b.Chats(ctx, p) })
	case "search":
		return call(ctx, req.Params, func(ctx context.Context, p SearchParams) (any, error) { return b.Search(ctx, p) })
	case "resolve":
		return call(ctx, req.Params, func(ctx context.Context, p ResolveParams) (any, error) { return b.Resolve(ctx, p) })
	case "members":
		return call(ctx, req.Params, func(ctx context.Context, p ChatParams) (any, error) { return b.Members(ctx, p) })
	case "history":
		return call(ctx, req.Params, func(ctx context.Context, p HistoryParams) (any, error) { return b.History(ctx, p) })
	case "send":
		return call(ctx, req.Params, func(ctx context.Context, p SendParams) (any, error) { return b.Send(ctx, p) })
	case "edit":
		return call(ctx, req.Params, func(ctx context.Context, p EditParams) (any, error) { return ok(b.Edit(ctx, p)) })
	case "react":
		return call(ctx, req.Params, func(ctx context.Context, p ReactParams) (any, error) { return ok(b.React(ctx, p)) })
	case "delete":
		return call(ctx, req.Params, func(ctx context.Context, p DeleteParams) (any, error) { return b.Delete(ctx, p) })
	case "typing":
		return call(ctx, req.Params, func(ctx context.Context, p TypingParams) (any, error) { return ok(b.Typing(ctx, p)) })
	case "read":
		return call(ctx, req.Params, func(ctx context.Context, p ReadParams) (any, error) { return ok(b.MarkRead(ctx, p)) })
	case "download":
		return call(ctx, req.Params, func(ctx context.Context, p DownloadParams) (any, error) { return media.Download(ctx, p) })
	case "create_group":
		return call(ctx, req.Params, func(ctx context.Context, p CreateGroupParams) (any, error) { return b.CreateGroup(ctx, p) })
	case "add_members":
		return call(ctx, req.Params, func(ctx context.Context, p MembersParams) (any, error) { return b.AddMembers(ctx, p) })
	case "remove_member":
		return call(ctx, req.Params, func(ctx context.Context, p MemberParams) (any, error) { return ok(b.RemoveMember(ctx, p)) })
	case "leave":
		return call(ctx, req.Params, func(ctx context.Context, p ChatParams) (any, error) { return ok(b.Leave(ctx, p)) })
	case "rename":
		return call(ctx, req.Params, func(ctx context.Context, p RenameParams) (any, error) { return ok(b.Rename(ctx, p)) })
	case "group_photo":
		return call(ctx, req.Params, func(ctx context.Context, p PhotoParams) (any, error) { return ok(b.SetGroupPhoto(ctx, p)) })
	case "block":
		return call(ctx, req.Params, func(ctx context.Context, p BlockParams) (any, error) { return ok(b.Block(ctx, p)) })
	case "delete_chat":
		return call(ctx, req.Params, func(ctx context.Context, p ChatParams) (any, error) { return ok(b.DeleteChat(ctx, p)) })
	case "accept":
		return call(ctx, req.Params, func(ctx context.Context, p ChatParams) (any, error) { return ok(b.AcceptRequest(ctx, p)) })
	case "mute":
		return call(ctx, req.Params, func(ctx context.Context, p MuteParams) (any, error) { return ok(b.Mute(ctx, p)) })
	case "pin":
		return call(ctx, req.Params, func(ctx context.Context, p PinParams) (any, error) { return ok(b.Pin(ctx, p)) })
	case "group_info":
		return call(ctx, req.Params, func(ctx context.Context, p ChatParams) (any, error) { return b.GroupInfo(ctx, p) })
	case "set_admin":
		return call(ctx, req.Params, func(ctx context.Context, p AdminParams) (any, error) { return ok(b.SetAdmin(ctx, p)) })
	case "group_settings":
		return call(ctx, req.Params, func(ctx context.Context, p GroupSettingsParams) (any, error) {
			done, err := b.GroupSettings(ctx, p)
			if err != nil {
				return nil, err
			}
			return map[string]any{"done": done}, nil
		})
	case "join_requests":
		return call(ctx, req.Params, func(ctx context.Context, p ChatParams) (any, error) { return b.JoinRequests(ctx, p) })
	case "answer_join":
		return call(ctx, req.Params, func(ctx context.Context, p AnswerJoinParams) (any, error) { return ok(b.AnswerJoin(ctx, p)) })
	case "set_profile":
		return call(ctx, req.Params, func(ctx context.Context, p ProfileParams) (any, error) { return ok(b.SetProfile(ctx, p)) })
	default:
		return nil, fmt.Errorf("unknown method %q", req.Method)
	}
}

// ErrNotLoggedIn is what every account call returns before login.
var ErrNotLoggedIn = errors.New("not logged in")

func siteTitle() (title, site string) {
	if platformName == "instagram" {
		return "Instagram", "https://www.instagram.com/accounts/login/"
	}
	return "Messenger", "https://www.facebook.com/"
}

// explain turns errors into what the model should do next: every error a tool returns
// passes through here.
func explain(method string, err error) error {
	title, site := siteTitle()
	switch {
	case errors.Is(err, ErrNotLoggedIn):
		return fmt.Errorf("not logged in to %s: sign in at %s in bia's Chrome (chrome MCP), then call %s_login", title, site, platformName)
	case errors.Is(err, context.DeadlineExceeded):
		return fmt.Errorf("%s did not answer in time (%s); try again, and check %s_status if it keeps happening", title, method, platformName)
	case errors.Is(err, context.Canceled):
		return fmt.Errorf("%s: cancelled", method)
	}
	var e error
	if platformName == "instagram" {
		e = igLoginError(err)
	} else {
		e = loginError(err)
	}
	if e != err {
		return e
	}
	return err
}

func handle(ctx context.Context, b Backend, req *request) {
	defer func() {
		if v := recover(); v != nil {
			log.Error().Interface("panic", v).Bytes("stack", debug.Stack()).Str("method", req.Method).Msg("panic in request")
			stdout.write(map[string]any{"id": req.ID, "error": fmt.Sprintf("internal error: %v", v)})
		}
	}()
	// a send of big files may take minutes; nothing else should hang for long
	timeout := 2 * time.Minute
	switch req.Method {
	case "send", "download", "login", "restore":
		timeout = 15 * time.Minute
	}
	ctx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()
	res, err := dispatch(ctx, b, req)
	if err != nil {
		if isLoggedOut(err) && req.Method != "login" {
			title, _ := siteTitle()
			b.SessionGone(title + " ended the session (logged out elsewhere or it expired)")
		}
		stdout.write(map[string]any{"id": req.ID, "error": explain(req.Method, err).Error()})
		return
	}
	if res == nil {
		res = map[string]any{}
	}
	stdout.write(map[string]any{"id": req.ID, "result": res})
}

var log zerolog.Logger

var (
	platformName string
	theStore     *Store
)

// loginCookies fills the cookies from pasted text, else from bia's Chrome.
func loginCookies(ctx context.Context, p *LoginParams) error {
	if len(p.Cookies) > 0 {
		return nil
	}
	if strings.TrimSpace(p.CookieText) != "" {
		p.Cookies = parseCookieText(p.CookieText)
		return nil
	}
	if platformName == "instagram" {
		c, err := chromeCookies(ctx, "instagram.com")
		if err != nil {
			return err
		}
		if c["sessionid"] == "" {
			return errors.New("Chrome is not signed in to Instagram: open https://www.instagram.com/accounts/login/ in Chrome, sign in, then call login again")
		}
		p.Cookies = c
		return nil
	}
	if p.Variant != "messenger" {
		c, err := chromeCookies(ctx, "facebook.com")
		if err != nil {
			return err
		}
		if c["xs"] != "" && c["c_user"] != "" {
			p.Cookies, p.Variant = c, "facebook"
			return nil
		}
	}
	c, err := chromeCookies(ctx, "messenger.com")
	if err != nil {
		return err
	}
	if c["xs"] == "" || c["c_user"] == "" {
		return errors.New("Chrome is not signed in to Facebook: open https://www.facebook.com/ in Chrome, sign in, then call login again")
	}
	p.Cookies, p.Variant = c, "messenger"
	return nil
}

func recent(ctx context.Context, p RecentParams) (any, error) {
	chat := strings.TrimSpace(p.Chat)
	for _, pre := range []string{"messenger:", "instagram:"} {
		chat = strings.TrimPrefix(chat, pre)
	}
	limit := p.Limit
	if limit <= 0 || limit > 200 {
		limit = 30
	}
	msgs, err := theStore.Recent(ctx, chat, limit, p.Before)
	if err != nil {
		return nil, err
	}
	out := make([]map[string]any, 0, len(msgs))
	for _, m := range msgs {
		out = append(out, map[string]any{"id": m.ID, "ts": m.TS / 1000, "sender": m.Sender,
			"sender_name": theStore.UserName(ctx, m.Sender), "me": m.Me, "text": m.Text})
	}
	return out, nil
}

func main() {
	platform := flag.String("platform", "messenger", "messenger or instagram")
	dir := flag.String("dir", "", "state directory (session, keys, chat index)")
	logLevel := flag.String("log", "warn", "trace, debug, info, warn or error")
	showVersion := flag.Bool("version", false, "print the version")
	flag.Parse()
	if *showVersion {
		fmt.Println(version)
		return
	}
	level, err := zerolog.ParseLevel(*logLevel)
	if err != nil {
		level = zerolog.WarnLevel
	}
	zerolog.SetGlobalLevel(level)
	log = zerolog.New(zerolog.ConsoleWriter{Out: os.Stderr, NoColor: true, TimeFormat: time.RFC3339}).
		With().Timestamp().Str("platform", *platform).Logger()
	if *dir == "" {
		fmt.Fprintln(os.Stderr, "--dir is required")
		os.Exit(2)
	}
	if err := os.MkdirAll(*dir, 0o700); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	st, err := openStore(*dir)
	if err != nil {
		fmt.Fprintln(os.Stderr, "open store:", err)
		os.Exit(1)
	}
	platformName, theStore = *platform, st
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	var b Backend
	switch *platform {
	case "messenger":
		b = newMessenger(ctx, st)
	case "instagram":
		b = newInstagram(ctx, st)
	default:
		fmt.Fprintln(os.Stderr, "unknown platform", *platform)
		os.Exit(2)
	}
	sig := make(chan os.Signal, 1)
	signal.Notify(sig, syscall.SIGTERM, syscall.SIGINT)
	go func() {
		<-sig
		b.Close()
		os.Exit(0)
	}()
	in := bufio.NewScanner(os.Stdin)
	in.Buffer(make([]byte, 1<<20), 64<<20)
	for in.Scan() {
		line := in.Bytes()
		if len(line) == 0 {
			continue
		}
		var req request
		if err := json.Unmarshal(line, &req); err != nil {
			stdout.write(map[string]any{"id": nil, "error": "parse error: " + err.Error()})
			continue
		}
		go handle(ctx, b, &req)
	}
	// stdin closed: the MCP is gone
	b.Close()
	st.Close()
}
