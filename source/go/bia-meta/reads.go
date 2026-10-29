package main

import (
	"context"
	"errors"
	"fmt"
	"sync"
	"time"
)

// errNotConnected: a read receipt could not go out because the connection is down.
var errNotConnected = errors.New("not connected")

// notConnected marks err as a connection gap (kept for after the reconnect); logged-out stays as is.
func notConnected(err error) error {
	if errors.Is(err, ErrNotLoggedIn) {
		return err
	}
	return fmt.Errorf("%w: %v", errNotConnected, err)
}

// maxPendingReads bounds the receipts kept while offline; the oldest go first.
const maxPendingReads = 200

// readRetry keeps read receipts that failed and sends each once more after the next
// (re)connect. One that fails again because the connection is still down waits for the
// connect after that; any other failure is logged and dropped.
type readRetry struct {
	mu      sync.Mutex
	pending []ReadParams
	send    func(context.Context, ReadParams) error
}

var reads readRetry

// mark sends a read receipt; a failure (other than logged out) is kept for the next connect.
func (r *readRetry) mark(ctx context.Context, send func(context.Context, ReadParams) error, p ReadParams) error {
	err := send(ctx, p)
	if err != nil && !errors.Is(err, ErrNotLoggedIn) {
		r.add(send, p)
	}
	return err
}

func (r *readRetry) add(send func(context.Context, ReadParams) error, p ReadParams) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.send = send
	for _, q := range r.pending {
		if q == p {
			return
		}
	}
	r.pending = append(r.pending, p)
	if n := len(r.pending) - maxPendingReads; n > 0 {
		r.pending = r.pending[n:]
	}
}

// flush sends the kept receipts; called when a connection comes up.
func (r *readRetry) flush(ctx context.Context) {
	r.mu.Lock()
	todo, send := r.pending, r.send
	r.pending = nil
	r.mu.Unlock()
	for _, p := range todo {
		c, cancel := context.WithTimeout(ctx, 30*time.Second)
		err := send(c, p)
		cancel()
		switch {
		case err == nil:
		case errors.Is(err, errNotConnected):
			r.add(send, p)
		default:
			log.Warn().Err(err).Str("chat", p.Chat).Str("id", p.ID).Msg("read receipt failed again; dropped")
		}
	}
}
