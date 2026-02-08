package tui

import "testing"

func TestParseHeadings_IgnoresFences(t *testing.T) {
	md := "" +
		"# A\n" +
		"\n" +
		"```go\n" +
		"# not a heading\n" +
		"```\n" +
		"\n" +
		"## B\n"

	hs := parseHeadings(md)
	if len(hs) != 2 {
		t.Fatalf("expected 2 headings, got %d", len(hs))
	}
	if hs[0].Text != "A" || hs[0].Level != 1 {
		t.Fatalf("unexpected heading[0]: %+v", hs[0])
	}
	if hs[1].Text != "B" || hs[1].Level != 2 {
		t.Fatalf("unexpected heading[1]: %+v", hs[1])
	}
}
