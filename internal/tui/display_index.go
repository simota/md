package tui

// displayIndex maps "display row" -> "rendered line index".
// In normal mode it's an identity mapping; in fold mode it becomes a filtered list.
type displayIndex struct {
	kind  displayIndexKind
	size  int
	lines []int
}

type displayIndexKind int

const (
	displayIndexIdentity displayIndexKind = iota
	displayIndexList
)

func newIdentityDisplayIndex(size int) displayIndex {
	return displayIndex{kind: displayIndexIdentity, size: size}
}

func newListDisplayIndex(lines []int) displayIndex {
	return displayIndex{kind: displayIndexList, size: len(lines), lines: lines}
}

func (d displayIndex) Len() int { return d.size }

func (d displayIndex) At(row int) int {
	if row < 0 {
		return 0
	}
	switch d.kind {
	case displayIndexList:
		if row >= len(d.lines) {
			return d.lines[len(d.lines)-1]
		}
		return d.lines[row]
	default:
		if row >= d.size {
			return d.size - 1
		}
		return row
	}
}
