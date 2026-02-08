package tui

import (
	"strings"

	"github.com/charmbracelet/lipgloss"
	"github.com/muesli/termenv"
)

type themeMode int

const (
	themeDark themeMode = iota
	themeLight
)

// Theme defines the semantic token set for TUI chrome (header/footer/modals).
// Markdown rendering is handled by glamour; this only governs our own UI layers.
type Theme struct {
	Mode themeMode

	Colors ThemeColors
	Styles ThemeStyles
}

type ThemeColors struct {
	// Chrome
	HeaderBg      lipgloss.Color
	HeaderFg      lipgloss.Color
	HeaderFgMuted lipgloss.Color

	FooterBg lipgloss.Color
	FooterFg lipgloss.Color

	// Overlays / modals
	OverlayBg lipgloss.Color
	ModalBg   lipgloss.Color
	ModalFg   lipgloss.Color
	Border    lipgloss.Color

	// Body hints (non-glamour)
	HeadingLineBg lipgloss.Color
	HeadingLineFg lipgloss.Color

	// Interaction
	Accent lipgloss.Color

	SelectionBg lipgloss.Color
	SelectionFg lipgloss.Color

	// UI details
	ScrollbarTrack lipgloss.Color
	ScrollbarThumb lipgloss.Color

	MarkerDim lipgloss.Color
}

type ThemeStyles struct {
	HeaderLeft  lipgloss.Style
	HeaderRight lipgloss.Style

	Footer lipgloss.Style

	HeadingLine lipgloss.Style

	HelpBox lipgloss.Style

	TOCTitle  lipgloss.Style
	TOCFilter lipgloss.Style
	TOCFooter lipgloss.Style
	TOCBox    lipgloss.Style

	TOCItemSelected lipgloss.Style
	TOCItemNormal   lipgloss.Style

	MarkerHeading      lipgloss.Style
	MarkerMatchCurrent lipgloss.Style
	MarkerMatchOther   lipgloss.Style
	MarkerNone         lipgloss.Style

	ScrollbarTrack lipgloss.Style
	ScrollbarThumb lipgloss.Style
}

func themeFor(style string) Theme {
	switch strings.ToLower(strings.TrimSpace(style)) {
	case "dark":
		return newDarkTheme()
	case "light":
		return newLightTheme()
	default:
		if termenv.HasDarkBackground() {
			return newDarkTheme()
		}
		return newLightTheme()
	}
}

func newDarkTheme() Theme {
	c := ThemeColors{
		HeaderBg:      lipgloss.Color("#2B2B2B"),
		HeaderFg:      lipgloss.Color("#F2F2F2"),
		HeaderFgMuted: lipgloss.Color("#D0D0D0"),

		FooterBg: lipgloss.Color("#1E1E1E"),
		FooterFg: lipgloss.Color("#B8B8B8"),

		OverlayBg: lipgloss.Color("#0A0A0A"),
		ModalBg:   lipgloss.Color("#141414"),
		ModalFg:   lipgloss.Color("#E6E6E6"),
		Border:    lipgloss.Color("#5C5C5C"),

		HeadingLineBg: lipgloss.Color("#202020"),
		HeadingLineFg: lipgloss.Color("#F2F2F2"),

		Accent: lipgloss.Color("#8AB4F8"),

		SelectionBg: lipgloss.Color("#303030"),
		SelectionFg: lipgloss.Color("#F2F2F2"),

		ScrollbarTrack: lipgloss.Color("#3D3D3D"),
		ScrollbarThumb: lipgloss.Color("#B8B8B8"),

		MarkerDim: lipgloss.Color("#7A7A7A"),
	}

	return Theme{
		Mode:   themeDark,
		Colors: c,
		Styles: buildThemeStyles(c, themeDark),
	}
}

