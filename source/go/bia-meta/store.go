package main

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	_ "modernc.org/sqlite"
)

// Store is the account's state directory: session.json (cookies; 0600) and meta.db
// (whatsmeow's encryption keys, plus what bia knows about chats, people and messages).
type Store struct {
	dir string
	db  *sql.DB
	mu  sync.Mutex
}

// Session is what a login leaves behind; cookies are the account.
type Session struct {
	Platform string            `json:"platform"`
	Variant  string            `json:"variant,omitempty"`
	Cookies  map[string]string `json:"cookies"`
	ID       string            `json:"id"`
	Name     string            `json:"name,omitempty"`
	Username string            `json:"username,omitempty"`
	Since    int64             `json:"since"`
	// Instagram's own user id (the account's FBID is ID)
	IGID string `json:"igid,omitempty"`
}

// Chat is one conversation; ID is the chat id the MCP uses.
type Chat struct {
	ID    string
	Title string
	// "dm" or "group"
	Kind string
	E2EE bool
	// Messenger: the Facebook thread key; Instagram: the thread key (FBID)
	FBKey int64
	// Messenger: the WhatsApp JID of an encrypted chat; Instagram: the short thread id
	WAJID string
	// Instagram: the long thread id (unsend, mark read)
	IGThread string
	// the other person in a DM
	Peer    string
	LastTS  int64
	Folder  string
	Request bool
}

// User is a person the account has seen.
type User struct {
	ID       string
	Name     string
	Username string
	IGID     string
}

// MsgRef is enough of a message to edit, react to, delete or quote it later.
type MsgRef struct {
	ID     string
	Chat   string
	Sender string
	Me     bool
	E2EE   bool
	TS     int64
	Text   string
}

const schema = `
CREATE TABLE IF NOT EXISTS bia_chats (
	id TEXT PRIMARY KEY,
	title TEXT NOT NULL DEFAULT '',
	kind TEXT NOT NULL DEFAULT 'dm',
	e2ee INTEGER NOT NULL DEFAULT 0,
	fb_key INTEGER NOT NULL DEFAULT 0,
	wa_jid TEXT NOT NULL DEFAULT '',
	ig_thread TEXT NOT NULL DEFAULT '',
	peer TEXT NOT NULL DEFAULT '',
	last_ts INTEGER NOT NULL DEFAULT 0,
	folder TEXT NOT NULL DEFAULT '',
	request INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS bia_chats_fb_key ON bia_chats(fb_key);
CREATE TABLE IF NOT EXISTS bia_users (
	id TEXT PRIMARY KEY,
	name TEXT NOT NULL DEFAULT '',
	username TEXT NOT NULL DEFAULT '',
	ig_id TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS bia_users_ig_id ON bia_users(ig_id);
CREATE TABLE IF NOT EXISTS bia_msgs (
	id TEXT PRIMARY KEY,
	chat TEXT NOT NULL,
	sender TEXT NOT NULL DEFAULT '',
	me INTEGER NOT NULL DEFAULT 0,
	e2ee INTEGER NOT NULL DEFAULT 0,
	ts INTEGER NOT NULL DEFAULT 0,
	text TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS bia_msgs_chat_ts ON bia_msgs(chat, ts);
CREATE TABLE IF NOT EXISTS bia_kv (
	key TEXT PRIMARY KEY,
	value TEXT NOT NULL DEFAULT ''
);
`

// Messages kept for edits, reactions and replies (oldest go first).
const msgCap = 50000

func openStore(dir string) (*Store, error) {
	path := filepath.Join(dir, "meta.db")
	dsn := "file:" + path + "?_pragma=foreign_keys(1)&_pragma=journal_mode(WAL)&_pragma=busy_timeout(15000)&_pragma=synchronous(NORMAL)"
	db, err := sql.Open("sqlite", dsn)
	if err != nil {
		return nil, err
	}
	db.SetMaxOpenConns(4)
	if _, err = db.Exec(schema); err != nil {
		db.Close()
		return nil, fmt.Errorf("schema: %w", err)
	}
	_ = os.Chmod(path, 0o600)
	return &Store{dir: dir, db: db}, nil
}

func (s *Store) Close() {
	if s.db != nil {
		_ = s.db.Close()
	}
}

func (s *Store) sessionPath() string {
	return filepath.Join(s.dir, "session.json")
}

func (s *Store) LoadSession() (*Session, error) {
	b, err := os.ReadFile(s.sessionPath())
	if errors.Is(err, os.ErrNotExist) {
		return nil, nil
	} else if err != nil {
		return nil, err
	}
	var sess Session
	if err = json.Unmarshal(b, &sess); err != nil {
		return nil, fmt.Errorf("session.json: %w", err)
	}
	if len(sess.Cookies) == 0 {
		return nil, nil
	}
	return &sess, nil
}

