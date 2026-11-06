use std::fs;
use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};

use crossterm::terminal;

pub enum Source {
    File(String),
    Stdin,
}

impl Source {
    pub fn title(&self) -> String {
        match self {
            Source::File(path) => Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(path)
                .to_string(),
            Source::Stdin => "stdin".to_string(),
        }
    }

    pub fn read_all(&self) -> Result<String, String> {
        match self {
            Source::File(path) => {
                let bytes = fs::read(path).map_err(|err| format!("read file {path:?}: {err}"))?;
                Ok(String::from_utf8_lossy(&bytes).into_owned())
            }
            Source::Stdin => {
                let mut buf = Vec::new();
                io::stdin()
                    .read_to_end(&mut buf)
                    .map_err(|err| format!("read stdin: {err}"))?;
                Ok(String::from_utf8_lossy(&buf).into_owned())
            }
        }
    }
}

/// The documents selected on the command line.
pub struct Sources {
    pub items: Vec<Source>,
    /// At least one argument was a directory (expanded to its Markdown files).
    pub from_directory: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PagerMode {
    Auto,
    Always,
    Never,
}

impl PagerMode {
    pub fn should_use_pager(self, stdout_is_tty: bool) -> bool {
        match self {
            PagerMode::Auto => stdout_is_tty,
            PagerMode::Always => true,
            PagerMode::Never => false,
        }
    }
}

pub fn parse_pager_mode(value: &str) -> Result<PagerMode, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "auto" => Ok(PagerMode::Auto),
        "always" => Ok(PagerMode::Always),
        "never" => Ok(PagerMode::Never),
        _ => Err(format!("invalid --pager={value:?} (use auto|always|never)")),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

pub fn parse_color_mode(value: &str) -> Result<ColorMode, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "auto" => Ok(ColorMode::Auto),
        "always" => Ok(ColorMode::Always),
        "never" => Ok(ColorMode::Never),
        _ => Err(format!("invalid --color={value:?} (use auto|always|never)")),
    }
}

impl ColorMode {
    /// `auto` keeps colors (also when piped, e.g. into `less -R`) unless
    /// `NO_COLOR` is set to a non-empty value or `TERM=dumb`.
    pub fn enabled(self, no_color: Option<&str>, term: Option<&str>) -> bool {
        match self {
            ColorMode::Always => true,
            ColorMode::Never => false,
            ColorMode::Auto => no_color.map_or(true, str::is_empty) && term != Some("dumb"),
        }
    }
}

/// Backwards-compatible single-source resolution.
pub fn resolve_source(args: &[String]) -> Result<Option<Source>, String> {
    if args.len() > 1 {
        return Err("too many arguments: provide at most one file path".to_string());
    }
    Ok(resolve_sources(args)?.items.into_iter().next())
}

/// Resolve CLI arguments into documents. Directories expand to the Markdown
/// files they contain (recursively, sorted, README first).
pub fn resolve_sources(args: &[String]) -> Result<Sources, String> {
    let mut items = Vec::new();
    let mut from_directory = false;
    let mut stdin_used = false;
    for arg in args {
        if arg == "-" {
            if stdin_used {
                return Err("stdin (-) can only be given once".to_string());
            }
            stdin_used = true;
            items.push(Source::Stdin);
            continue;
        }
        let path = Path::new(arg);
        if path.is_dir() {
            from_directory = true;
            let files = collect_markdown_files(path);
            if files.is_empty() {
                return Err(format!("no Markdown files found in {arg:?}"));
            }
            items.extend(
                files
                    .into_iter()
                    .map(|p| Source::File(p.to_string_lossy().into_owned())),
            );
        } else {
            items.push(Source::File(arg.clone()));
        }
    }
    if items.is_empty() && !io::stdin().is_terminal() {
        items.push(Source::Stdin);
    }
    Ok(Sources {
        items,
        from_directory,
    })
}

const MARKDOWN_EXTENSIONS: &[&str] = &["md", "markdown", "mdown", "mkd", "mkdn", "mdx"];
const SKIP_DIRS: &[&str] = &["node_modules", "target", "vendor", "dist", "build"];
const MAX_DEPTH: usize = 8;
const MAX_FILES: usize = 5000;

pub fn is_markdown_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            MARKDOWN_EXTENSIONS
                .iter()
                .any(|known| known.eq_ignore_ascii_case(ext))
        })
}

pub fn collect_markdown_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(dir, 0, &mut out);
    out
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH || out.len() >= MAX_FILES {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) {
                dirs.push(path);
            }
        } else if is_markdown_path(&path) {
            files.push(path);
        }
    }
    files.sort_by_key(|p| {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        (!name.starts_with("readme"), name)
    });
    dirs.sort();
    for file in files {
        if out.len() >= MAX_FILES {
            return;
        }
        out.push(file);
    }
    for sub in dirs {
        walk(&sub, depth + 1, out);
    }
}

pub fn detect_terminal_width(fallback: usize) -> usize {
    match terminal::size() {
        Ok((w, _)) if w > 4 => usize::from(w - 2),
        Ok((w, _)) if w > 0 => usize::from(w),
        _ => fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pager_modes() {
        assert_eq!(parse_pager_mode("auto").unwrap(), PagerMode::Auto);
        assert_eq!(parse_pager_mode("always").unwrap(), PagerMode::Always);
        assert_eq!(parse_pager_mode("never").unwrap(), PagerMode::Never);
        assert!(parse_pager_mode("sometimes").is_err());
    }

    #[test]
    fn resolve_source_file() {
        let args = vec!["README.md".to_string()];
        let src = resolve_source(&args).unwrap().unwrap();
        assert_eq!(src.title(), "README.md");
    }

    #[test]
    fn color_mode_respects_no_color() {
        assert!(ColorMode::Auto.enabled(None, Some("xterm")));
        assert!(!ColorMode::Auto.enabled(Some("1"), Some("xterm")));
        assert!(ColorMode::Auto.enabled(Some(""), Some("xterm")));
        assert!(!ColorMode::Auto.enabled(None, Some("dumb")));
        assert!(ColorMode::Always.enabled(Some("1"), Some("dumb")));
        assert!(!ColorMode::Never.enabled(None, None));
        assert!(parse_color_mode("bogus").is_err());
    }

    #[test]
    fn directories_expand_to_markdown_files() {
        let dir = std::env::temp_dir().join(format!("md-input-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("guide")).unwrap();
        fs::create_dir_all(dir.join("node_modules/pkg")).unwrap();
        fs::create_dir_all(dir.join(".hidden")).unwrap();
        for f in [
            "b.md",
            "README.md",
            "notes.txt",
            "guide/intro.markdown",
            "node_modules/pkg/x.md",
            ".hidden/y.md",
        ] {
            fs::write(dir.join(f), "# x\n").unwrap();
        }
        let files: Vec<String> = collect_markdown_files(&dir)
            .iter()
            .map(|p| {
                p.strip_prefix(&dir)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        assert_eq!(files, vec!["README.md", "b.md", "guide/intro.markdown"]);

        let args = vec![dir.to_string_lossy().into_owned()];
        let sources = resolve_sources(&args).unwrap();
        assert!(sources.from_directory);
        assert_eq!(sources.items.len(), 3);
        let _ = fs::remove_dir_all(&dir);
    }
}
