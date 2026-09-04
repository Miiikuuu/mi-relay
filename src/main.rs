use std::process::ExitCode;

fn main() -> ExitCode {
    match mirelay::cli::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let message = format!("{error:#}");
            eprintln!("error: {}", mirelay::cli::terminal_safe(&message));
            ExitCode::FAILURE
        }
    }
}
