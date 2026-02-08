package tui

import "testing"

func TestComputeHeadingLocsFromRendered_InOrder(t *testing.T) {
	plain := []string{
		"Title",
		"intro",
		"Section A",
		"body",
		"Section B",
		"tail",
	}
	hs := []heading{
		{Level: 1, Text: "Title", Line: 0},
		{Level: 2, Text: "Section A", Line: 10},
		{Level: 2, Text: "Section B", Line: 20},
	}

	locs := computeHeadingLocsFromRendered(plain, hs)
	if len(locs) != 3 {
		t.Fatalf("expected 3 locs, got %d", len(locs))
	}
	if locs[0].RenderedLine != 0 || locs[1].RenderedLine != 2 || locs[2].RenderedLine != 4 {
		t.Fatalf("unexpected rendered lines: %+v", locs)
	}
}

func TestCurrentHeadingIndex(t *testing.T) {
	locs := []headingLoc{
		{Heading: heading{Level: 1, Text: "A", Line: 0}, RenderedLine: 0},
		{Heading: heading{Level: 2, Text: "B", Line: 1}, RenderedLine: 10},
		{Heading: heading{Level: 2, Text: "C", Line: 2}, RenderedLine: 20},
	}

	if got := currentHeadingIndex(locs, 0); got != 0 {
		t.Fatalf("anchor=0 got %d", got)
	}
	if got := currentHeadingIndex(locs, 15); got != 1 {
		t.Fatalf("anchor=15 got %d", got)
	}
	if got := currentHeadingIndex(locs, 100); got != 2 {
		t.Fatalf("anchor=100 got %d", got)
	}
}

func TestBreadcrumbForIndex_ParentChain(t *testing.T) {
	locs := []headingLoc{
		{Heading: heading{Level: 1, Text: "H1", Line: 0}, RenderedLine: 0},
		{Heading: heading{Level: 2, Text: "H2", Line: 1}, RenderedLine: 5},
		{Heading: heading{Level: 3, Text: "H3", Line: 2}, RenderedLine: 10},
		{Heading: heading{Level: 2, Text: "H2b", Line: 3}, RenderedLine: 15},
	}

	chain := breadcrumbForIndex(locs, 2)
	if len(chain) != 3 {
		t.Fatalf("expected 3 chain elements, got %d", len(chain))
	}
	if chain[0].Heading.Text != "H1" || chain[1].Heading.Text != "H2" || chain[2].Heading.Text != "H3" {
		t.Fatalf("unexpected chain: %+v", chain)
	}
}
