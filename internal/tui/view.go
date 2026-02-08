package tui

import (
	"fmt"
	"os"
	"strings"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"

	"github.com/simota/md/internal/render"
)

type model struct {
	title string

	md         string
	renderOpts render.Options
	theme      Theme

	lines  []string
	plain  []string
	offset int // display row offset (top of viewport)

	foldLevel int // 0 = no fold (full), 1..6 = outline up to that heading level
	display   displayIndex

	width  int
	height int
	ready  bool

	showHelp bool
	showTOC  bool

	headings         []heading
	headingSet       map[string]int
	headingLocs      []headingLoc
	headingLocsWidth int
	headingLineSet   map[int]bool
	headingByMDLine  map[int]int // raw markdown line -> rendered line
	tocIdx           int
	tocFilterMode    bool
	tocFilterDraft   string
	tocFilter        string

	// Cache of raw markdown line -> rendered offset for current render width.
	tocOffsetCache      map[int]int
	tocOffsetCacheWidth int

	searchMode        bool
	searchSavedQuery  string
	searchDraft       string
	searchQuery       string
	searchMatches     []int
	searchIdx         int
	searchSet         map[int]bool
	searchCurrentLine int

	statusMessage string

	lastErr error
}

type clearStatusMsg struct{}

func ViewMarkdown(title string, md string, opts render.Options, stdout *os.File) error {
	m := model{
		title:           title,
		md:              md,
		renderOpts:      opts,
		theme:           themeFor(opts.Style),
		headings:        parseHeadings(md),
		headingSet:      map[string]int{},
		headingLineSet:  map[int]bool{},
		headingByMDLine: map[int]int{},
		searchSet:       map[int]bool{},
	}
	for _, h := range m.headings {
		m.headingSet[normalizeText(h.Text)] = h.Level
	}
	m.display = newIdentityDisplayIndex(0)

	p := tea.NewProgram(
		m,
		tea.WithOutput(stdout),
		tea.WithMouseAllMotion(),
	)
	_, err := p.Run()
	return err
}

func (m model) Init() tea.Cmd { return nil }

func (m model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.KeyMsg:
		if m.searchMode {
			m.handleSearchKey(msg)
			m.offset = clamp(m.offset, 0, m.maxOffset())
			return m, nil
		}

		if m.showTOC {
			m.handleTOCKey(msg)
			m.offset = clamp(m.offset, 0, m.maxOffset())
			return m, nil
		}

		switch msg.String() {
		case "q", "esc", "ctrl+c":
			return m, tea.Quit
		case "?":
			m.showHelp = !m.showHelp
			return m, nil
		case "t":
			m.showTOC = !m.showTOC
			m.showHelp = false
			if m.showTOC {
				m.tocIdx = clamp(m.tocIdx, 0, max(0, len(m.headings)-1))
				m.syncTOCToCurrentHeading()
				m.tocFilterMode = false
				m.tocFilterDraft = m.tocFilter
			}
			return m, nil
		case "/":
			m.searchMode = true
			m.searchSavedQuery = m.searchQuery
			m.searchDraft = m.searchQuery
			m.setSearchQueryNoJump(m.searchDraft)
			m.showHelp = false
			m.showTOC = false
			return m, nil
		}

		if m.showHelp {
			// While help is open, only allow closing with "?" or quit keys above.
			return m, nil
		}

		switch msg.String() {
		case "0":
			m.foldLevel = 0
			m.rebuildDisplay()
			m.statusMessage = "Outline: off (press 1-6 to fold)"
			return m, m.statusTick()
		case "j", "down":
			m.offset++
		case "k", "up":
			m.offset--
		case "1", "2", "3", "4", "5", "6":
			m.foldLevel = int(msg.String()[0] - '0')
			m.rebuildDisplay()
			m.statusMessage = fmt.Sprintf("Outline: H%d (press 0 to show all)", m.foldLevel)
			return m, m.statusTick()
		case "d":
			m.offset += max(1, m.pageSize()/2)
		case "u":
			m.offset -= max(1, m.pageSize()/2)
		case "pgdown", "f", " ":
			m.offset += max(1, m.pageSize())
		case "pgup", "b":
			m.offset -= max(1, m.pageSize())
		case "home", "g":
			m.offset = 0
		case "end", "G":
			m.offset = m.maxOffset()
		case "]":
			m.jumpHeading(+1)
		case "[":
			m.jumpHeading(-1)
		case "n":
			m.jumpNextMatch(+1)
		case "N":
			m.jumpNextMatch(-1)
		case "c":
			// Clear search.
			m.setSearchQuery("")
		}
	case tea.MouseMsg:
		// Keep mouse handling minimal and reliable:
		// wheel up/down scrolls content.
		if m.showHelp {
			return m, nil
		}
		switch msg.Type {
		case tea.MouseWheelUp:
			m.offset -= 3
		case tea.MouseWheelDown:
			m.offset += 3
		}
	case tea.WindowSizeMsg:
		m.width = msg.Width
		m.height = msg.Height
		m.ready = true
		m.tocOffsetCache = map[int]int{}
		m.tocOffsetCacheWidth = 0
		m.reRender()
	case clearStatusMsg:
		m.statusMessage = ""
	}

	m.offset = clamp(m.offset, 0, m.maxOffset())
	return m, nil
}

