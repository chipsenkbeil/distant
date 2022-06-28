use log::error;

mod cli;
pub mod config;
mod constants;
mod error;

pub use cli::Cli;
pub use config::Config;
pub use error::{ExitCode, ExitCodeError};

/// Main entrypoint into the program
pub async fn run() {
    let cli = Cli::initialize().await.expect("Failed to initialize CLI");
    let logger = cli.init_logger();
    if let Err(x) = cli.run().await {
        if !x.is_silent() {
            error!("Exiting due to error: {}", x);
        }
        logger.flush();
        logger.shutdown();

        std::process::exit(x.to_i32());
    }
}
