use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match mirelay::server::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let message = format!("{error:#}");
            eprintln!("error: {}", mirelay::cli::terminal_safe(&message));
            ExitCode::FAILURE
        }
    }
}