func (m *model) reRender() {
	renderWidth := m.renderOpts.Width
	if renderWidth <= 0 {
		renderWidth = m.bodyTextWidth()
	}

	out, err := render.RenderMarkdown(m.md, render.Options{
		Style: m.renderOpts.Style,
		Width: renderWidth,
	})
	if err != nil {
		m.lastErr = err
		m.lines = []string{"(render error)", err.Error()}
		m.plain = []string{"(render error)", err.Error()}
		m.headingLocs = nil
		m.headingLineSet = map[int]bool{}
		m.headingByMDLine = map[int]int{}
		return
	}
	m.lastErr = nil
	m.lines = splitLines(out)
	m.plain = make([]string, 0, len(m.lines))
	for _, ln := range m.lines {
		m.plain = append(m.plain, stripANSI(ln))
	}

	m.refreshHeadingLocs(renderWidth)
	m.rebuildDisplay()

	// Refresh search matches after rerender (e.g. resize changes wrapping).
	if m.searchQuery != "" {
		m.recomputeSearch()
	}
}

func (m model) View() string {
	if !m.ready || m.width <= 0 || m.height <= 0 {
		return ""
	}

	if m.showHelp {
		return m.helpView()
	}

	if m.showTOC {
		return m.tocView()
	}

	header := m.headerView()
	body := m.bodyView()
	footer := m.footerView()

	// If terminal is too short, fall back gracefully.
	if m.height <= 2 {
		return header + "\n"
	}
	return header + "\n" + body + "\n" + footer
}

func (m model) headerView() string {
	title := strings.TrimSpace(m.title)
	if title == "" {
		title = "md"
	}

	pct := m.progressPercent()
	rightText := fmt.Sprintf("%3d%%", pct)
	if m.foldLevel > 0 {
		rightText = fmt.Sprintf("%3d%%  H%d", pct, m.foldLevel)
	}
	right := m.theme.Styles.HeaderRight.Render(rightText)

	label := title
	if bc := m.currentBreadcrumb(); bc != "" {
		label = title + "  \u203a  " + bc
	}

	leftStyle := m.theme.Styles.HeaderLeft

	maxLeftTotal := max(4, m.width-lipgloss.Width(right))
	// Account for left padding (2 columns).
	left := leftStyle.Render(truncateEnd(label, max(1, maxLeftTotal-2)))

	space := max(0, m.width-lipgloss.Width(left)-lipgloss.Width(right))
	mid := strings.Repeat(" ", space)

	return left + mid + right
}

func (m model) footerView() string {
	startDoc, endDoc, totalDoc := m.visibleDocRange()
	meta := fmt.Sprintf("doc %d-%d/%d", startDoc, endDoc, totalDoc)
	if m.foldLevel > 0 {
		startOL, endOL, totalOL := m.visibleOutlineRange()
		meta = fmt.Sprintf("doc %d-%d/%d | ol %d-%d/%d", startDoc, endDoc, totalDoc, startOL, endOL, totalOL)
	}

	help := "q quit  ? help  / search  t toc  [ ] section  1-6 fold 0 all"

	leftText := help
	if m.statusMessage != "" && !m.searchMode && !m.showTOC {
		leftText = m.statusMessage
	}
	if m.searchMode {
		if strings.TrimSpace(m.searchDraft) == "" {
			leftText = "/"
		} else {
			leftText = fmt.Sprintf("/%s (%d) Enter jump Esc cancel", m.searchDraft, len(m.searchMatches))
		}
	}
	if !m.searchMode && m.searchQuery != "" {
		leftText = fmt.Sprintf("/%s %d/%d (n/N)",
			m.searchQuery,
			m.currentMatchNumber(),
			len(m.searchMatches),
		)
	}

	left := m.theme.Styles.Footer.Render(truncateEnd(leftText, max(10, m.width-20)))

	right := m.theme.Styles.Footer.Render(meta)

	space := max(0, m.width-lipgloss.Width(left)-lipgloss.Width(right))
	mid := strings.Repeat(" ", space)

	return left + mid + right
}

