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
    let cli = Cli::new();
    let config = cli.load_config().await.expect("Failed to load config");
    let logger = init_logging(&config, opt.subcommand.is_remote_process());
    if let Err(x) = cli.run(config).await {
        if !x.is_silent() {
            error!("Exiting due to error: {}", x);
        }
        logger.flush();
        logger.shutdown();

        std::process::exit(x.to_i32());
    }
}
