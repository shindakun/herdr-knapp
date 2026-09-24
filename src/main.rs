use std::process::ExitCode;

use knapp::cli::{self, USAGE};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("links") => cli::links(&args[1..]),
        Some("backlinks") => cli::backlinks(&args[1..]),
        Some("unresolved") => cli::unresolved(&args[1..]),
        Some("orphans") => cli::orphans(&args[1..]),
        Some("tags") => cli::tags(&args[1..]),
        Some("graph") => cli::graph(&args[1..]),
        Some("index") => cli::index(&args[1..]),
        Some("pane") => cli::pane(&args[1..]),
        Some("peek") => cli::peek(&args[1..]),
        Some("open-pane") => cli::open_pane(&args[1..]),
        Some("peek-selection") => cli::peek_selection(&args[1..]),
        None | Some("--help" | "-h" | "help") => {
            println!("{USAGE}");
            Ok(())
        }
        Some("--version" | "-V" | "version") => {
            println!("knapp {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(other) => Err(format!("unknown command: {other}\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("knapp: {err}");
            ExitCode::FAILURE
        }
    }
}
