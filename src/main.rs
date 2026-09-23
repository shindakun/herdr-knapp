use std::process::ExitCode;

const USAGE: &str = "usage: knapp <command>

commands:
  help       show this message
  version    print the version";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
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
