package tui

import (
	"strings"
)

type headingLoc struct {
	Heading      heading
	RenderedLine int // 0-based in rendered output (m.lines)
}

func computeHeadingLocsFromRendered(plain []string, hs []heading) []headingLoc {
	if len(plain) == 0 || len(hs) == 0 {
		return nil
	}

	// Search forward so repeated headings resolve to their first occurrence after the previous.
	cursor := 0
	var out []headingLoc

	for _, h := range hs {
		target := normalizeText(h.Text)
		if target == "" {
			continue
		}

		found := -1
		for i := cursor; i < len(plain); i++ {
			if normalizeText(plain[i]) == target {
				found = i
				break
			}
		}
		// Fallback: if exact match is not found (renderer decoration), use contains.
		if found == -1 {
			for i := cursor; i < len(plain); i++ {
				if strings.Contains(normalizeText(plain[i]), target) {
					found = i
					break
				}
			}
		}
		if found == -1 {
			continue
		}

		out = append(out, headingLoc{
			Heading:      h,
			RenderedLine: found,
		})
		cursor = found + 1
	}

	return out
}

func currentHeadingIndex(locs []headingLoc, anchorLine int) int {
	if len(locs) == 0 {
		return -1
	}
	if anchorLine < 0 {
		anchorLine = 0
	}

	// locs are in document order; rendered line is non-decreasing.
	lo, hi := 0, len(locs)-1
	best := -1
	for lo <= hi {
		mid := (lo + hi) / 2
		if locs[mid].RenderedLine <= anchorLine {
			best = mid
			lo = mid + 1
		} else {
			hi = mid - 1
		}
	}
	return best
}

func breadcrumbForIndex(locs []headingLoc, idx int) []headingLoc {
	if idx < 0 || idx >= len(locs) {
		return nil
	}
	cur := locs[idx]
	chain := []headingLoc{cur}
	level := cur.Heading.Level

	// Walk backwards to pick the nearest parent headings.
	for i := idx - 1; i >= 0 && level > 1; i-- {
		h := locs[i]
		if h.Heading.Level < level {
			chain = append(chain, h)
			level = h.Heading.Level
		}
	}

	// Reverse.
	for i, j := 0, len(chain)-1; i < j; i, j = i+1, j-1 {
		chain[i], chain[j] = chain[j], chain[i]
	}
	return chain
}
