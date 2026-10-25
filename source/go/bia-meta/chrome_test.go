package main

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/coder/websocket"
)

// fakeChrome answers /json/version and Storage.getCookies like Chrome's DevTools.
func fakeChrome(t *testing.T) *httptest.Server {
	t.Helper()
	var srv *httptest.Server
	srv = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/json/version":
			ws := "ws" + strings.TrimPrefix(srv.URL, "http") + "/devtools/browser/x"
			_, _ = w.Write([]byte(`{"webSocketDebuggerUrl":"` + ws + `"}`))
		case "/devtools/browser/x":
			c, err := websocket.Accept(w, r, nil)
			if err != nil {
				return
			}
			defer c.Close(websocket.StatusNormalClosure, "")
			_, _, _ = c.Read(r.Context())
			// an unrelated event first, then the answer
			_ = c.Write(r.Context(), websocket.MessageText, []byte(`{"method":"Target.targetCreated","params":{}}`))
			_ = c.Write(r.Context(), websocket.MessageText, []byte(`{"id":1,"result":{"cookies":[
				{"name":"c_user","value":"100","domain":".facebook.com","httpOnly":false},
				{"name":"xs","value":"secret","domain":".facebook.com","httpOnly":true},
				{"name":"sessionid","value":"ig","domain":".instagram.com"},
				{"name":"xs","value":"other","domain":".notfacebook.com"}]}}`))
		default:
			http.NotFound(w, r)
		}
	}))
	t.Cleanup(srv.Close)
	return srv
}

func TestChromeCookies(t *testing.T) {
	srv := fakeChrome(t)
	t.Setenv("BIA_CHROME_DEBUG_URL", srv.URL)
	got, err := chromeCookies(context.Background(), "facebook.com")
	if err != nil {
		t.Fatal(err)
	}
	if got["c_user"] != "100" || got["xs"] != "secret" || got["sessionid"] != "" {
		t.Fatalf("facebook cookies: %v", got)
	}
	got, _ = chromeCookies(context.Background(), "instagram.com")
	if got["sessionid"] != "ig" || len(got) != 1 {
		t.Fatalf("instagram cookies: %v", got)
	}
}

func TestLoginCookiesFromChrome(t *testing.T) {
	srv := fakeChrome(t)
	t.Setenv("BIA_CHROME_DEBUG_URL", srv.URL)
	platformName = "messenger"
	p := LoginParams{}
	if err := loginCookies(context.Background(), &p); err != nil {
		t.Fatal(err)
	}
	if p.Variant != "facebook" || p.Cookies["xs"] != "secret" {
		t.Fatalf("login params %+v", p)
	}
	platformName = "instagram"
	p = LoginParams{}
	if err := loginCookies(context.Background(), &p); err != nil || p.Cookies["sessionid"] != "ig" {
		t.Fatalf("instagram %+v %v", p, err)
	}
	p = LoginParams{CookieText: "sessionid=pasted; ds_user_id=1"}
	if err := loginCookies(context.Background(), &p); err != nil || p.Cookies["sessionid"] != "pasted" {
		t.Fatalf("pasted %+v %v", p, err)
	}
	t.Setenv("BIA_CHROME_DEBUG_URL", "http://127.0.0.1:1")
	p = LoginParams{}
	if err := loginCookies(context.Background(), &p); err == nil || !strings.Contains(err.Error(), "Chrome is not running") {
		t.Fatalf("no chrome: %v", err)
	}
}