func (m model) bodyView() string {
	contentHeight := max(1, m.height-2) // header+footer
	start := clamp(m.offset, 0, max(0, m.display.Len()))
	end := clamp(start+contentHeight, 0, m.display.Len())

	textWidth := m.bodyTextWidth()
	scroll := m.scrollbar(contentHeight)

	var b strings.Builder

	// A subtle gutter makes content easier to scan in long documents.
	for row := start; row < end; row++ {
		i := m.display.At(row)
		b.WriteString(m.markerGutter(i))
		text := padOrTruncateANSI(m.lines[i], textWidth)
		if m.isHeadingRenderedLine(i) {
			text = m.theme.Styles.HeadingLine.Render(text)
		}
		b.WriteString(text)
		b.WriteString(scroll.line(row - start))
		if row != end-1 {
			b.WriteByte('\n')
		}
	}

	// Pad with empty lines so footer stays pinned when at EOF.
	for i := end - start; i < contentHeight; i++ {
		if b.Len() > 0 {
			b.WriteByte('\n')
		}
		b.WriteString(leftGutterPad())
		b.WriteString(strings.Repeat(" ", textWidth))
		b.WriteString(scroll.line(i))
	}

	return b.String()
}

func (m model) helpView() string {
	lines := []string{
		"Keys",
		"",
		"  q / Esc        quit",
		"  j/k, Up/Down   scroll",
		"  PgUp/PgDn      page",
		"  u/d            half page",
		"  g/G            top / bottom",
		"  1-6 / 0        fold outline by heading level",
		"  [ / ]          previous/next heading",
		"  /              search (n/N to navigate, c to clear)",
		"  t              table of contents",
		"  / (in TOC)     filter headings",
		"  ?              toggle this help",
		"  mouse wheel    scroll",
	}

	box := m.theme.Styles.HelpBox.Render(strings.Join(lines, "\n"))

	// Dim overlay + centered box.
	return lipgloss.Place(
		m.width,
		m.height,
		lipgloss.Center,
		lipgloss.Center,
		box,
		lipgloss.WithWhitespaceBackground(m.theme.Colors.OverlayBg),
	)
}

func (m model) pageSize() int {
	return max(1, m.height-2)
}

func (m model) maxOffset() int {
	return max(0, m.display.Len()-m.pageSize())
}

func (m model) progressPercent() int {
	if len(m.lines) == 0 {
		return 0
	}
	docMaxOff := max(0, len(m.lines)-m.pageSize())
	if docMaxOff == 0 {
		return 100
	}
	top, _, _ := m.visibleDocRange()
	return int(float64(top) * 100.0 / float64(docMaxOff))
}

func (m model) visibleDocRange() (start int, end int, total int) {
	total = len(m.lines)
	if total == 0 || m.display.Len() == 0 {
		return 0, 0, total
	}

	topRow := clamp(m.offset, 0, m.display.Len()-1)
	bottomRow := clamp(m.offset+m.pageSize()-1, 0, m.display.Len()-1)

	start = m.display.At(topRow) + 1
	end = m.display.At(bottomRow) + 1
	start = clamp(start, 1, max(1, total))
	end = clamp(end, start, total)
	return start, end, total
}

func (m model) visibleOutlineRange() (start int, end int, total int) {
	total = m.display.Len()
	if total == 0 {
		return 0, 0, 0
	}
	start = clamp(m.offset+1, 1, total)
	end = clamp(m.offset+m.pageSize(), start, total)
	return start, end, total
}

func (m model) statusTick() tea.Cmd {
	return tea.Tick(1500*time.Millisecond, func(time.Time) tea.Msg {
		return clearStatusMsg{}
	})
}

