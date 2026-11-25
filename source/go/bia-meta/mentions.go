package main

// Tags in what bia writes to a group: "@Ann", "@Ann Lee" or "@ann_lee" of a member
// becomes a Messenger mention (highlighted, notifies them). Same rules as the Rust MCPs
// (bia_mcp::share::find_tags): the longest member name after an "@", any case, ending at
// a word boundary; an "@" inside a word or one that names nobody stays text.

import (
	"context"
	"sort"
	"strconv"
	"strings"
	"unicode"
	"unicode/utf8"

	"go.mau.fi/mautrix-meta/pkg/messagix/socket"
)

type tagMember struct {
	ID    int64
	Names []string
}

// findTags returns the mentions in text, byte offsets as mautrix-meta sends them.
func findTags(text string, members []tagMember) socket.Mentions {
	type cand struct {
		name string
		id   int64
	}
	var names []cand
	for _, m := range members {
		for _, n := range m.Names {
			n = strings.ToLower(strings.TrimPrefix(strings.TrimSpace(n), "@"))
			if utf8.RuneCountInString(n) >= 2 {
				names = append(names, cand{n, m.ID})
			}
		}
	}
	sort.SliceStable(names, func(i, j int) bool { return len(names[i].name) > len(names[j].name) })
	low := strings.ToLower(text)
	if len(low) != len(text) {
		return nil
	}
	var out socket.Mentions
	for at := 0; at < len(text); {
		off := strings.IndexByte(text[at:], '@')
		if off < 0 {
			break
		}
		start := at + off
		at = start + 1
		if start > 0 {
			r, _ := utf8.DecodeLastRuneInString(text[:start])
			if unicode.IsLetter(r) || unicode.IsDigit(r) {
				continue
			}
		}
		for _, c := range names {
			if !strings.HasPrefix(low[start+1:], c.name) {
				continue
			}
			end := start + 1 + len(c.name)
			if end < len(text) {
				r, _ := utf8.DecodeRuneInString(text[end:])
				if unicode.IsLetter(r) || unicode.IsDigit(r) || r == '_' {
					continue
				}
			}
			out = append(out, socket.Mention{ID: c.id, Offset: start, Length: end - start, Type: socket.MentionTypePerson})
			at = end
			break
		}
	}
	return out
}

// chatTagMembers lists a chat's known members (bia left out) with the names people
// write: full name, first name, username.
func chatTagMembers(ctx context.Context, st *Store, chat, own string) []tagMember {
	var out []tagMember
	for uid := range st.KVPrefix(ctx, "member:"+chat+":") {
		if uid == own {
			continue
		}
		id, err := strconv.ParseInt(uid, 10, 64)
		if err != nil {
			continue
		}
		u, _ := st.GetUser(ctx, uid)
		if u == nil {
			continue
		}
		m := tagMember{ID: id}
		if u.Name != "" {
			m.Names = append(m.Names, u.Name)
			if f := strings.Fields(u.Name); len(f) > 1 {
				m.Names = append(m.Names, f[0])
			}
		}
		if u.Username != "" {
			m.Names = append(m.Names, u.Username)
		}
		out = append(out, m)
	}
	return out
}