func (s *Store) SaveSession(sess *Session) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	b, err := json.MarshalIndent(sess, "", "  ")
	if err != nil {
		return err
	}
	tmp := s.sessionPath() + ".tmp"
	if err = os.WriteFile(tmp, b, 0o600); err != nil {
		return err
	}
	return os.Rename(tmp, s.sessionPath())
}

func (s *Store) DeleteSession() error {
	err := os.Remove(s.sessionPath())
	if errors.Is(err, os.ErrNotExist) {
		return nil
	}
	return err
}

// Wipe forgets everything about the account (logout).
func (s *Store) Wipe(ctx context.Context) error {
	for _, t := range []string{"bia_chats", "bia_users", "bia_msgs", "bia_kv"} {
		if _, err := s.db.ExecContext(ctx, "DELETE FROM "+t); err != nil {
			return err
		}
	}
	return s.DeleteSession()
}

// PutChat merges what is known: empty fields keep the stored value.
func (s *Store) PutChat(ctx context.Context, c *Chat) error {
	_, err := s.db.ExecContext(ctx, `
INSERT INTO bia_chats (id, title, kind, e2ee, fb_key, wa_jid, ig_thread, peer, last_ts, folder, request)
VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
ON CONFLICT(id) DO UPDATE SET
	title = CASE WHEN excluded.title <> '' THEN excluded.title ELSE bia_chats.title END,
	kind = excluded.kind,
	e2ee = MAX(bia_chats.e2ee, excluded.e2ee),
	fb_key = CASE WHEN excluded.fb_key <> 0 THEN excluded.fb_key ELSE bia_chats.fb_key END,
	wa_jid = CASE WHEN excluded.wa_jid <> '' THEN excluded.wa_jid ELSE bia_chats.wa_jid END,
	ig_thread = CASE WHEN excluded.ig_thread <> '' THEN excluded.ig_thread ELSE bia_chats.ig_thread END,
	peer = CASE WHEN excluded.peer <> '' THEN excluded.peer ELSE bia_chats.peer END,
	last_ts = MAX(bia_chats.last_ts, excluded.last_ts),
	folder = CASE WHEN excluded.folder <> '' THEN excluded.folder ELSE bia_chats.folder END,
	request = CASE WHEN excluded.folder <> '' THEN excluded.request ELSE bia_chats.request END`,
		c.ID, c.Title, orStr(c.Kind, "dm"), b2i(c.E2EE), c.FBKey, c.WAJID, c.IGThread, c.Peer, c.LastTS, c.Folder, b2i(c.Request))
	return err
}

func (s *Store) DeleteChat(ctx context.Context, id string) error {
	_, err := s.db.ExecContext(ctx, "DELETE FROM bia_chats WHERE id = ?", id)
	return err
}

func scanChat(row interface{ Scan(...any) error }) (*Chat, error) {
	var c Chat
	var e2ee, req int
	err := row.Scan(&c.ID, &c.Title, &c.Kind, &e2ee, &c.FBKey, &c.WAJID, &c.IGThread, &c.Peer, &c.LastTS, &c.Folder, &req)
	if err != nil {
		return nil, err
	}
	c.E2EE, c.Request = e2ee != 0, req != 0
	return &c, nil
}

const chatCols = "id, title, kind, e2ee, fb_key, wa_jid, ig_thread, peer, last_ts, folder, request"

func (s *Store) GetChat(ctx context.Context, id string) (*Chat, error) {
	c, err := scanChat(s.db.QueryRowContext(ctx, "SELECT "+chatCols+" FROM bia_chats WHERE id = ?", id))
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	return c, err
}

// ChatByFBKey finds a chat by its Facebook thread key (Messenger) or thread key (Instagram).
func (s *Store) ChatByFBKey(ctx context.Context, key int64) (*Chat, error) {
	c, err := scanChat(s.db.QueryRowContext(ctx, "SELECT "+chatCols+" FROM bia_chats WHERE fb_key = ? ORDER BY e2ee DESC LIMIT 1", key))
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	return c, err
}

// ChatByWAJID finds a chat by its WAJID column (Instagram: the short thread id).
func (s *Store) ChatByWAJID(ctx context.Context, jid string) (*Chat, error) {
	c, err := scanChat(s.db.QueryRowContext(ctx, "SELECT "+chatCols+" FROM bia_chats WHERE wa_jid = ? LIMIT 1", jid))
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	return c, err
}

