use std::io::{self, IsTerminal, Write};

use crate::input::{detect_terminal_width, parse_pager_mode, resolve_source};
use crate::render::{render_markdown, RenderOptions};
use crate::tui::view_markdown;

pub struct Options {
    pub style: String,
    pub width: usize,
    pub pager: String,
    pub args: Vec<String>,
}

pub fn run(opts: Options) -> Result<(), String> {
    let source = resolve_source(&opts.args)?;
    let source = source
        .ok_or_else(|| "no input: provide a file path or pipe markdown via stdin".to_string())?;
    let markdown = source.read_all()?;
    let pager_mode = parse_pager_mode(&opts.pager)?;
    let stdout_is_tty = io::stdout().is_terminal();

    if pager_mode.should_use_pager(stdout_is_tty) {
        let title = if source.title().is_empty() {
            "md".to_string()
        } else {
            source.title()
        };
        return view_markdown(
            &title,
            &markdown,
            RenderOptions {
                style: opts.style,
                width: opts.width,
            },
        );
    }

    let width = if opts.width == 0 {
        detect_terminal_width(80)
    } else {
        opts.width
    };
    let out = render_markdown(
        &markdown,
        RenderOptions {
            style: opts.style,
            width,
        },
    )?;
    io::stdout()
        .write_all(out.as_bytes())
        .map_err(|err| format!("write stdout: {err}"))?;
    Ok(())
}
