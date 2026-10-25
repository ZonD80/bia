package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"go.mau.fi/mautrix-meta/pkg/messagix/httpclient"

	"github.com/rs/zerolog"
	"go.mau.fi/mautrix-meta/pkg/instameow/slidetypes"
	"go.mau.fi/mautrix-meta/pkg/messagix/table"
)

func testStore(t *testing.T) *Store {
	t.Helper()
	log = zerolog.Nop()
	st, err := openStore(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(st.Close)
	return st
}

func TestChatMergeKeepsKnownFields(t *testing.T) {
	st := testStore(t)
	ctx := context.Background()
	must(t, st.PutChat(ctx, &Chat{ID: "100", Title: "Ann", Kind: "dm", E2EE: true, FBKey: 100, Peer: "100", LastTS: 50}))
	// a later, thinner update must not erase what is known
	must(t, st.PutChat(ctx, &Chat{ID: "100", Kind: "dm", LastTS: 40}))
	c, err := st.GetChat(ctx, "100")
	must(t, err)
	if c.Title != "Ann" || !c.E2EE || c.FBKey != 100 || c.LastTS != 50 {
		t.Fatalf("merge lost data: %+v", c)
	}
	byKey, _ := st.ChatByFBKey(ctx, 100)
	if byKey == nil || byKey.ID != "100" {
		t.Fatalf("by fb key: %+v", byKey)
	}
	must(t, st.PutUser(ctx, &User{ID: "100", Name: "Ann Lee", Username: "annlee"}))
	hits, _ := st.FindChats(ctx, "lee", 5)
	if len(hits) != 1 || hits[0].ID != "100" {
		t.Fatalf("find by peer name: %+v", hits)
	}
	if got := st.UserName(ctx, "100"); got != "Ann Lee" {
		t.Fatalf("user name %q", got)
	}
	u, _ := st.UserByName(ctx, "@AnnLee")
	if u == nil || u.ID != "100" {
		t.Fatalf("by username: %+v", u)
	}
}

func TestRecentPagesNewestFirst(t *testing.T) {
	st := testStore(t)
	ctx := context.Background()
	for i, id := range []string{"a", "b", "c", "d"} {
		must(t, st.PutMsg(ctx, &MsgRef{ID: id, Chat: "t1", Sender: "1", TS: int64(1000 * (i + 1)), Text: id}))
	}
	page, err := st.Recent(ctx, "t1", 2, "")
	must(t, err)
	if len(page) != 2 || page[0].ID != "d" || page[1].ID != "c" {
		t.Fatalf("first page %v", ids(page))
	}
	page, _ = st.Recent(ctx, "t1", 2, "c")
	if len(page) != 2 || page[0].ID != "b" || page[1].ID != "a" {
		t.Fatalf("second page %v", ids(page))
	}
	if _, err = st.Recent(ctx, "t1", 2, "zz"); err == nil {
		t.Fatal("unknown before id must fail")
	}
	st.SetMsgText(ctx, "a", "edited")
	m, _ := st.GetMsg(ctx, "a")
	if m.Text != "edited" {
		t.Fatalf("edit not kept: %+v", m)
	}
}

func TestKVPrefix(t *testing.T) {
	st := testStore(t)
	ctx := context.Background()
	st.PutKV(ctx, "member:t1:5", "admin")
	st.PutKV(ctx, "member:t1:6", "member")
	st.PutKV(ctx, "member:t10:7", "member")
	got := st.KVPrefix(ctx, "member:t1:")
	if len(got) != 2 || got["5"] != "admin" || got["6"] != "member" {
		t.Fatalf("prefix: %v", got)
	}
	st.DelKV(ctx, "member:t1:6")
	if len(st.KVPrefix(ctx, "member:t1:")) != 1 {
		t.Fatal("delete")
	}
	must(t, st.Wipe(ctx))
	if len(st.KVPrefix(ctx, "member:")) != 0 {
		t.Fatal("wipe keeps kv")
	}
}

func TestSessionFileIsPrivate(t *testing.T) {
	st := testStore(t)
	must(t, st.SaveSession(&Session{Platform: "messenger", Cookies: map[string]string{"xs": "1"}, ID: "9"}))
	sess, err := st.LoadSession()
	must(t, err)
	if sess == nil || sess.ID != "9" || sess.Cookies["xs"] != "1" {
		t.Fatalf("session %+v", sess)
	}
	must(t, st.DeleteSession())
	if sess, _ = st.LoadSession(); sess != nil {
		t.Fatal("deleted session still loads")
	}
}

func TestParseCookieText(t *testing.T) {
	c := parseCookieText("Cookie: c_user=100; xs=abc%3D; datr=zz")
	if c["c_user"] != "100" || c["xs"] != "abc%3D" || c["datr"] != "zz" {
		t.Fatalf("header: %v", c)
	}
	c = parseCookieText(`{"sessionid": "s", "ds_user_id": 5}`)
	if c["sessionid"] != "s" || c["ds_user_id"] != "5" {
		t.Fatalf("json: %v", c)
	}
}

func TestMessengerTargets(t *testing.T) {
	st := testStore(t)
	m := &messenger{st: st, rootCtx: context.Background(), state: "logged_out"}
	ctx := context.Background()
	dm, err := m.target(ctx, "messenger:100012345678")
	must(t, err)
	if !dm.e2ee || dm.group || dm.jid.String() != "100012345678@msgr" {
		t.Fatalf("new DM must go encrypted: %+v", dm)
	}
	must(t, st.PutChat(ctx, &Chat{ID: "100012345678", Kind: "dm", FBKey: 100012345678}))
	dm, _ = m.target(ctx, "100012345678")
	if dm.e2ee {
		t.Fatal("a known classic DM stays classic")
	}
	g, _ := m.target(ctx, "t771234")
	if !g.group || g.e2ee || g.fbKey != 771234 {
		t.Fatalf("classic group %+v", g)
	}
	eg, _ := m.target(ctx, "5551@g.us")
	if !eg.group || !eg.e2ee || eg.jid.String() != "5551@g.us" {
		t.Fatalf("encrypted group %+v", eg)
	}
	if _, err = m.target(ctx, "nobody"); err == nil {
		t.Fatal("unknown name must fail")
	}
}

func TestChatOf(t *testing.T) {
	st := testStore(t)
	m := &messenger{st: st}
	ctx := context.Background()
	tt := &tableThreads{
		types: map[int64]table.ThreadType{1: table.ONE_TO_ONE, 2: table.GROUP_THREAD, 3: table.ENCRYPTED_OVER_WA_GROUP, 4: table.ONE_TO_ONE},
		wa:    map[int64]int64{3: 33, 4: 44},
	}
	cases := map[int64][3]any{1: {"1", false, false}, 2: {"t2", true, false}, 3: {"33@g.us", true, true}, 4: {"44", false, true}}
	for key, want := range cases {
		id, group, e2ee := m.chatOf(ctx, tt, key)
		if id != want[0] || group != want[1] || e2ee != want[2] {
			t.Fatalf("thread %d: %s %v %v, want %v", key, id, group, e2ee, want)
		}
	}
}

func TestShareTexts(t *testing.T) {
	x := &table.WrappedXMA{LSInsertXmaAttachment: &table.LSInsertXmaAttachment{TitleText: "A post", SubtitleText: "by Ann"},
		CTA: &table.LSInsertAttachmentCta{ActionUrl: "https://l.facebook.com/l.php?u=https%3A%2F%2Fexample.com%2Fx"}}
	if got := xmaText(x); got != "[shared] A post — by Ann — https://example.com/x" {
		t.Fatalf("xma %q", got)
	}
	loc := locationText("location", 52.5, 13.4, "Cafe", "Main St 1")
	if !strings.HasPrefix(loc, "[location] Cafe, Main St 1 — 52.500000, 13.400000 https://maps.google.com/?q=52.500000,13.400000") {
		t.Fatalf("location %q", loc)
	}
	ig := xmaShare(&slidetypes.MessageContentXMA{XMATextBody: "look", XMA: &slidetypes.XMAContent{TitleText: "Reel", TargetURL: "https://www.instagram.com/reel/abc/"}})
	if ig != "look\n[shared] Reel — https://www.instagram.com/reel/abc/" {
		t.Fatalf("ig share %q", ig)
	}
}

func TestDispatchLoggedOut(t *testing.T) {
	st := testStore(t)
	theStore = st
	b := newMessenger(context.Background(), st)
	res, err := dispatch(context.Background(), b, &request{Method: "status"})
	must(t, err)
	if res.(map[string]any)["state"] != "logged_out" {
		t.Fatalf("status %v", res)
	}
	if _, err = dispatch(context.Background(), b, &request{Method: "chats", Params: json.RawMessage(`{}`)}); err == nil || err != ErrNotLoggedIn {
		t.Fatalf("chats before login: %v", err)
	}
	if _, err = dispatch(context.Background(), b, &request{Method: "nope"}); err == nil {
		t.Fatal("unknown method")
	}
	if _, err = dispatch(context.Background(), b, &request{Method: "download", Params: json.RawMessage(`{"ref":"x","path":"/tmp/x"}`)}); err == nil {
		t.Fatal("unknown ref must fail")
	}
}

func ids(ms []*MsgRef) []string {
	var out []string
	for _, m := range ms {
		out = append(out, m.ID)
	}
	return out
}

func must(t *testing.T, err error) {
	t.Helper()
	if err != nil {
		t.Fatal(err)
	}
}

func TestExplainTellsWhatToDo(t *testing.T) {
	platformName = "messenger"
	if e := explain("send", ErrNotLoggedIn).Error(); !strings.Contains(e, "messenger_login") || !strings.Contains(e, "facebook.com") {
		t.Fatalf("not logged in: %s", e)
	}
	if e := explain("send", context.DeadlineExceeded).Error(); !strings.Contains(e, "messenger_status") {
		t.Fatalf("timeout: %s", e)
	}
	if e := explain("send", fmt.Errorf("wrap: %w", httpclient.ErrTokenInvalidated)).Error(); !strings.Contains(e, "log in again in the browser") {
		t.Fatalf("expired session: %s", e)
	}
	platformName = "instagram"
	if e := explain("chats", ErrNotLoggedIn).Error(); !strings.Contains(e, "instagram_login") || !strings.Contains(e, "instagram.com") {
		t.Fatalf("ig not logged in: %s", e)
	}
	if e := explain("x", errors.New("plain")).Error(); e != "plain" {
		t.Fatalf("other errors pass through: %s", e)
	}
}

func TestCheckBio(t *testing.T) {
	ok := `{"data":{"profile_intro_card_set":{"profile_intro_card":{"bio":{"text":"hi there"}}}}}` + "\n" + `{"extensions":{}}`
	if err := checkBio([]byte(ok), "hi there"); err != nil {
		t.Fatal(err)
	}
	if err := checkBio([]byte(`{"errors":[{"message":"Rate limit"}]}`), "x"); err == nil || !strings.Contains(err.Error(), "Rate limit") {
		t.Fatalf("refusal: %v", err)
	}
	if err := checkBio([]byte(`{"data":{}}`), "x"); err == nil {
		t.Fatal("unconfirmed bio must fail")
	}
}

// An attachment ref downloads from the CDN (mediadl's client was nil: every
// Instagram voice message failed with a nil pointer dereference).
func TestDownloadURLRef(t *testing.T) {
	log = zerolog.Nop()
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Sec-Fetch-Dest") != "audio" {
			t.Errorf("Sec-Fetch-Dest %q", r.Header.Get("Sec-Fetch-Dest"))
		}
		w.Header().Set("Content-Type", "audio/mp4")
		_, _ = w.Write([]byte("voice-bytes"))
	}))
	defer srv.Close()
	ref := urlRef(srv.URL+"/v.mp4", "audio/mp4")
	path := filepath.Join(t.TempDir(), "in", "voice.m4a")
	res, err := media.Download(context.Background(), DownloadParams{Ref: ref, Path: path})
	if err != nil {
		t.Fatal(err)
	}
	if b, _ := os.ReadFile(path); string(b) != "voice-bytes" || res["size"] != len("voice-bytes") {
		t.Fatalf("got %q %v", b, res)
	}
}
