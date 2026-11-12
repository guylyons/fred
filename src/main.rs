use fred::args::{self, ArgsOrInfo};
use fred::config::Config;
use std::io::Write;

fn main() {
    let argv: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| {
            a.into_string()
                .unwrap_or_else(|o| o.to_string_lossy().into_owned())
        })
        .collect();
    if argv == ["--mcp"] {
        std::process::exit(fred::remote::mcp());
    }
    // `fred pack …` manages packages; a file named pack is `fred ./pack`.
    if argv.first().is_some_and(|a| a == "pack") {
        let (lines, ok) = fred::pack::command(&argv[1..].join(" "));
        println!("{}", lines.join("\n"));
        std::process::exit(if ok { 0 } else { 1 });
    }
    let code = match args::parse(argv) {
        Ok(ArgsOrInfo::Help) => {
            println!("{}", args::USAGE);
            0
        }
        Ok(ArgsOrInfo::Version) => {
            println!("fred {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Ok(ArgsOrInfo::Run(a)) => {
            let (cfg, err) = Config::load();
            if !a.no_packages {
                fred::pack::load();
            }
            let run = if a.gui {
                fred::gui::run
            } else {
                fred::app::run
            };
            match run(a, cfg, err) {
                Ok(c) => c,
                Err(e) => {
                    // Not eprintln!: with the terminal gone it would panic.
                    let _ = writeln!(std::io::stderr(), "fred: {e}");
                    1
                }
            }
        }
        Err(e) => {
            eprintln!("fred: {e}\n\n{}", args::USAGE);
            1
        }
    };
    std::process::exit(code);
}
