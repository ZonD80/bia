package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"strings"
	"time"

	"github.com/coder/websocket"
)

// bia's Chrome listens for DevTools on this address (chrome MCP, --remote-debugging-port).
func chromeDebugURL() string {
	if u := os.Getenv("BIA_CHROME_DEBUG_URL"); u != "" {
		return strings.TrimRight(u, "/")
	}
	return "http://127.0.0.1:9222"
}

type cdpCookie struct {
	Name   string `json:"name"`
	Value  string `json:"value"`
	Domain string `json:"domain"`
}

// chromeCookies reads the cookies Chrome keeps for these site domains (HttpOnly ones too).
func chromeCookies(ctx context.Context, sites ...string) (map[string]string, error) {
	ctx, cancel := context.WithTimeout(ctx, 20*time.Second)
	defer cancel()
	req, _ := http.NewRequestWithContext(ctx, http.MethodGet, chromeDebugURL()+"/json/version", nil)
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return nil, errors.New("bia's Chrome is not running (or not reachable for DevTools): start it with the chrome MCP (chrome_start), sign in, then call login again")
	}
	body, _ := io.ReadAll(io.LimitReader(resp.Body, 1<<20))
	resp.Body.Close()
	var ver struct {
		WS string `json:"webSocketDebuggerUrl"`
	}
	if err = json.Unmarshal(body, &ver); err != nil || ver.WS == "" {
		return nil, fmt.Errorf("Chrome DevTools answered without a websocket address")
	}
	conn, _, err := websocket.Dial(ctx, ver.WS, nil)
	if err != nil {
		return nil, fmt.Errorf("Chrome DevTools: %w", err)
	}
	defer conn.Close(websocket.StatusNormalClosure, "")
	conn.SetReadLimit(64 << 20)
	if err = conn.Write(ctx, websocket.MessageText, []byte(`{"id":1,"method":"Storage.getCookies"}`)); err != nil {
		return nil, fmt.Errorf("Chrome DevTools: %w", err)
	}
	for {
		_, data, err := conn.Read(ctx)
		if err != nil {
			return nil, fmt.Errorf("Chrome DevTools: %w", err)
		}
		var msg struct {
			ID     int `json:"id"`
			Result struct {
				Cookies []cdpCookie `json:"cookies"`
			} `json:"result"`
			Error *struct {
				Message string `json:"message"`
			} `json:"error"`
		}
		if json.Unmarshal(data, &msg) != nil || msg.ID != 1 {
			continue
		}
		if msg.Error != nil {
			return nil, fmt.Errorf("Chrome DevTools: %s", msg.Error.Message)
		}
		out := map[string]string{}
		for _, c := range msg.Result.Cookies {
			d := strings.TrimPrefix(strings.ToLower(c.Domain), ".")
			for _, site := range sites {
				if d == site || strings.HasSuffix(d, "."+site) {
					out[c.Name] = c.Value
				}
			}
		}
		return out, nil
	}
}

// parseCookieText accepts "a=1; b=2" (a Cookie header) or a JSON object.
func parseCookieText(s string) map[string]string {
	s = strings.TrimSpace(s)
	out := map[string]string{}
	if strings.HasPrefix(s, "{") {
		var m map[string]any
		if json.Unmarshal([]byte(s), &m) == nil {
			for k, v := range m {
				out[k] = fmt.Sprint(v)
			}
		}
		return out
	}
	s = strings.TrimPrefix(s, "Cookie:")
	for _, part := range strings.Split(s, ";") {
		k, v, ok := strings.Cut(strings.TrimSpace(part), "=")
		if ok && k != "" {
			out[strings.TrimSpace(k)] = strings.TrimSpace(v)
		}
	}
	return out
}
