package render

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/glamour"
	"github.com/charmbracelet/glamour/ansi"
	"github.com/charmbracelet/glamour/styles"
	"github.com/muesli/termenv"
)

type Options struct {
	Style string // auto|dark|light
	Width int
}

func strPtr(s string) *string { return &s }
func boolPtr(b bool) *bool    { return &b }

func editorialStyleConfig(style string) (ansi.StyleConfig, error) {
	switch strings.ToLower(strings.TrimSpace(style)) {
	case "dark":
		return editorialDark(), nil
	case "light":
		return editorialLight(), nil
	case "", "auto":
		if termenv.HasDarkBackground() {
			return editorialDark(), nil
		}
		return editorialLight(), nil
	default:
		return ansi.StyleConfig{}, fmt.Errorf("invalid --style=%q (use auto|dark|light)", style)
	}
}

func editorialDark() ansi.StyleConfig {
	cfg := styles.DarkStyleConfig

	// Editorial Minimal: monochrome base + one accent.
	accent := "#8AB4F8"
	fg := "#E6E6E6"
	muted := "#B8B8B8"
	codeBg := "#141414"

	cfg.Document.StylePrimitive.Color = strPtr(fg)
	cfg.Heading.StylePrimitive.Color = strPtr(accent)
	cfg.Heading.StylePrimitive.Bold = boolPtr(true)

	// Remove the "banner" H1 background from the default style; keep it calm.
	cfg.H1.StylePrimitive.BackgroundColor = nil
	cfg.H1.StylePrimitive.Color = strPtr(accent)
	cfg.H1.StylePrimitive.Bold = boolPtr(true)

	// Slightly dim the lowest heading level so hierarchy is visible without extra colors.
	cfg.H6.StylePrimitive.Color = strPtr(muted)
	cfg.H6.StylePrimitive.Bold = boolPtr(false)

	cfg.Link.Color = strPtr(accent)
	cfg.Link.Underline = boolPtr(true)
	cfg.LinkText.Color = strPtr(accent)
	cfg.LinkText.Bold = boolPtr(true)

	// Inline code: subtle background, readable foreground.
	cfg.Code.StylePrimitive.Color = strPtr(fg)
	cfg.Code.StylePrimitive.BackgroundColor = strPtr(codeBg)
	cfg.Code.StylePrimitive.Prefix = " "
	cfg.Code.StylePrimitive.Suffix = " "

	// Code blocks: keep syntax colors, but reduce surrounding noise.
	cfg.CodeBlock.StyleBlock.StylePrimitive.Color = strPtr(muted)
	if cfg.CodeBlock.Chroma != nil {
		cfg.CodeBlock.Chroma.Text.Color = strPtr(fg)
		cfg.CodeBlock.Chroma.Background.BackgroundColor = strPtr(codeBg)
	}

	return cfg
}

func editorialLight() ansi.StyleConfig {
	cfg := styles.LightStyleConfig

	accent := "#2563EB"
	fg := "#1A1A1A"
	muted := "#444444"
	codeBg := "#F2F2F2"

	cfg.Document.StylePrimitive.Color = strPtr(fg)
	cfg.Heading.StylePrimitive.Color = strPtr(accent)
	cfg.Heading.StylePrimitive.Bold = boolPtr(true)

	cfg.H1.StylePrimitive.BackgroundColor = nil
	cfg.H1.StylePrimitive.Color = strPtr(accent)
	cfg.H1.StylePrimitive.Bold = boolPtr(true)

	cfg.H6.StylePrimitive.Color = strPtr(muted)
	cfg.H6.StylePrimitive.Bold = boolPtr(false)

	cfg.Link.Color = strPtr(accent)
	cfg.Link.Underline = boolPtr(true)
	cfg.LinkText.Color = strPtr(accent)
	cfg.LinkText.Bold = boolPtr(true)

	cfg.Code.StylePrimitive.Color = strPtr(fg)
	cfg.Code.StylePrimitive.BackgroundColor = strPtr(codeBg)
	cfg.Code.StylePrimitive.Prefix = " "
	cfg.Code.StylePrimitive.Suffix = " "

	cfg.CodeBlock.StyleBlock.StylePrimitive.Color = strPtr(muted)
	if cfg.CodeBlock.Chroma != nil {
		cfg.CodeBlock.Chroma.Text.Color = strPtr(fg)
		cfg.CodeBlock.Chroma.Background.BackgroundColor = strPtr(codeBg)
	}

	return cfg
}

func RenderMarkdown(md string, opts Options) (string, error) {
	w := opts.Width
	if w <= 0 {
		w = 80
	}

	var renderer *glamour.TermRenderer
	var err error

	cfg, cfgErr := editorialStyleConfig(opts.Style)
	if cfgErr != nil {
		return "", cfgErr
	}
	renderer, err = glamour.NewTermRenderer(
		glamour.WithStyles(cfg),
		glamour.WithWordWrap(w),
	)
	if err != nil {
		return "", fmt.Errorf("init renderer: %w", err)
	}

	out, err := renderer.Render(md)
	if err != nil {
		return "", fmt.Errorf("render markdown: %w", err)
	}
	return out, nil
}
