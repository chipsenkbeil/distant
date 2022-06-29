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

#[tokio::main]
async fn main() {
    match Cli::initialize().await {
        Ok(cli) => {
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
        Err(x) => eprintln!("{}", x),
    }
}
