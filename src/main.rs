use std::env;
use std::io::{self, Write};
use std::process;

use md::app::{run, Options};

fn main() {
    let opts = match parse_args(env::args().collect()) {
        Ok(opts) => opts,
        Err(err) if err == "__help__" => {
            print_usage();
            return;
        }
        Err(err) => {
            eprintln!("{err}");
            process::exit(2);
        }
    };

    if let Err(err) = run(opts) {
        eprintln!("{err}");
        process::exit(1);
    }
}

fn parse_args(args: Vec<String>) -> Result<Options, String> {
    let mut style = "auto".to_string();
    let mut width = 0usize;
    let mut pager = "never".to_string();
    let mut pager_always = false;
    let mut color = "auto".to_string();
    let mut section = None;
    let mut toc = false;
    let mut positional = Vec::new();
    let mut only_positional = false;

    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        if only_positional {
            positional.push(arg.clone());
            i += 1;
            continue;
        }
        match arg.as_str() {
            "--" => only_positional = true,
            "--toc" => toc = true,
            "--no-color" => color = "never".to_string(),
            "--color" => {
                i += 1;
                color = args.get(i).ok_or("missing value for --color")?.clone();
            }
            "-S" | "--section" => {
                i += 1;
                section = Some(args.get(i).ok_or("missing value for --section")?.clone());
            }
            "-h" | "--help" => return Err("__help__".to_string()),
            "-p" => pager_always = true,
            "-s" | "--style" => {
                i += 1;
                style = args.get(i).ok_or("missing value for --style")?.clone();
            }
            "-w" | "--width" => {
                i += 1;
                width = parse_width(args.get(i).ok_or("missing value for --width")?)?;
            }
            "--pager" => {
                i += 1;
                pager = args.get(i).ok_or("missing value for --pager")?.clone();
            }
            _ if arg.starts_with("--style=") => {
                style = arg["--style=".len()..].to_string();
            }
            _ if arg.starts_with("--width=") => {
                width = parse_width(&arg["--width=".len()..])?;
            }
            _ if arg.starts_with("--color=") => {
                color = arg["--color=".len()..].to_string();
            }
            _ if arg.starts_with("--section=") => {
                section = Some(arg["--section=".len()..].to_string());
            }
            _ if arg.starts_with("--pager=") => {
                pager = arg["--pager=".len()..].to_string();
            }
            _ if arg.starts_with('-') && arg != "-" => {
                return Err(format!("unknown option: {arg}"));
            }
            _ => positional.push(arg.clone()),
        }
        i += 1;
    }

    if pager_always && pager == "never" {
        pager = "always".to_string();
    }

    Ok(Options {
        style,
        width,
        pager,
        args: positional,
        color,
        section,
        toc,
    })
}

fn parse_width(value: &str) -> Result<usize, String> {
    let width: usize = value
        .parse()
        .map_err(|_| format!("invalid --width={value:?}"))?;
    Ok(width)
}

fn print_usage() {
    let program = env::args().next().unwrap_or_else(|| "md".to_string());
    let mut out = io::stdout();
    let _ = writeln!(out, "Usage: {program} [options] [file|dir|-]...\n");
    let _ = writeln!(out, "Options:");
    let _ = writeln!(out, "  -p               open interactive pager (TUI)");
    let _ = writeln!(out, "  -s, --style      auto|dark|light (default: auto)");
    let _ = writeln!(
        out,
        "  -S, --section    show only the section under a heading"
    );
    let _ = writeln!(
        out,
        "                   (heading text, substring or #anchor)"
    );
    let _ = writeln!(
        out,
        "  --toc            print the heading outline with anchors"
    );
    let _ = writeln!(
        out,
        "  --color          auto|always|never (default: auto; honors NO_COLOR)"
    );
    let _ = writeln!(out, "  --no-color       same as --color=never");
    let _ = writeln!(out);
    let _ = writeln!(out, "Advanced:");
    let _ = writeln!(out, "  --pager          auto|always|never (default: never)");
    let _ = writeln!(out, "  -w, --width      render width (0 = auto)");
    let _ = writeln!(
        out,
        "\nDirectories expand to the Markdown files they contain."
    );
    let _ = writeln!(out, "\nExamples:");
    let _ = writeln!(out, "  {program} README.md");
    let _ = writeln!(out, "  {program} -p README.md");
    let _ = writeln!(
        out,
        "  {program} -p docs/            # browse a folder of docs"
    );
    let _ = writeln!(out, "  {program} -S install README.md");
    let _ = writeln!(out, "  {program} --toc README.md");
    let _ = writeln!(out, "  cat README.md | {program}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        std::iter::once("md")
            .chain(list.iter().copied())
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn parses_new_flags() {
        let opts = parse_args(args(&[
            "-S",
            "Install",
            "--color=never",
            "--toc",
            "a.md",
            "b.md",
        ]))
        .unwrap();
        assert_eq!(opts.section.as_deref(), Some("Install"));
        assert_eq!(opts.color, "never");
        assert!(opts.toc);
        assert_eq!(opts.args, vec!["a.md", "b.md"]);
    }

    #[test]
    fn double_dash_ends_options() {
        let opts = parse_args(args(&["--", "-weird.md"])).unwrap();
        assert_eq!(opts.args, vec!["-weird.md"]);
    }

    #[test]
    fn keeps_existing_flags() {
        let opts = parse_args(args(&["-p", "-w", "70", "--style", "light"])).unwrap();
        assert_eq!(opts.pager, "always");
        assert_eq!(opts.width, 70);
        assert_eq!(opts.style, "light");
        assert_eq!(opts.color, "auto");
        assert!(parse_args(args(&["--bogus"])).is_err());
    }
}
