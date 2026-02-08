# md

Terminal Markdown viewer written in Go. Single-binary friendly.

## Install

```bash
go install github.com/simota/md/cmd/md@latest
```

## Install (binaries from GitHub Releases)

If you don't want to install Go, download a prebuilt binary from GitHub Releases.

Release assets are named like:

- `md_<VERSION>_<GOOS>_<ARCH>.tar.gz` (Linux/macOS)
- `md_<VERSION>_<GOOS>_<ARCH>.zip` (Windows)
- `md_<VERSION>_checksums.txt` (SHA-256 checksums for all assets)

### macOS / Linux

```bash
# Pick a version and platform.
VERSION=v0.1.0
GOOS=darwin   # or linux
ARCH=arm64    # or amd64, armv7

ASSET="md_${VERSION}_${GOOS}_${ARCH}.tar.gz"

curl -fL -o "${ASSET}" "https://github.com/simota/md/releases/download/${VERSION}/${ASSET}"
curl -fL -o "md_${VERSION}_checksums.txt" "https://github.com/simota/md/releases/download/${VERSION}/md_${VERSION}_checksums.txt"

# Verify checksum (pick one depending on your OS).
grep " ${ASSET}\$" "md_${VERSION}_checksums.txt" | sha256sum -c -  # Linux
# grep " ${ASSET}\$" "md_${VERSION}_checksums.txt" | shasum -a 256 -c -  # macOS

tar -xzf "${ASSET}"

# Install (no sudo): make sure ~/.local/bin is in PATH
mkdir -p "${HOME}/.local/bin"
install -m 0755 md "${HOME}/.local/bin/md"

# Or install system-wide:
# sudo install -m 0755 md /usr/local/bin/md
```

### Windows

1. Download the matching `md_<VERSION>_windows_<ARCH>.zip` from GitHub Releases.
2. Extract `md.exe`.
3. Put it somewhere in your `PATH` (e.g. `C:\Users\<you>\bin`) and restart your terminal.

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
