use fred::args::{self, ArgsOrInfo};
use fred::config::Config;

fn main() {
    let argv: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| {
            a.into_string()
                .unwrap_or_else(|o| o.to_string_lossy().into_owned())
        })
        .collect();
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
            match fred::app::run(a, cfg, err) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("fred: {e}");
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