func (m *model) refreshHeadingLocs(renderWidth int) {
	if len(m.headings) == 0 || len(m.plain) == 0 {
		m.headingLocs = nil
		m.headingLineSet = map[int]bool{}
		m.headingByMDLine = map[int]int{}
		m.headingLocsWidth = renderWidth
		return
	}
	if m.headingLocsWidth == renderWidth && len(m.headingLocs) > 0 && len(m.headingLineSet) > 0 {
		return
	}

	locs := computeHeadingLocsFromRendered(m.plain, m.headings)
	lineSet := map[int]bool{}
	byMD := map[int]int{}
	for _, loc := range locs {
		lineSet[loc.RenderedLine] = true
		byMD[loc.Heading.Line] = loc.RenderedLine
	}

	m.headingLocs = locs
	m.headingLineSet = lineSet
	m.headingByMDLine = byMD
	m.headingLocsWidth = renderWidth
}

func (m *model) rebuildDisplay() {
	// Default: identity mapping.
	if len(m.lines) == 0 {
		m.display = newIdentityDisplayIndex(0)
		m.offset = 0
		return
	}

	if m.foldLevel <= 0 {
		m.display = newIdentityDisplayIndex(len(m.lines))
		m.offset = clamp(m.offset, 0, m.maxOffset())
		return
	}

	// Outline view: show headings up to foldLevel.
	if len(m.headingLocs) == 0 {
		m.display = newIdentityDisplayIndex(len(m.lines))
		m.offset = clamp(m.offset, 0, m.maxOffset())
		return
	}

	var idx []int
	for _, loc := range m.headingLocs {
		if loc.Heading.Level <= m.foldLevel {
			idx = append(idx, loc.RenderedLine)
		}
	}

	// If filtering produced nothing, keep normal view (least surprising).
	if len(idx) == 0 {
		m.display = newIdentityDisplayIndex(len(m.lines))
		m.offset = clamp(m.offset, 0, m.maxOffset())
		return
	}

	// Keep current anchor stable by moving to the closest visible ancestor heading.
	anchor := m.anchorLine()
	target := idx[0]
	for _, v := range idx {
		if v <= anchor {
			target = v
			continue
		}
		break
	}

	m.display = newListDisplayIndex(idx)
	m.offset = m.displayRowForRenderedLine(target)
	m.offset = clamp(m.offset, 0, m.maxOffset())
}

func (m model) displayRowForRenderedLine(line int) int {
	if m.display.kind == displayIndexIdentity {
		return clamp(line, 0, max(0, m.display.Len()-1))
	}
	// Find last visible line <= target (stable for jumps to hidden lines).
	lo, hi := 0, len(m.display.lines)-1
	best := 0
	for lo <= hi {
		mid := (lo + hi) / 2
		v := m.display.lines[mid]
		if v <= line {
			best = mid
			lo = mid + 1
		} else {
			hi = mid - 1
		}
	}
	return best
}

func (m *model) setOffsetForRenderedLine(line int) {
	m.offset = m.displayRowForRenderedLine(line)
	m.offset = clamp(m.offset, 0, m.maxOffset())
}

func (m model) anchorLine() int {
	// Use a point slightly below the top for a more stable "current section".
	// anchorLine is in rendered-line coordinates.
	if m.display.Len() == 0 {
		return 0
	}
	row := clamp(m.offset+1, 0, m.display.Len()-1)
	return m.display.At(row)
}

func (m model) currentBreadcrumb() string {
	if len(m.headingLocs) == 0 {
		return ""
	}
	idx := currentHeadingIndex(m.headingLocs, m.anchorLine())
	if idx < 0 {
		return ""
	}
	chain := breadcrumbForIndex(m.headingLocs, idx)
	if len(chain) == 0 {
		return ""
	}
	parts := make([]string, 0, len(chain))
	for _, h := range chain {
		parts = append(parts, strings.TrimSpace(h.Heading.Text))
	}
	return strings.Join(parts, " \u203a ")
}

func (m model) currentHeadingMDLine() (int, bool) {
	if len(m.headingLocs) == 0 {
		return 0, false
	}
	idx := currentHeadingIndex(m.headingLocs, m.anchorLine())
	if idx < 0 {
		return 0, false
	}
	return m.headingLocs[idx].Heading.Line, true
}