func newLightTheme() Theme {
	c := ThemeColors{
		HeaderBg:      lipgloss.Color("#EFEFEF"),
		HeaderFg:      lipgloss.Color("#1A1A1A"),
		HeaderFgMuted: lipgloss.Color("#444444"),

		FooterBg: lipgloss.Color("#F5F5F5"),
		FooterFg: lipgloss.Color("#444444"),

		OverlayBg: lipgloss.Color("#EDEDED"),
		ModalBg:   lipgloss.Color("#F8F8F8"),
		ModalFg:   lipgloss.Color("#1A1A1A"),
		Border:    lipgloss.Color("#B0B0B0"),

		HeadingLineBg: lipgloss.Color("#EAEAEA"),
		HeadingLineFg: lipgloss.Color("#1A1A1A"),

		Accent: lipgloss.Color("#2563EB"),

		SelectionBg: lipgloss.Color("#2563EB"),
		SelectionFg: lipgloss.Color("#F8F8F8"),

		ScrollbarTrack: lipgloss.Color("#C9C9C9"),
		ScrollbarThumb: lipgloss.Color("#666666"),

		MarkerDim: lipgloss.Color("#888888"),
	}

	return Theme{
		Mode:   themeLight,
		Colors: c,
		Styles: buildThemeStyles(c, themeLight),
	}
}

func buildThemeStyles(c ThemeColors, mode themeMode) ThemeStyles {
	headerLeft := lipgloss.NewStyle().
		Bold(true).
		Foreground(c.HeaderFg).
		Background(c.HeaderBg).
		Padding(0, 1)

	headerRight := lipgloss.NewStyle().
		Foreground(c.HeaderFgMuted).
		Background(c.HeaderBg).
		Padding(0, 1)

	footer := lipgloss.NewStyle().
		Foreground(c.FooterFg).
		Background(c.FooterBg).
		Padding(0, 1)

	headingLine := lipgloss.NewStyle().
		Background(c.HeadingLineBg).
		Foreground(c.HeadingLineFg).
		Bold(true)

	box := lipgloss.NewStyle().
		Border(lipgloss.RoundedBorder()).
		BorderForeground(c.Border).
		Padding(1, 2).
		Background(c.ModalBg).
		Foreground(c.ModalFg)

	tocTitle := lipgloss.NewStyle().
		Bold(true).
		Foreground(c.HeaderFg).
		Background(c.HeaderBg).
		Padding(0, 1)

	tocFilter := lipgloss.NewStyle().
		Foreground(c.HeaderFgMuted).
		Background(c.HeaderBg).
		Padding(0, 1)

	tocFooter := lipgloss.NewStyle().
		Foreground(c.FooterFg).
		Background(c.FooterBg).
		Padding(0, 1)

	tocBox := lipgloss.NewStyle().
		Border(lipgloss.RoundedBorder()).
		BorderForeground(c.Border).
		Padding(0, 0).
		Background(c.ModalBg)

	tocSel := lipgloss.NewStyle().
		Bold(true).
		Foreground(c.SelectionFg).
		Background(c.SelectionBg).
		Padding(0, 1)

	tocNorm := lipgloss.NewStyle().
		Foreground(c.ModalFg).
		Padding(0, 1)

	markerHeading := lipgloss.NewStyle().Foreground(c.Accent)
	markerCurrent := lipgloss.NewStyle().Bold(true).Foreground(c.Accent)
	markerOther := lipgloss.NewStyle().Foreground(c.Accent)
	markerNone := lipgloss.NewStyle().Foreground(c.MarkerDim)

	scrollTrack := lipgloss.NewStyle().Foreground(c.ScrollbarTrack)
	scrollThumb := lipgloss.NewStyle().Foreground(c.ScrollbarThumb)

	// In light mode, a bit of distinction helps the modal content separate from the terminal background.
	if mode == themeLight {
		tocNorm = tocNorm.Foreground(c.HeaderFg)
	}

	return ThemeStyles{
		HeaderLeft:  headerLeft,
		HeaderRight: headerRight,
		Footer:      footer,
		HeadingLine: headingLine,
		HelpBox:     box,

		TOCTitle:  tocTitle,
		TOCFilter: tocFilter,
		TOCFooter: tocFooter,
		TOCBox:    tocBox,

		TOCItemSelected: tocSel,
		TOCItemNormal:   tocNorm,

		MarkerHeading:      markerHeading,
		MarkerMatchCurrent: markerCurrent,
		MarkerMatchOther:   markerOther,
		MarkerNone:         markerNone,

		ScrollbarTrack: scrollTrack,
		ScrollbarThumb: scrollThumb,
	}
}
