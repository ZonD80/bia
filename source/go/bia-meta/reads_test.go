package main

import (
	"context"
	"errors"
	"testing"
)

func TestReadRetry(t *testing.T) {
	var r readRetry
	ctx := context.Background()
	down := true
	var sent []string
	send := func(_ context.Context, p ReadParams) error {
		if down {
			return notConnected(errors.New("socket down"))
		}
		if p.ID == "bad" {
			return errors.New("refused")
		}
		sent = append(sent, p.ID)
		return nil
	}
	for _, id := range []string{"a", "b", "a", "bad"} {
		if err := r.mark(ctx, send, ReadParams{Chat: "1", ID: id}); !errors.Is(err, errNotConnected) {
			t.Fatalf("mark %s: %v", id, err)
		}
	}
	if len(r.pending) != 3 {
		t.Fatalf("pending %v", r.pending)
	}
	// still down at this connect: everything waits for the next one
	r.flush(ctx)
	if len(r.pending) != 3 || len(sent) != 0 {
		t.Fatalf("after down flush: pending %v sent %v", r.pending, sent)
	}
	down = false
	r.flush(ctx)
	if len(r.pending) != 0 || len(sent) != 2 || sent[0] != "a" || sent[1] != "b" {
		t.Fatalf("after flush: pending %v sent %v", r.pending, sent)
	}
	// logged out is not kept
	logout := func(context.Context, ReadParams) error { return ErrNotLoggedIn }
	_ = r.mark(ctx, logout, ReadParams{Chat: "1", ID: "c"})
	if len(r.pending) != 0 {
		t.Fatalf("kept a logged-out receipt: %v", r.pending)
	}
	if !errors.Is(notConnected(ErrNotLoggedIn), ErrNotLoggedIn) || errors.Is(notConnected(ErrNotLoggedIn), errNotConnected) {
		t.Fatal("notConnected must leave logged-out alone")
	}
}
