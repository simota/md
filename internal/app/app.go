package app

import (
	"errors"
	"fmt"
	"io"
	"os"

	"github.com/simota/md/internal/input"
	"github.com/simota/md/internal/render"
	"github.com/simota/md/internal/tui"
)

type Options struct {
	Style  string
	Width  int
	Pager  string
	Args   []string
	Stdin  *os.File
	Stdout *os.File
	Stderr *os.File
}

func Run(opts Options) error {
	if opts.Stdout == nil || opts.Stderr == nil || opts.Stdin == nil {
		return errors.New("internal error: stdio is nil")
	}

	src, err := input.ResolveSource(opts.Args, opts.Stdin)
	if err != nil {
		return err
	}
	if src == nil {
		return errors.New("no input: provide a file path or pipe markdown via stdin")
	}

	md, err := src.ReadAll()
	if err != nil {
		return err
	}

	pagerMode, err := input.ParsePagerMode(opts.Pager)
	if err != nil {
		return err
	}

	// If stdout is not a TTY, avoid interactive pager (print-only).
	stdoutIsTTY := input.IsTerminal(opts.Stdout)
	usePager := pagerMode.ShouldUsePager(stdoutIsTTY)

	if usePager {
		title := src.Title()
		if title == "" {
			title = "md"
		}
		return tui.ViewMarkdown(title, string(md), render.Options{
			Style: opts.Style,
			Width: opts.Width, // 0 means auto; TUI will choose based on window size.
		}, opts.Stdout)
	}

	w := opts.Width
	if w <= 0 {
		w = input.DetectTerminalWidth(opts.Stdout, 80)
	}

	out, err := render.RenderMarkdown(string(md), render.Options{
		Style: opts.Style,
		Width: w,
	})
	if err != nil {
		return err
	}

	if _, err := io.WriteString(opts.Stdout, out); err != nil {
		return fmt.Errorf("write stdout: %w", err)
	}
	return nil
}
