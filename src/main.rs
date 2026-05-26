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
    let mut positional = Vec::new();

    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
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
    let _ = writeln!(out, "Usage: {program} [options] [file|-]\n");
    let _ = writeln!(out, "Options:");
    let _ = writeln!(out, "  -p             open interactive pager (TUI)");
    let _ = writeln!(out, "  -s, --style    auto|dark|light (default: auto)");
    let _ = writeln!(out);
    let _ = writeln!(out, "Advanced:");
    let _ = writeln!(out, "  --pager        auto|always|never (default: never)");
    let _ = writeln!(out, "  -w, --width    render width (0 = auto)");
    let _ = writeln!(out, "\nExamples:");
    let _ = writeln!(out, "  {program} README.md");
    let _ = writeln!(out, "  {program} -p README.md");
    let _ = writeln!(out, "  cat README.md | {program}");
}
