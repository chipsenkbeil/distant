use clap::{Args, ValueEnum};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Contains settings are associated with logging.
#[derive(Args, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoggingSettings {
    /// Log level to use throughout the application
    #[clap(long, global = true, value_enum)]
    pub log_level: Option<LogLevel>,

    /// Path to file to use for logging
    #[clap(long, global = true)]
    pub log_file: Option<PathBuf>,
}

impl LoggingSettings {
    pub fn log_level_or_default(&self) -> LogLevel {
        self.log_level.as_ref().copied().unwrap_or_default()
    }
}

/// Represents the level associated with logging.
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
