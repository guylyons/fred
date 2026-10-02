//! Command-line arguments.

use std::path::PathBuf;

pub const USAGE: &str = "usage: fred [-i | -f] [--height N] [+LINE] [FILE | DIR]

  FILE              file to edit (created on first write if missing)
  DIR               browse files there
  +LINE             start on line LINE; + alone starts on the last line
  -i, --inline      edit in a window under the prompt
  -f, --fullscreen  take over the terminal (the default, unless the
                    config says otherwise)
  --height N        inline, showing N lines of text, or `max` (default 12)
  -h, --help        show this help
  -V, --version     show the version

Config: ~/.config/fred/config.toml (fullscreen, height, wrap, numbers,
relative_numbers, theme, tabstop, autocomplete)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineArg {
    N(usize),
    Last,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Args {
    pub inline: bool,
    pub fullscreen: bool,
    pub height: Option<usize>,
    pub line: Option<LineArg>,
    pub file: Option<PathBuf>,
}

#[derive(Debug)]
pub enum ArgsOrInfo {
    Run(Args),
    Help,
    Version,
}

fn height(v: &str) -> Result<usize, String> {
    if v == "max" {
        return Ok(usize::MAX);
    }
    match v.parse::<usize>() {
        Ok(n) if n > 0 => Ok(n),
        _ => Err(format!("invalid height: {v}")),
    }
}

/// Parse arguments (without the program name).
pub fn parse(args: Vec<String>) -> Result<ArgsOrInfo, String> {
    let mut out = Args::default();
    let mut it = args.into_iter();
    let mut only_files = false;
    while let Some(a) = it.next() {
        if !only_files {
            match a.as_str() {
                "-h" | "--help" => return Ok(ArgsOrInfo::Help),
                "-V" | "--version" => return Ok(ArgsOrInfo::Version),
                "--" => {
                    only_files = true;
                    continue;
                }
                "-i" | "--inline" => {
                    out.inline = true;
                    continue;
                }
                "-f" | "--fullscreen" => {
                    out.fullscreen = true;
                    continue;
                }
                "--height" => {
                    let v = it.next().ok_or("--height needs a value")?;
                    out.height = Some(height(&v)?);
                    continue;
                }
                _ if a.starts_with("--height=") => {
                    out.height = Some(height(&a["--height=".len()..])?);
                    continue;
                }
                "+" => {
                    out.line = Some(LineArg::Last);
                    continue;
                }
                _ if a.starts_with('+') => {
                    let n = a[1..]
                        .parse::<usize>()
                        .map_err(|_| format!("invalid line: {a}"))?;
                    out.line = Some(LineArg::N(n.max(1)));
                    continue;
                }
                _ if a.starts_with('-') && a.len() > 1 => {
                    return Err(format!("unknown option: {a}"));
                }
                _ => {}
            }
        }
        if out.file.is_some() {
            return Err("only one file can be edited at a time".into());
        }
        out.file = Some(PathBuf::from(a));
    }
    Ok(ArgsOrInfo::Run(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_args() {
        assert!(matches!(
            parse(v(&["--height", "5", "+3", "f"])),
            Ok(ArgsOrInfo::Run(Args {
                inline: false,
                fullscreen: false,
                height: Some(5),
                line: Some(LineArg::N(3)),
                file: Some(_)
            }))
        ));
        assert!(matches!(
            parse(v(&["+"])),
            Ok(ArgsOrInfo::Run(Args {
                line: Some(LineArg::Last),
                file: None,
                ..
            }))
        ));
        assert!(matches!(
            parse(v(&["--height=7"])),
            Ok(ArgsOrInfo::Run(Args {
                inline: false,
                fullscreen: false,
                height: Some(7),
                ..
            }))
        ));
        assert!(matches!(
            parse(v(&["--", "-weird"])),
            Ok(ArgsOrInfo::Run(Args { file: Some(_), .. }))
        ));
        assert!(matches!(parse(v(&["--help"])), Ok(ArgsOrInfo::Help)));
        assert!(matches!(parse(v(&["-V"])), Ok(ArgsOrInfo::Version)));
        assert!(parse(v(&["a", "b"])).is_err());
        assert!(parse(v(&["--height"])).is_err());
        assert!(parse(v(&["--height", "0"])).is_err());
        assert!(
            matches!(parse(v(&["--height", "max"])), Ok(ArgsOrInfo::Run(a)) if a.height == Some(usize::MAX))
        );
        assert!(
            matches!(parse(v(&["-i", "f"])), Ok(ArgsOrInfo::Run(a)) if a.inline && a.file.is_some())
        );
        assert!(matches!(parse(v(&["-f"])), Ok(ArgsOrInfo::Run(a)) if a.fullscreen));
        assert!(parse(v(&["--bogus"])).is_err());
        assert!(parse(v(&["+x"])).is_err());
    }
}
