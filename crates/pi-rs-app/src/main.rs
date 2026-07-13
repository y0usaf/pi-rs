use std::process::ExitCode;

use pi_rs_app::launcher::{help_text, parse, run};

fn main() -> ExitCode {
    let options = match parse(std::env::args_os().skip(1)) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("pi: {error}");
            return ExitCode::FAILURE;
        }
    };

    if options.help {
        print!("{}", help_text());
        return ExitCode::SUCCESS;
    }
    if options.version {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }

    match run(&options, &mut std::io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("pi: {error}");
            ExitCode::FAILURE
        }
    }
}
