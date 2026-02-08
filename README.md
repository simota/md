# md

Terminal Markdown viewer written in Go. Single-binary friendly.

## Install

```bash
go install github.com/simota/md/cmd/md@latest
```

## Usage

```bash
# View a file (print-only by default)
md README.md

# Render from stdin (also print-only)
cat README.md | md

# Explicitly read from stdin
cat README.md | md -

# Open TUI pager (simple)
md -p README.md

# Force print-only mode (no TUI pager)
md --pager=never README.md

# Auto pager: use TUI only when stdout is a TTY
md --pager=auto README.md
```

## Flags

- `-p` : open TUI pager (same as `--pager=always`)
- `-s`, `--style` : `auto|dark|light` (default: `auto`)
- `--pager` : `auto|always|never` (default: `never`) (advanced)
- `-w`, `--width` : render width (default: auto-detect terminal width; fallback 80) (advanced)

## Notes

- Current dependencies require Go `>= 1.24.2`. The `go.mod` includes a `toolchain` directive so builds can auto-fetch a compatible toolchain.
- TUI pager keybinds: `j/k` or arrow keys, `PgUp/PgDn`, `u/d` (half page), `g/G`, `q`/`Esc`, `?` (help), mouse wheel.
- Extra navigation: `/` (search), `n/N` (next/prev match), `c` (clear search), `t` (TOC).
- Section navigation: `[` / `]` (prev/next heading).
- Outline: `1-6` (fold by heading level), `0` (show all).
- In TOC, press `/` to filter headings.
- When outline is active, the header shows `H{level}` and the footer shows both `doc` and `ol` ranges.

## Release

This repo ships binaries via GitHub Actions. Pushing a `v*` tag builds and uploads
artifacts for Linux/macOS/Windows (multi-arch).

```bash
git tag v0.1.0
git push origin v0.1.0
```

## Build (from source)

```bash
make test
make build
./md README.md

# Optional: create local release artifacts (tar.gz/zip + checksums)
make dist VERSION=v0.1.0
```
