use std::process::ExitCode;

fn main() -> ExitCode {
    match mirelay::tus_client::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let message = format!("{error:#}");
            eprintln!("error: {}", mirelay::cli::terminal_safe(&message));
            ExitCode::FAILURE
        }
    }
}
