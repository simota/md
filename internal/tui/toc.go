package tui

import (
	"fmt"
	"strings"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"

	"github.com/simota/md/internal/render"
)

type heading struct {
	Level int
	Text  string
	Line  int // 0-based line index in raw markdown
}

func parseHeadings(md string) []heading {
	lines := strings.Split(strings.ReplaceAll(md, "\r\n", "\n"), "\n")

	var hs []heading
	inFence := false
	var fence string

	for i, ln := range lines {
		t := strings.TrimSpace(ln)

		if isFenceLine(t) {
			// Toggle on ``` or ~~~. Keep fence type to avoid accidental toggles.
			if !inFence {
				inFence = true
				fence = t[:3]
			} else if fence != "" && strings.HasPrefix(t, fence) {
				inFence = false
				fence = ""
			}
			continue
		}
		if inFence {
			continue
		}

		if !strings.HasPrefix(t, "#") {
			continue
		}
		n := countPrefix(t, '#')
		if n < 1 || n > 6 {
			continue
		}
		rest := strings.TrimSpace(t[n:])
		if rest == "" {
			continue
		}
		hs = append(hs, heading{
			Level: n,
			Text:  rest,
			Line:  i,
		})
	}

	return hs
}

func isFenceLine(t string) bool {
	return strings.HasPrefix(t, "```") || strings.HasPrefix(t, "~~~")
}

func countPrefix(s string, ch byte) int {
	n := 0
	for i := 0; i < len(s) && s[i] == ch; i++ {
		n++
	}
	return n
}

func (m *model) handleTOCKey(msg tea.KeyMsg) {
	if m.tocFilterMode {
		m.handleTOCFilterKey(msg)
		return
	}

	switch msg.String() {
	case "esc", "q", "t":
		m.showTOC = false
		return
	case "/":
		m.tocFilterMode = true
		m.tocFilterDraft = m.tocFilter
		return
	case "j", "down":
		m.tocIdx++
	case "k", "up":
		m.tocIdx--
	case "home", "g":
		m.tocIdx = 0
	case "end", "G":
		m.tocIdx = len(m.tocFilteredHeadings()) - 1
	case "enter":
		m.jumpToHeading()
		m.showTOC = false
		return
	}
	m.tocIdx = clamp(m.tocIdx, 0, max(0, len(m.tocFilteredHeadings())-1))
}

func (m *model) handleTOCFilterKey(msg tea.KeyMsg) {
	switch msg.String() {
	case "esc":
		m.tocFilterMode = false
		m.tocFilterDraft = m.tocFilter
		m.tocIdx = clamp(m.tocIdx, 0, max(0, len(m.tocFilteredHeadings())-1))
		return
	case "enter":
		m.tocFilterMode = false
		m.tocFilter = strings.TrimSpace(m.tocFilterDraft)
		m.tocIdx = 0
		return
	case "backspace", "ctrl+h":
		if m.tocFilterDraft == "" {
			return
		}
		m.tocFilterDraft = dropLastRune(m.tocFilterDraft)
	case "ctrl+u":
		m.tocFilterDraft = ""
	default:
		if len(msg.Runes) > 0 {
			m.tocFilterDraft += string(msg.Runes)
		}
	}
	// Keep selection stable-ish as the filter changes.
	m.tocIdx = clamp(m.tocIdx, 0, max(0, len(m.tocFilteredHeadings())-1))
}

func (m *model) jumpToHeading() {
	hs := m.tocFilteredHeadings()
	if len(hs) == 0 || m.tocIdx < 0 || m.tocIdx >= len(hs) {
		return
	}
	h := hs[m.tocIdx]

	// Prefer already-computed heading mapping from current render, if available.
	if m.headingLocsWidth == m.currentRenderWidth() {
		if off, ok := m.headingByMDLine[h.Line]; ok {
			m.setOffsetForRenderedLine(off)
			return
		}
	}

	off, ok := m.tocOffsetCache[h.Line]
	if !ok || m.tocOffsetCacheWidth != m.currentRenderWidth() {
		m.tocOffsetCacheWidth = m.currentRenderWidth()
		if m.tocOffsetCache == nil {
			m.tocOffsetCache = map[int]int{}
		}
		off = m.computeOffsetForMarkdownLine(h.Line)
		m.tocOffsetCache[h.Line] = off
	}
	m.setOffsetForRenderedLine(off)
}

