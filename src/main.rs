use anyhow::Context;
use distant::{AppResult, Cli, ExitCodeError};
use log::*;

#[cfg(unix)]
fn main() -> anyhow::Result<()> {
    let cli = Cli::initialize().context("Failed to initialize CLI")?;
    let logger = cli.init_logger();
    cli.run()
}

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    let cli = Cli::initialize().context("Failed to initialize CLI")?;
    let logger = cli.init_logger();

    // If we are trying to listen as a manager, try as a service first
    if cli.is_manager_listen_command() {
        match distant::win_service::run() {
            // Success! So we don't need to run again
            Ok(_) => return Ok(()),

            // In this case, we know there was a service error, and we're assuming it
            // means that we were trying to dispatch a service when we were not started
            // as a service, so we will move forward as a console application
            Err(distant::win_service::ServiceError::Service(_)) => (),

            // Otherwise, we got a raw error that we want to return
            Err(distant::win_service::ServiceError::Anyhow(x)) => return Err(x),
        }
    }

    // Otherwise, execute as a non-service CLI
    cli.run()
}
