package input

import (
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"

	"golang.org/x/term"
)

type Source interface {
	Title() string
	ReadAll() ([]byte, error)
}

type fileSource struct {
	path string
}

func (s fileSource) Title() string { return filepath.Base(s.path) }

func (s fileSource) ReadAll() ([]byte, error) {
	b, err := os.ReadFile(s.path)
	if err != nil {
		return nil, fmt.Errorf("read file %q: %w", s.path, err)
	}
	return b, nil
}

type stdinSource struct {
	r io.Reader
}

func (s stdinSource) Title() string { return "stdin" }

func (s stdinSource) ReadAll() ([]byte, error) {
	b, err := io.ReadAll(s.r)
	if err != nil {
		return nil, fmt.Errorf("read stdin: %w", err)
	}
	return b, nil
}

func ResolveSource(args []string, stdin *os.File) (Source, error) {
	if len(args) > 1 {
		return nil, errors.New("too many arguments: provide at most one file path")
	}
	if len(args) == 1 {
		if args[0] == "-" {
			if stdin == nil {
				return nil, errors.New("stdin is not available")
			}
			return stdinSource{r: stdin}, nil
		}
		return fileSource{path: args[0]}, nil
	}

	// No args: read from stdin only when it is not a terminal.
	if stdin != nil && !term.IsTerminal(int(stdin.Fd())) {
		return stdinSource{r: stdin}, nil
	}
	return nil, nil
}

func IsTerminal(f *os.File) bool {
	if f == nil {
		return false
	}
	return term.IsTerminal(int(f.Fd()))
}

func DetectTerminalWidth(stdout *os.File, fallback int) int {
	if stdout == nil {
		return fallback
	}
	w, _, err := term.GetSize(int(stdout.Fd()))
	if err != nil || w <= 0 {
		return fallback
	}
	// Keep a bit of margin to avoid hard wrap on right edge.
	if w > 4 {
		return w - 2
	}
	return w
}