func (m model) currentRenderWidth() int {
	w := m.renderOpts.Width
	if w <= 0 {
		w = m.bodyTextWidth()
	}
	return w
}

func (m model) computeOffsetForMarkdownLine(line int) int {
	raw := strings.Split(strings.ReplaceAll(m.md, "\r\n", "\n"), "\n")
	if line < 0 {
		line = 0
	}
	if line > len(raw) {
		line = len(raw)
	}
	prefix := strings.Join(raw[:line+1], "\n") + "\n"

	out, err := render.RenderMarkdown(prefix, render.Options{
		Style: m.renderOpts.Style,
		Width: m.currentRenderWidth(),
	})
	if err != nil {
		return 0
	}
	// Place the heading line at the top of the viewport.
	return max(0, len(splitLines(out))-1)
}

func (m model) tocView() string {
	title := "TOC"
	if len(m.headings) == 0 {
		title = "TOC (no headings)"
	}

	titleBar := m.theme.Styles.TOCTitle.Render(title)

	hs := m.tocFilteredHeadings()
	filterText := "/ filter"
	if m.tocFilterMode {
		filterText = "/" + m.tocFilterDraft
	} else if strings.TrimSpace(m.tocFilter) != "" {
		filterText = fmt.Sprintf("/%s (%d/%d)", m.tocFilter, len(hs), len(m.headings))
	}
	filterBar := m.theme.Styles.TOCFilter.Render(truncateEnd(filterText, max(10, m.width-10)))

	header := titleBar + "\n" + filterBar

	help := "j/k move  Enter jump  / filter  Esc close"
	if m.tocFilterMode {
		help = "type to filter  Enter apply  Esc cancel"
	}
	footer := m.theme.Styles.TOCFooter.Render(help)

	bodyLines := m.tocBodyLines()
	body := strings.Join(bodyLines, "\n")

	box := m.theme.Styles.TOCBox.Render(header + "\n" + body + "\n" + footer)

	maxW := min(m.width-4, 80)
	maxH := min(m.height-2, 24)
	box = lipgloss.NewStyle().MaxWidth(maxW).MaxHeight(maxH).Render(box)

	return lipgloss.Place(
		m.width,
		m.height,
		lipgloss.Center,
		lipgloss.Center,
		box,
		lipgloss.WithWhitespaceBackground(m.theme.Colors.OverlayBg),
	)
}

func (m model) tocBodyLines() []string {
	innerW := min(m.width-8, 76)
	innerH := min(m.height-7, 19) // leave space for border + 2-line header + footer
	innerW = max(20, innerW)
	innerH = max(3, innerH)

	if len(m.headings) == 0 {
		return []string{"  (no headings found)"}
	}

	hs := m.tocFilteredHeadings()
	if len(hs) == 0 {
		active := strings.TrimSpace(m.tocFilter)
		if m.tocFilterMode {
			active = strings.TrimSpace(m.tocFilterDraft)
		}
		if active == "" {
			return []string{"  (no headings found)"}
		}
		return []string{"  (no matches)"}
	}

	start := clamp(m.tocIdx-innerH/2, 0, max(0, len(hs)-innerH))
	end := min(len(hs), start+innerH)

	var out []string
	sel := m.theme.Styles.TOCItemSelected
	norm := m.theme.Styles.TOCItemNormal

	for i := start; i < end; i++ {
		h := hs[i]
		indent := strings.Repeat("  ", clamp(h.Level-1, 0, 5))
		line := fmt.Sprintf("%s%s", indent, h.Text)
		line = truncateEnd(line, innerW-2)
		if i == m.tocIdx {
			out = append(out, sel.Render(line))
		} else {
			out = append(out, norm.Render(line))
		}
	}
	return out
}

func (m model) tocFilteredHeadings() []heading {
	active := strings.TrimSpace(m.tocFilter)
	if m.tocFilterMode {
		active = strings.TrimSpace(m.tocFilterDraft)
	}
	if active == "" {
		return m.headings
	}

	matcher := newSearchMatcher(active)
	var out []heading
	for _, h := range m.headings {
		if matcher.Contains(h.Text) {
			out = append(out, h)
		}
	}
	return out
}
