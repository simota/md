package tui

import (
	"strings"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

func (m *model) handleSearchKey(msg tea.KeyMsg) {
	switch msg.String() {
	case "esc":
		m.searchMode = false
		m.searchDraft = m.searchSavedQuery
		m.setSearchQueryNoJump(m.searchSavedQuery)
		return
	case "enter":
		m.searchMode = false
		m.setSearchQuery(m.searchDraft)
		return
	case "backspace", "ctrl+h":
		if m.searchDraft == "" {
			return
		}
		m.searchDraft = dropLastRune(m.searchDraft)
		m.setSearchQueryNoJump(m.searchDraft)
		return
	case "ctrl+u":
		m.searchDraft = ""
		m.setSearchQueryNoJump(m.searchDraft)
		return
	}

	// Best-effort: append printable runes.
	if len(msg.Runes) > 0 {
		m.searchDraft += string(msg.Runes)
		m.setSearchQueryNoJump(m.searchDraft)
	}
}

func (m *model) setSearchQuery(q string) {
	m.setSearchQueryNoJump(q)
	if m.searchCurrentLine >= 0 {
		m.setOffsetForRenderedLine(m.searchCurrentLine)
	}
}

func (m *model) setSearchQueryNoJump(q string) {
	q = strings.TrimSpace(q)
	m.searchQuery = q
	m.searchDraft = q
	m.searchIdx = 0
	m.searchCurrentLine = -1
	m.searchMatches = nil
	m.searchSet = map[int]bool{}
	if q == "" {
		return
	}

	m.recomputeSearch()
	if len(m.searchMatches) == 0 {
		return
	}

	// Pick first match at/after current offset, otherwise wrap to first.
	i := 0
	for i < len(m.searchMatches) && m.searchMatches[i] < m.offset {
		i++
	}
	if i >= len(m.searchMatches) {
		i = 0
	}
	m.searchIdx = i
	m.searchCurrentLine = m.searchMatches[m.searchIdx]
}

func (m *model) recomputeSearch() {
	m.searchMatches = nil
	m.searchSet = map[int]bool{}
	m.searchCurrentLine = -1

	if m.searchQuery == "" {
		return
	}

	matcher := newSearchMatcher(m.searchQuery)
	for i := 0; i < len(m.plain); i++ {
		if matcher.Contains(m.plain[i]) {
			m.searchMatches = append(m.searchMatches, i)
			m.searchSet[i] = true
		}
	}

	if len(m.searchMatches) == 0 {
		m.searchIdx = 0
		return
	}

	// Keep current match if possible.
	if m.searchIdx >= 0 && m.searchIdx < len(m.searchMatches) {
		m.searchCurrentLine = m.searchMatches[m.searchIdx]
	}
}

func (m *model) jumpNextMatch(delta int) {
	if m.searchQuery == "" || len(m.searchMatches) == 0 {
		return
	}
	if delta >= 0 {
		m.searchIdx = (m.searchIdx + 1) % len(m.searchMatches)
	} else {
		m.searchIdx--
		if m.searchIdx < 0 {
			m.searchIdx = len(m.searchMatches) - 1
		}
	}
	m.searchCurrentLine = m.searchMatches[m.searchIdx]
	m.setOffsetForRenderedLine(m.searchCurrentLine)
}

func (m model) currentMatchNumber() int {
	if m.searchQuery == "" || len(m.searchMatches) == 0 {
		return 0
	}
	return m.searchIdx + 1
}

func (m model) markerGutter(lineIdx int) string {
	// 3 columns: " <marker> "
	// marker is:
	// - '>' current match
	// - '*' other match
	// - '§' heading
	// - ' ' none
	marker := ' '
	if m.isHeadingRenderedLine(lineIdx) {
		marker = '§'
	}
	if m.searchQuery != "" {
		if lineIdx == m.searchCurrentLine {
			marker = '>'
		} else if m.searchSet[lineIdx] {
			marker = '*'
		}
	}
	var st lipgloss.Style
	switch marker {
	case '§':
		st = m.theme.Styles.MarkerHeading
	case '>':
		st = m.theme.Styles.MarkerMatchCurrent
	case '*':
		st = m.theme.Styles.MarkerMatchOther
	default:
		st = m.theme.Styles.MarkerNone
		marker = ' '
	}
	return " " + st.Render(string(marker)) + " "
}

func leftGutterPad() string { return "   " }

type searchMatcher struct {
	query     string
	lower     string
	sensitive bool
}

func newSearchMatcher(q string) searchMatcher {
	s := searchMatcher{query: q}
	s.sensitive = hasUpper(q)
	if !s.sensitive {
		s.lower = strings.ToLower(q)
	}
	return s
}

func (m searchMatcher) Contains(line string) bool {
	if m.sensitive {
		return strings.Contains(line, m.query)
	}
	return strings.Contains(strings.ToLower(line), m.lower)
}

func hasUpper(s string) bool {
	for _, r := range s {
		if r >= 'A' && r <= 'Z' {
			return true
		}
	}
	return false
}

func stripANSI(s string) string {
	// Minimal ANSI stripper for search indexing. Removes CSI sequences.
	// Example: "\x1b[31mred\x1b[0m" -> "red".
	var b strings.Builder
	b.Grow(len(s))

	for i := 0; i < len(s); i++ {
		if s[i] != 0x1b {
			b.WriteByte(s[i])
			continue
		}

		// ESC
		if i+1 >= len(s) {
			break
		}
		if s[i+1] != '[' {
			// Not CSI; skip ESC only.
			continue
		}
		// Skip CSI until final byte in 0x40-0x7E range.
		i += 2
		for i < len(s) {
			c := s[i]
			if c >= 0x40 && c <= 0x7e {
				break
			}
			i++
		}
	}

	return b.String()
}

func dropLastRune(s string) string {
	if s == "" {
		return ""
	}
	r := []rune(s)
	if len(r) <= 1 {
		return ""
	}
	return string(r[:len(r)-1])
}