func (s *Store) Chats(ctx context.Context, limit int) ([]*Chat, error) {
	if limit <= 0 {
		limit = 50
	}
	rows, err := s.db.QueryContext(ctx, "SELECT "+chatCols+" FROM bia_chats ORDER BY last_ts DESC, id LIMIT ?", limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []*Chat
	for rows.Next() {
		c, err := scanChat(rows)
		if err != nil {
			return nil, err
		}
		out = append(out, c)
	}
	return out, rows.Err()
}

// FindChats matches titles and peer names (case-insensitive), most recent first.
func (s *Store) FindChats(ctx context.Context, q string, limit int) ([]*Chat, error) {
	like := "%" + strings.ToLower(strings.TrimSpace(q)) + "%"
	rows, err := s.db.QueryContext(ctx, `
SELECT c.id, c.title, c.kind, c.e2ee, c.fb_key, c.wa_jid, c.ig_thread, c.peer, c.last_ts, c.folder, c.request
FROM bia_chats c LEFT JOIN bia_users u ON u.id = c.peer
WHERE lower(c.title) LIKE ? OR lower(u.name) LIKE ? OR lower(u.username) LIKE ? OR c.id = ?
ORDER BY c.last_ts DESC LIMIT ?`, like, like, like, strings.TrimSpace(q), limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []*Chat
	for rows.Next() {
		c, err := scanChat(rows)
		if err != nil {
			return nil, err
		}
		out = append(out, c)
	}
	return out, rows.Err()
}

func (s *Store) PutUser(ctx context.Context, u *User) error {
	if u.ID == "" || u.ID == "0" {
		return nil
	}
	_, err := s.db.ExecContext(ctx, `
INSERT INTO bia_users (id, name, username, ig_id) VALUES (?, ?, ?, ?)
ON CONFLICT(id) DO UPDATE SET
	name = CASE WHEN excluded.name <> '' THEN excluded.name ELSE bia_users.name END,
	username = CASE WHEN excluded.username <> '' THEN excluded.username ELSE bia_users.username END,
	ig_id = CASE WHEN excluded.ig_id <> '' THEN excluded.ig_id ELSE bia_users.ig_id END`,
		u.ID, u.Name, u.Username, u.IGID)
	return err
}

func (s *Store) GetUser(ctx context.Context, id string) (*User, error) {
	var u User
	err := s.db.QueryRowContext(ctx, "SELECT id, name, username, ig_id FROM bia_users WHERE id = ?", id).
		Scan(&u.ID, &u.Name, &u.Username, &u.IGID)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	return &u, err
}

// UserByName finds a user by username (without @) or exact name.
func (s *Store) UserByName(ctx context.Context, q string) (*User, error) {
	q = strings.ToLower(strings.TrimPrefix(strings.TrimSpace(q), "@"))
	var u User
	err := s.db.QueryRowContext(ctx, "SELECT id, name, username, ig_id FROM bia_users WHERE lower(username) = ? OR lower(name) = ? LIMIT 1", q, q).
		Scan(&u.ID, &u.Name, &u.Username, &u.IGID)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	return &u, err
}

func (s *Store) UserByIGID(ctx context.Context, igid string) (*User, error) {
	var u User
	err := s.db.QueryRowContext(ctx, "SELECT id, name, username, ig_id FROM bia_users WHERE ig_id = ? LIMIT 1", igid).
		Scan(&u.ID, &u.Name, &u.Username, &u.IGID)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	return &u, err
}

// UserName is the best label for a person: name, else @username, else the id.
func (s *Store) UserName(ctx context.Context, id string) string {
	u, _ := s.GetUser(ctx, id)
	if u != nil {
		if u.Name != "" {
			return u.Name
		}
		if u.Username != "" {
			return "@" + u.Username
		}
	}
	return ""
}

func (s *Store) PutMsg(ctx context.Context, m *MsgRef) error {
	_, err := s.db.ExecContext(ctx, `
INSERT INTO bia_msgs (id, chat, sender, me, e2ee, ts, text) VALUES (?, ?, ?, ?, ?, ?, ?)
ON CONFLICT(id) DO UPDATE SET
	text = CASE WHEN excluded.text <> '' THEN excluded.text ELSE bia_msgs.text END,
	ts = CASE WHEN bia_msgs.ts = 0 THEN excluded.ts ELSE bia_msgs.ts END`,
		m.ID, m.Chat, m.Sender, b2i(m.Me), b2i(m.E2EE), m.TS, m.Text)
	if err != nil {
		// callers go on (the message itself was sent or received); history misses it
		log.Warn().Err(err).Str("id", m.ID).Str("chat", m.Chat).Msg("message not kept for history")
	}
	return err
}

func (s *Store) SetMsgText(ctx context.Context, id, text string) {
	_, _ = s.db.ExecContext(ctx, "UPDATE bia_msgs SET text = ? WHERE id = ?", text, id)
}

func (s *Store) GetMsg(ctx context.Context, id string) (*MsgRef, error) {
	var m MsgRef
	var me, e2ee int
	err := s.db.QueryRowContext(ctx, "SELECT id, chat, sender, me, e2ee, ts, text FROM bia_msgs WHERE id = ?", id).
		Scan(&m.ID, &m.Chat, &m.Sender, &me, &e2ee, &m.TS, &m.Text)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	m.Me, m.E2EE = me != 0, e2ee != 0
	return &m, err
}

func (s *Store) DeleteMsg(ctx context.Context, id string) {
	_, _ = s.db.ExecContext(ctx, "DELETE FROM bia_msgs WHERE id = ?", id)
}

// LatestMsgs is the newest messages of a chat (for read marks).
func (s *Store) LatestMsgs(ctx context.Context, chat string, limit int) ([]*MsgRef, error) {
	rows, err := s.db.QueryContext(ctx, "SELECT id, chat, sender, me, e2ee, ts, text FROM bia_msgs WHERE chat = ? ORDER BY ts DESC LIMIT ?", chat, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []*MsgRef
	for rows.Next() {
		var m MsgRef
		var me, e2ee int
		if err := rows.Scan(&m.ID, &m.Chat, &m.Sender, &me, &e2ee, &m.TS, &m.Text); err != nil {
			return nil, err
		}
		m.Me, m.E2EE = me != 0, e2ee != 0
		out = append(out, &m)
	}
	return out, rows.Err()
}

// Recent is a chat's newest messages, newest first; before is the id the previous page ended with.
func (s *Store) Recent(ctx context.Context, chat string, limit int, before string) ([]*MsgRef, error) {
	ts := int64(1) << 62
	if before != "" {
		ref, err := s.GetMsg(ctx, before)
		if err != nil {
			return nil, err
		} else if ref == nil {
			return nil, fmt.Errorf("unknown message id %s", before)
		}
		ts = ref.TS
	}
	rows, err := s.db.QueryContext(ctx, "SELECT id, chat, sender, me, e2ee, ts, text FROM bia_msgs WHERE chat = ? AND ts < ? ORDER BY ts DESC LIMIT ?", chat, ts, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []*MsgRef
	for rows.Next() {
		var m MsgRef
		var me, e2ee int
		if err := rows.Scan(&m.ID, &m.Chat, &m.Sender, &me, &e2ee, &m.TS, &m.Text); err != nil {
			return nil, err
		}
		m.Me, m.E2EE = me != 0, e2ee != 0
		out = append(out, &m)
	}
	return out, rows.Err()
}

// Prune keeps the newest msgCap messages.
func (s *Store) Prune(ctx context.Context) {
	_, _ = s.db.ExecContext(ctx, `DELETE FROM bia_msgs WHERE id IN (
		SELECT id FROM bia_msgs ORDER BY ts DESC LIMIT -1 OFFSET ?)`, msgCap)
}

func (s *Store) GetKV(ctx context.Context, key string) string {
	var v string
	_ = s.db.QueryRowContext(ctx, "SELECT value FROM bia_kv WHERE key = ?", key).Scan(&v)
	return v
}

func (s *Store) PutKV(ctx context.Context, key, value string) {
	_, _ = s.db.ExecContext(ctx, "INSERT INTO bia_kv (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value", key, value)
}

func (s *Store) DelKV(ctx context.Context, key string) {
	_, _ = s.db.ExecContext(ctx, "DELETE FROM bia_kv WHERE key = ?", key)
}

// KVPrefix is every key/value under a prefix.
func (s *Store) KVPrefix(ctx context.Context, prefix string) map[string]string {
	out := map[string]string{}
	rows, err := s.db.QueryContext(ctx, "SELECT key, value FROM bia_kv WHERE substr(key, 1, ?) = ?", len(prefix), prefix)
	if err != nil {
		return out
	}
	defer rows.Close()
	for rows.Next() {
		var k, v string
		if rows.Scan(&k, &v) == nil {
			out[strings.TrimPrefix(k, prefix)] = v
		}
	}
	return out
}

func b2i(b bool) int {
	if b {
		return 1
	}
	return 0
}

func orStr(s, def string) string {
	if s == "" {
		return def
	}
	return s
}

func nowMS() int64 {
	return time.Now().UnixMilli()
}
