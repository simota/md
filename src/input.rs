use std::fs;
use std::io::{self, IsTerminal, Read};
use std::path::Path;

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
                fs::read_to_string(path).map_err(|err| format!("read file {path:?}: {err}"))
            }
            Source::Stdin => {
                let mut buf = String::new();
                io::stdin()
                    .read_to_string(&mut buf)
                    .map_err(|err| format!("read stdin: {err}"))?;
                Ok(buf)
            }
        }
    }
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

pub fn resolve_source(args: &[String]) -> Result<Option<Source>, String> {
    if args.len() > 1 {
        return Err("too many arguments: provide at most one file path".to_string());
    }
    if let Some(arg) = args.first() {
        if arg == "-" {
            return Ok(Some(Source::Stdin));
        }
        return Ok(Some(Source::File(arg.clone())));
    }
    if !io::stdin().is_terminal() {
        return Ok(Some(Source::Stdin));
    }
    Ok(None)
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
}
