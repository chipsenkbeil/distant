use anyhow::Context;
use distant::{Cli, MainResult};

#[cfg(unix)]
fn main() -> MainResult {
    let cli = match Cli::initialize().context("Failed to initialize CLI") {
        Ok(cli) => cli,
        Err(x) => return MainResult::from(x),
    };
    let _logger = cli.init_logger();
    MainResult::from(cli.run())
}

#[cfg(windows)]
fn main() -> MainResult {
    let cli = match Cli::initialize().context("Failed to initialize CLI") {
        Ok(cli) => cli,
        Err(x) => return MainResult::from(x),
    };
    let _logger = cli.init_logger();

    // If we are trying to listen as a manager, try as a service first
    if cli.is_manager_listen_command() {
        match distant::win_service::run() {
            // Success! So we don't need to run again
            Ok(_) => return MainResult::OK,

            // In this case, we know there was a service error, and we're assuming it
            // means that we were trying to dispatch a service when we were not started
            // as a service, so we will move forward as a console application
            Err(distant::win_service::ServiceError::Service(_)) => (),

            // Otherwise, we got a raw error that we want to return
            Err(distant::win_service::ServiceError::Anyhow(x)) => return MainResult::from(x),
        }
    }

    // Otherwise, execute as a non-service CLI
    MainResult::from(cli.run())
}
