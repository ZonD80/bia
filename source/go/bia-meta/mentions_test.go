package main

import "testing"

func TestFindTags(t *testing.T) {
	m := []tagMember{{ID: 1, Names: []string{"Ann Lee", "Ann", "ann_lee"}}, {ID: 2, Names: []string{"Bob"}}}
	text := "@ann lee and @Bob, mail bob@x.com, @Bobby, @ann_lee"
	got := findTags(text, m)
	want := []struct {
		s  string
		id int64
	}{{"@ann lee", 1}, {"@Bob", 2}, {"@ann_lee", 1}}
	if len(got) != len(want) {
		t.Fatalf("got %d tags: %+v", len(got), got)
	}
	for i, w := range want {
		g := got[i]
		if text[g.Offset:g.Offset+g.Length] != w.s || g.ID != w.id {
			t.Errorf("tag %d: %q id %d, want %q id %d", i, text[g.Offset:g.Offset+g.Length], g.ID, w.s, w.id)
		}
	}
}
