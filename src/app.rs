use std::env;
use std::io::{self, IsTerminal, Write};

use crate::input::{
    detect_terminal_width, parse_color_mode, parse_pager_mode, resolve_sources, Source,
};
use crate::render::{render_document, strip_ansi, RenderOptions, Rendered};
use crate::tui::{view_documents, DocSource, ViewConfig};

pub struct Options {
    pub style: String,
    pub width: usize,
    pub pager: String,
    pub args: Vec<String>,
    /// auto|always|never
    pub color: String,
    /// Only show the section under this heading (text or anchor).
    pub section: Option<String>,
    /// Print the document outline instead of the document.
    pub toc: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            style: "auto".to_string(),
            width: 0,
            pager: "never".to_string(),
            args: Vec::new(),
            color: "auto".to_string(),
            section: None,
            toc: false,
        }
    }
}

pub fn run(opts: Options) -> Result<(), String> {
    let sources = resolve_sources(&opts.args)?;
    if sources.items.is_empty() {
        return Err("no input: provide a file path or pipe markdown via stdin".to_string());
    }
    let pager_mode = parse_pager_mode(&opts.pager)?;
    let color = parse_color_mode(&opts.color)?.enabled(
        env::var("NO_COLOR").ok().as_deref(),
        env::var("TERM").ok().as_deref(),
    );
    let stdout_is_tty = io::stdout().is_terminal();

    if !opts.toc && pager_mode.should_use_pager(stdout_is_tty) {
        let mut docs = Vec::with_capacity(sources.items.len());
        for source in &sources.items {
            docs.push(match source {
                Source::File(path) => DocSource::Path(path.into()),
                Source::Stdin => DocSource::Text {
                    title: source.title(),
                    markdown: source.read_all()?,
                },
            });
        }
        return view_documents(
            docs,
            ViewConfig {
                render: RenderOptions {
                    style: opts.style,
                    width: opts.width,
                },
                section: opts.section,
                show_files: sources.from_directory,
            },
        );
    }

    let width = if opts.width == 0 {
        detect_terminal_width(80)
    } else {
        opts.width
    };
    let render_opts = RenderOptions {
        style: opts.style,
        width,
    };
    let multiple = sources.items.len() > 1;
    let mut out = String::new();
    let mut matched_section = false;
    let mut printed_any = false;

    for source in &sources.items {
        let markdown = source.read_all()?;
        let rendered = render_document(&markdown, &render_opts)?;
        let body = if opts.toc {
            outline(&rendered)
        } else if let Some(query) = opts.section.as_deref() {
            match rendered
                .find_heading(query)
                .and_then(|idx| rendered.section_range(idx))
            {
                Some((start, end)) => {
                    matched_section = true;
                    join_lines(&rendered.lines[start..end])
                }
                None => continue,
            }
        } else {
            rendered.text()
        };

        if multiple {
            if printed_any {
                out.push('\n');
            }
            let name = match source {
                Source::File(path) => path.clone(),
                Source::Stdin => source.title(),
            };
            out.push_str(&format!("\x1b[1m==> {name} <==\x1b[0m\n"));
        }
        out.push_str(&body);
        printed_any = true;
    }

    if let Some(query) = opts.section.as_deref() {
        if !matched_section {
            return Err(format!(
                "section {query:?} not found (use --toc to list headings)"
            ));
        }
    }

    let out = if color { out } else { strip_ansi(&out) };
    let mut stdout = io::stdout();
    match stdout
        .write_all(out.as_bytes())
        .and_then(|_| stdout.flush())
    {
        Ok(()) => Ok(()),
        // `md file | head` closes the pipe early; that is not an error.
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(err) => Err(format!("write stdout: {err}")),
    }
}

fn join_lines(lines: &[String]) -> String {
    let mut out = String::new();
    for line in lines {
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Indented heading outline with anchors, e.g. for `--toc`.
fn outline(rendered: &Rendered) -> String {
    let mut out = String::new();
    for h in &rendered.headings {
        out.push_str(&"  ".repeat(h.level.saturating_sub(1)));
        out.push_str(&format!("\x1b[1m{}\x1b[0m", h.text));
        out.push_str(&format!("  \x1b[2m#{}\x1b[0m\n", h.anchor));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outline_lists_headings_with_anchors() {
        let r = render_document(
            "# Title\n\n## Install it\n\ntext\n",
            &RenderOptions {
                style: "dark".to_string(),
                width: 60,
            },
        )
        .unwrap();
        assert_eq!(
            strip_ansi(&outline(&r)),
            "Title  #title\n  Install it  #install-it\n"
        );
    }
}
