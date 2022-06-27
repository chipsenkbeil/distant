use clap::{Args, ValueEnum};
use merge::Merge;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[clap(rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Off,
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub fn to_log_level_filter(self) -> log::LevelFilter {
        match self {
            Self::Off => log::LevelFilter::Off,
            Self::Error => log::LevelFilter::Error,
            Self::Warn => log::LevelFilter::Warn,
            Self::Info => log::LevelFilter::Info,
            Self::Debug => log::LevelFilter::Debug,
            Self::Trace => log::LevelFilter::Trace,
        }
    }
}

impl Default for LogLevel {
    fn default() -> Self {
        Self::Info
    }
}

/// Contains options that are common across subcommands
#[derive(Args, Clone, Debug, Default, Merge, Serialize, Deserialize)]
pub struct CommonConfig {
    /// Quiet mode, suppresses all logging (shortcut for log level off)
    #[clap(short, long, global = true)]
    #[merge(strategy = merge::bool::overwrite_false)]
    pub quiet: bool,

    /// Log level to use throughout the application
    #[clap(long, global = true, case_insensitive = true, value_enum)]
    pub log_level: Option<LogLevel>,

    /// Log output to disk instead of stderr
    #[clap(long, global = true)]
    pub log_file: Option<PathBuf>,

    /// Represents the maximum time (in seconds) to wait for a network request before timing out
    #[clap(short, long, global = true)]
    pub timeout: Option<f32>,
}

impl CommonConfig {
    pub fn log_level_or_default(&self) -> LogLevel {
        self.log_level.as_ref().copied().unwrap_or_default()
    }
}
