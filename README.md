# md

Terminal Markdown viewer written in Rust. Single-binary friendly.

## Install

```bash
cargo install --git https://github.com/simota/md md
```

## Install (binaries from GitHub Releases)

If you don't want to install Rust, download a prebuilt binary from GitHub Releases.

Release assets are named like:

- `md_<VERSION>_<GOOS>_<ARCH>.tar.gz` (Linux/macOS)
- `md_<VERSION>_<GOOS>_<ARCH>.zip` (Windows)
- `md_<VERSION>_checksums.txt` (SHA-256 checksums for all assets)

### macOS / Linux

```bash
# Pick a version and platform.
VERSION=v0.1.0
GOOS=darwin   # or linux
ARCH=arm64    # or amd64

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

# Browse a folder of docs (opens a file picker; Tab/Shift-Tab switch files)
md -p docs/

# Several files at once
md -p README.md CHANGELOG.md

# Print only one section (heading text, substring, or #anchor)
md -S install README.md
md -S '#getting-started' README.md

# Open the pager directly at a section
md -p -S usage README.md

# List the headings (with anchors) of a document
md --toc README.md

# Plain text without colors (also honors NO_COLOR)
md --color=never README.md > README.txt

# Force print-only mode (no TUI pager)
md --pager=never README.md

# Auto pager: use TUI only when stdout is a TTY
md --pager=auto README.md
```

## Flags

- `-p` : open TUI pager (same as `--pager=always`)
- `-s`, `--style` : `auto|dark|light` (default: `auto`; `auto` reads `COLORFGBG` to detect light terminals)
- `-S`, `--section` : show only the section under a heading (exit code 1 if not found)
- `--toc` : print the heading outline with anchors
- `--color` : `auto|always|never` (default: `auto` = colors unless `NO_COLOR` is set or `TERM=dumb`); `--no-color` = `--color=never`
- `--pager` : `auto|always|never` (default: `never`) (advanced)
- `-w`, `--width` : render width (default: auto-detect terminal width; fallback 80) (advanced)

Arguments may be files, directories (expanded recursively to `*.md`, `*.markdown`, ...; hidden
directories and `node_modules`/`target`/`vendor`/`dist`/`build` are skipped), or `-` for stdin.

## Rendering

GitHub-flavored Markdown: tables (with alignment, cells wrap to fit), nested lists and quotes,
task lists, footnotes, GitHub alerts (`> [!NOTE]`, `[!WARNING]`, ...), strikethrough, italics,
link URLs, image alt text, and syntax highlighting for shell, Rust, Go, Python, JS/TS, C-family,
Ruby, SQL, JSON/YAML/TOML and diffs. Text wraps by display width, including CJK text without
spaces (with basic kinsoku rules). Control characters in documents are shown as `^[` etc. so a
file cannot inject terminal escape sequences.

## Pager keys

| Keys | Action |
|---|---|
| `j/k`, arrows, mouse wheel | scroll |
| `Space`/`f`, `b`, `PgDn/PgUp`, `d/u` | page / half page |
| `g/G` | top / bottom |
| `[` / `]` | previous / next heading |
| `1-6` / `0` | fold outline by heading level / show all |
| `/`, `n/N`, `c` | search (matches are highlighted; smart case), next/prev, clear |
| `t` | table of contents (`/` to filter) |
| `l` | link list: `Enter` follows `#anchors` and relative Markdown links, opens web URLs and images in the system viewer; `y` copies the URL |
| `Backspace` | back to the previous document |
| `Tab` / `Shift-Tab`, `F` | next / previous file, file picker |
| `y` | copy the code block on screen to the clipboard (press again for the next one) |
| `r` | reload (the pager also reloads automatically when the file changes) |
| `?` | help |
| `q` / `Esc` | quit |

Notes:

- Current dependencies require Rust `>= 1.80`.
- Clipboard copy uses OSC 52 (supported by most modern terminals; inside tmux enable `set -g allow-passthrough on` or `set -g set-clipboard on`).
- Opening external links uses `open` (macOS), `xdg-open` (Linux) or the URL handler (Windows). Only `http(s)`/`mailto` URLs and image/PDF/text files are opened.
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
./target/release/md README.md

# Optional: create local release artifacts (tar.gz/zip + checksums)
make dist VERSION=v0.1.0
```
