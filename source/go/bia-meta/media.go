package main

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"io"
	"mime"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"go.mau.fi/mautrix-meta/pkg/msgconv/mediadl"
	"go.mau.fi/util/exhttp"
)

// Meta CDN downloads (Instagram media, Messenger files outside E2EE) use mediadl's own
// HTTP client, which is nil until set; the mautrix bridges set it at start.
func init() {
	mediadl.SetHTTP(exhttp.SensibleClientSettings)
}

// Attachments of incoming messages are not downloaded on arrival: the event carries a
// ref, and the MCP asks for the file (download) once the message's turn comes.

type fetchFunc func(ctx context.Context) (data []byte, mimeType string, err error)

type mediaRef struct {
	fetch fetchFunc
	at    time.Time
}

type mediaRegistry struct {
	mu   sync.Mutex
	refs map[string]*mediaRef
}

var media = &mediaRegistry{refs: map[string]*mediaRef{}}

const (
	refTTL      = 12 * time.Hour
	refCap      = 5000
	maxDownload = 512 << 20
)

func (r *mediaRegistry) add(fetch fetchFunc) string {
	var b [12]byte
	_, _ = rand.Read(b[:])
	ref := hex.EncodeToString(b[:])
	r.mu.Lock()
	defer r.mu.Unlock()
	now := time.Now()
	if len(r.refs) >= refCap {
		for k, v := range r.refs {
			if now.Sub(v.at) > refTTL || len(r.refs) >= refCap {
				delete(r.refs, k)
			}
		}
	}
	r.refs[ref] = &mediaRef{fetch: fetch, at: now}
	return ref
}

// urlRef is a ref for a plain (Meta CDN) URL.
func urlRef(url, mimeType string) string {
	return media.add(func(ctx context.Context) ([]byte, string, error) {
		return media.fetchURL(ctx, url, mimeType)
	})
}

// fetchURL downloads from Meta's CDN with the headers it expects.
func (r *mediaRegistry) fetchURL(ctx context.Context, url, mimeType string) ([]byte, string, error) {
	size, body, err := mediadl.DownloadMedia(ctx, mimeType, url, maxDownload)
	if err != nil {
		return nil, "", err
	}
	defer body.Close()
	data, err := io.ReadAll(io.LimitReader(body, maxDownload))
	if err != nil {
		return nil, "", err
	}
	if size > 0 && int64(len(data)) < size {
		return nil, "", fmt.Errorf("download cut short (%d of %d bytes)", len(data), size)
	}
	if mimeType == "" {
		mimeType = http.DetectContentType(data)
	}
	return data, mimeType, nil
}

func (r *mediaRegistry) Download(ctx context.Context, p DownloadParams) (map[string]any, error) {
	r.mu.Lock()
	ref := r.refs[p.Ref]
	r.mu.Unlock()
	if ref == nil {
		return nil, fmt.Errorf("unknown or expired attachment %q", p.Ref)
	}
	if p.Path == "" {
		return nil, fmt.Errorf("path required")
	}
	data, mimeType, err := ref.fetch(ctx)
	if err != nil {
		return nil, err
	}
	if len(data) == 0 {
		return nil, fmt.Errorf("empty file")
	}
	if err = os.MkdirAll(filepath.Dir(p.Path), 0o755); err != nil {
		return nil, err
	}
	tmp := p.Path + ".part"
	if err = os.WriteFile(tmp, data, 0o644); err != nil {
		return nil, err
	}
	if err = os.Rename(tmp, p.Path); err != nil {
		return nil, err
	}
	return map[string]any{"path": p.Path, "mime": mimeType, "size": len(data)}, nil
}

// Attachment is how an incoming file is described to the MCP.
type Attachment struct {
	Ref     string  `json:"ref,omitempty"`
	Kind    string  `json:"kind"`
	Mime    string  `json:"mime,omitempty"`
	Name    string  `json:"name,omitempty"`
	Size    int64   `json:"size,omitempty"`
	Seconds float64 `json:"seconds,omitempty"`
	// why it cannot be downloaded (view-once media, unsupported kind)
	Note string `json:"note,omitempty"`
}

// kindOf maps a MIME type to image / video / audio / document.
func kindOf(mimeType string) string {
	switch {
	case strings.HasPrefix(mimeType, "image/gif"):
		return "gif"
	case strings.HasPrefix(mimeType, "image/"):
		return "image"
	case strings.HasPrefix(mimeType, "video/"):
		return "video"
	case strings.HasPrefix(mimeType, "audio/"):
		return "audio"
	default:
		return "document"
	}
}

var extMime = map[string]string{
	".jpg": "image/jpeg", ".jpeg": "image/jpeg", ".png": "image/png", ".webp": "image/webp", ".gif": "image/gif",
	".heic": "image/heic", ".mp4": "video/mp4", ".mov": "video/quicktime", ".webm": "video/webm", ".mkv": "video/x-matroska",
	".mp3": "audio/mpeg", ".m4a": "audio/mp4", ".aac": "audio/aac", ".ogg": "audio/ogg", ".oga": "audio/ogg",
	".opus": "audio/ogg", ".wav": "audio/wav", ".pdf": "application/pdf", ".txt": "text/plain", ".zip": "application/zip",
}

// readFile loads a file to send and works out its MIME type and name.
func readFile(f FileParam) (data []byte, mimeType, name string, err error) {
	data, err = os.ReadFile(f.Path)
	if err != nil {
		return nil, "", "", err
	}
	name = f.Name
	if name == "" {
		name = filepath.Base(f.Path)
	}
	mimeType = f.Mime
	if mimeType == "" {
		ext := strings.ToLower(filepath.Ext(f.Path))
		mimeType = extMime[ext]
		if mimeType == "" {
			mimeType = mime.TypeByExtension(ext)
		}
		if mimeType == "" {
			mimeType = http.DetectContentType(data)
		}
	}
	if i := strings.Index(mimeType, ";"); i > 0 && !strings.HasPrefix(mimeType, "audio/ogg") {
		mimeType = strings.TrimSpace(mimeType[:i])
	}
	return data, mimeType, name, nil
}

// shownText is what history keeps for a message: its text, or "[voice]" / "[image]" … for
// a message that is only attachments.
func shownText(text string, atts []Attachment) string {
	if text != "" || len(atts) == 0 {
		return text
	}
	kinds := make([]string, 0, len(atts))
	for _, a := range atts {
		kinds = append(kinds, "["+a.Kind+"]")
	}
	return strings.Join(kinds, " ")
}
