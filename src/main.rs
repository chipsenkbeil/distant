//! # distant
//!
//! ### Exit codes
//!
//! * EX_USAGE (64) - being used when arguments missing or bad arguments provided to CLI
//! * EX_DATAERR (65) - being used when bad data received not in UTF-8 format or transport data is bad
//! * EX_NOINPUT (66) - being used when not getting expected data from launch
//! * EX_NOHOST (68) - being used when failed to resolve a host
//! * EX_UNAVAILABLE (69) - being used when IO error encountered where connection is problem
//! * EX_OSERR (71) - being used when fork failed
//! * EX_IOERR (74) - being used as catchall for IO errors
//! * EX_TEMPFAIL (75) - being used when we get a timeout
//! * EX_PROTOCOL (76) - being used as catchall for transport errors
use distant::{Cli, ExitCodeError};
use log::*;

#[cfg(unix)]
fn main() {
    match Cli::initialize() {
        Ok(cli) => {
            let logger = cli.init_logger();
            if let Err(x) = cli.run() {
                if !x.is_silent() {
                    error!("{}", x);
                    eprintln!("{}", x);
                }
                logger.flush();
                logger.shutdown();

                std::process::exit(x.to_i32());
            }
        }
        Err(x) => eprintln!("{}", x),
    }
}

#[cfg(windows)]
fn main() {
    match Cli::initialize() {
        Ok(cli) => {
            // If we are trying to listen as a manager, try as a service first, and if that
            // fails (because we are not a service) then try as a regular console application
            if cli.is_manager_listen_command() {
                match distant::win_service::run() {
                    Ok(_) => return,

                    // In this case, we know there was a service error, and we're assuming it
                    // means that we were trying to dispatch a service when we were not started
                    // as a service, so we will move forward as a console application
                    Err(distant::win_service::ServiceError::Service(_)) => (),

                    // In this case, the service wouldn't work, so we want to fail
                    Err(distant::win_service::ServiceError::FailedToCreateServiceConfig(x)) => {
                        let logger = cli.init_logger();
                        error!("Failed to create service config: {x}");
                        logger.flush();
                        logger.shutdown();

                        std::process::exit(distant::ExitCode::Config.to_i32());
                        return;
                    }

                    // In this case, there was an error deleting the service config after
                    // sucessfully running and stopping a windows service using the dispatcher;
                    // so, we want to log the error and then fail
                    Err(distant::win_service::ServiceError::FailedToDeleteServiceConfig(x)) => {
                        let logger = cli.init_logger();
                        error!("Failed to delete service config: {x}");
                        logger.flush();
                        logger.shutdown();
                        return;
                    }
                }
            }

            let logger = cli.init_logger();
            if let Err(x) = cli.run() {
                if !x.is_silent() {
                    error!("{}", x);
                    eprintln!("{}", x);
                }
                logger.flush();
                logger.shutdown();

                std::process::exit(x.to_i32());
            }
        }
        Err(x) => eprintln!("{}", x),
    }
}
