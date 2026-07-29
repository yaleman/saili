mod app;
mod backend;

use std::process::ExitCode;

fn main() -> ExitCode {
    match app::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("saili: {error}");
            ExitCode::FAILURE
        }
    }
}
