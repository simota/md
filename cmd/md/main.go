package main

import (
	"flag"
	"fmt"
	"os"

	"github.com/simota/md/internal/app"
)

func main() {
	var (
		style       string
		width       int
		pager       string
		pagerAlways bool
	)

	flag.StringVar(&style, "style", "auto", "render style: auto|dark|light")
	flag.StringVar(&style, "s", "auto", "alias for --style")
	flag.IntVar(&width, "width", 0, "render width (0 = auto)")
	flag.IntVar(&width, "w", 0, "alias for --width")
	flag.StringVar(&pager, "pager", "never", "pager mode: auto|always|never")
	flag.BoolVar(&pagerAlways, "p", false, "open interactive pager (same as --pager=always)")

	flag.Usage = func() {
		out := flag.CommandLine.Output()

		fmt.Fprintf(out, "Usage: %s [options] [file|-]\n\n", os.Args[0])
		fmt.Fprintln(out, "Options:")
		fmt.Fprintln(out, "  -p             open interactive pager (TUI)")
		fmt.Fprintln(out, "  -s, --style    auto|dark|light (default: auto)")
		fmt.Fprintln(out, "")
		fmt.Fprintln(out, "Advanced:")
		fmt.Fprintln(out, "  --pager        auto|always|never (default: never)")
		fmt.Fprintln(out, "  -w, --width    render width (0 = auto)")
		fmt.Fprintln(flag.CommandLine.Output(), "\nExamples:")
		fmt.Fprintf(out, "  %s README.md\n", os.Args[0])
		fmt.Fprintf(out, "  %s -p README.md\n", os.Args[0])
		fmt.Fprintf(out, "  cat README.md | %s\n", os.Args[0])
	}
	flag.Parse()

	// -p is the primary "simple" flag. If the user also specified --pager explicitly,
	// respect --pager.
	visited := map[string]bool{}
	flag.Visit(func(f *flag.Flag) { visited[f.Name] = true })
	if pagerAlways && !visited["pager"] {
		pager = "always"
	}

	opts := app.Options{
		Style:  style,
		Width:  width,
		Pager:  pager,
		Args:   flag.Args(),
		Stdin:  os.Stdin,
		Stdout: os.Stdout,
		Stderr: os.Stderr,
	}

	if err := app.Run(opts); err != nil {
		fmt.Fprintln(os.Stderr, err.Error())
		os.Exit(1)
	}
}