func (m *model) jumpHeading(delta int) {
	if len(m.headingLocs) == 0 || delta == 0 {
		return
	}
	locs := m.headingLocs
	if m.foldLevel > 0 {
		var filtered []headingLoc
		for _, loc := range m.headingLocs {
			if loc.Heading.Level <= m.foldLevel {
				filtered = append(filtered, loc)
			}
		}
		if len(filtered) > 0 {
			locs = filtered
		}
	}

	idx := currentHeadingIndex(locs, m.anchorLine())
	if idx < 0 {
		if delta > 0 {
			idx = -1
		} else {
			return
		}
	}
	next := idx + delta
	next = clamp(next, 0, len(locs)-1)
	m.setOffsetForRenderedLine(locs[next].RenderedLine)
}

func (m *model) syncTOCToCurrentHeading() {
	mdLine, ok := m.currentHeadingMDLine()
	if !ok {
		return
	}
	hs := m.tocFilteredHeadings()
	for i := 0; i < len(hs); i++ {
		if hs[i].Line == mdLine {
			m.tocIdx = i
			return
		}
	}
}

func splitLines(s string) []string {
	s = strings.ReplaceAll(s, "\r\n", "\n")
	s = strings.TrimRight(s, "\n")
	if s == "" {
		return nil
	}
	return strings.Split(s, "\n")
}

func (m model) bodyTextWidth() int {
	// Layout:
	// - 3 chars marker gutter
	// - text column
	// - 1 char scrollbar (or blank space when content fits)
	// Keep guardrails for narrow terminals.
	w := m.width - 3 - 1
	return max(10, w)
}

type scrollbarModel struct {
	enabled   bool
	thumbTop  int
	thumbSize int
	track     string
	thumb     string
}

func (s scrollbarModel) line(row int) string {
	if !s.enabled {
		return " "
	}
	if row >= s.thumbTop && row < s.thumbTop+s.thumbSize {
		return s.thumb
	}
	return s.track
}

func (m model) scrollbar(visible int) scrollbarModel {
	total := m.display.Len()
	if total <= visible || visible <= 0 {
		return scrollbarModel{enabled: false}
	}

	thumbSize := max(1, (visible*visible)/total)
	thumbSize = min(visible, thumbSize)

	maxOff := m.maxOffset()
	thumbTop := 0
	if maxOff > 0 && visible > thumbSize {
		thumbTop = int(float64(m.offset) * float64(visible-thumbSize) / float64(maxOff))
	}
	thumbTop = clamp(thumbTop, 0, max(0, visible-thumbSize))

	return scrollbarModel{
		enabled:   true,
		thumbTop:  thumbTop,
		thumbSize: thumbSize,
		track:     m.theme.Styles.ScrollbarTrack.Render("."),
		thumb:     m.theme.Styles.ScrollbarThumb.Render("|"),
	}
}

func truncateEnd(s string, width int) string {
	if width <= 0 {
		return ""
	}
	if lipgloss.Width(s) <= width {
		return s
	}
	if width <= 3 {
		return strings.Repeat(".", width)
	}
	// Best-effort: assume ASCII titles. (Good enough for file basenames.)
	return s[:max(0, width-3)] + "..."
}

func padOrTruncateANSI(s string, width int) string {
	// "Pretty enough" for glamour output:
	// - it is already word-wrapped to fit.
	// - we mostly need padding so scrollbar aligns.
	w := lipgloss.Width(s)
	if w == width {
		return s
	}
	if w < width {
		return s + strings.Repeat(" ", width-w)
	}
	// Truncation could break ANSI sequences; but glamour should already fit.
	// Keep a conservative fallback anyway.
	return lipgloss.NewStyle().MaxWidth(width).Render(s)
}

func clamp(v, lo, hi int) int {
	if v < lo {
		return lo
	}
	if v > hi {
		return hi
	}
	return v
}

func max(a, b int) int {
	if a > b {
		return a
	}
	return b
}

func min(a, b int) int {
	if a < b {
		return a
	}
	return b
}

func normalizeText(s string) string {
	s = strings.TrimSpace(s)
	if s == "" {
		return ""
	}
	// Collapse whitespace for more robust matching against renderer output.
	s = strings.Join(strings.Fields(s), " ")
	return strings.ToLower(s)
}

func (m model) isHeadingRenderedLine(lineIdx int) bool {
	if len(m.headingLineSet) == 0 {
		return false
	}
	if lineIdx < 0 || lineIdx >= len(m.plain) {
		return false
	}
	return m.headingLineSet[lineIdx]
}
