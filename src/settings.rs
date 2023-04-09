mod common;
mod config;
mod options;

use self::config::Config;
use self::options::Options;

/// Contains all settings applied to the CLI.
///
/// This is a mixture of [`Config`] and [`Options`] with the order of precedence being to use
/// explicit options > config > implicit options.
#[derive(Clone, Debug, Default)]
pub struct Settings {
    config: Config,
    options: Options,
}
