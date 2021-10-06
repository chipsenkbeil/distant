use mlua::prelude::*;
use serde::{Deserialize, Serialize};
use simplelog::{
    ColorChoice, CombinedLogger, ConfigBuilder, LevelFilter, SharedLogger, TermLogger,
    TerminalMode, WriteLogger,
};
use std::{fs::File, path::PathBuf};

macro_rules! set_log_fn {
    ($lua:expr, $tbl:expr, $name:ident) => {
        $tbl.set(
            stringify!($name),
            $lua.create_function(|_, msg: String| {
                ::log::$name!("{}", msg);
                Ok(())
            })?,
        )?;
    };
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum LogLevel {
    Off,
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl From<LogLevel> for LevelFilter {
    fn from(level: LogLevel) -> Self {
        match level {
            LogLevel::Off => Self::Off,
            LogLevel::Error => Self::Error,
            LogLevel::Warn => Self::Warn,
            LogLevel::Info => Self::Info,
            LogLevel::Debug => Self::Debug,
            LogLevel::Trace => Self::Trace,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
struct LogOpts {
    /// Indicating whether or not to log to terminal
    terminal: bool,

    /// Path to file to store logs
    file: Option<PathBuf>,

    /// Base level at which to write logs
    /// (e.g. if debug then trace would not be logged)
    level: LogLevel,
}

impl Default for LogOpts {
    fn default() -> Self {
        Self {
            terminal: false,
            file: None,
            level: LogLevel::Warn,
        }
    }
}

fn init_logger(opts: LogOpts) -> LuaResult<()> {
    let mut loggers: Vec<Box<dyn SharedLogger>> = Vec::new();
    let config = ConfigBuilder::new()
        .add_filter_allow_str("distant_core")
        .add_filter_allow_str("distant_ssh2")
        .add_filter_allow_str("distant_lua")
        .build();

    if opts.terminal {
        loggers.push(TermLogger::new(
            opts.level.into(),
            config.clone(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ));
    }

    if let Some(path) = opts.file {
        loggers.push(WriteLogger::new(
            opts.level.into(),
            config,
            File::create(path)?,
        ));
    }

    CombinedLogger::init(loggers).to_lua_err()?;
    Ok(())
}

/// Makes a Lua table containing the log functions
pub fn make_log_tbl(lua: &Lua) -> LuaResult<LuaTable> {
    let tbl = lua.create_table()?;

    tbl.set(
        "init",
        lua.create_function(|lua, opts: LuaValue| init_logger(lua.from_value(opts)?))?,
    )?;

    set_log_fn!(lua, tbl, error);
    set_log_fn!(lua, tbl, warn);
    set_log_fn!(lua, tbl, info);
    set_log_fn!(lua, tbl, debug);
    set_log_fn!(lua, tbl, trace);

    Ok(tbl)
}
