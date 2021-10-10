use crate::constants::TIMEOUT_MILLIS;
use distant_ssh2::{Ssh2AuthHandler, Ssh2SessionOpts};
use mlua::prelude::*;
use serde::Deserialize;
use std::{fmt, io, time::Duration};

#[derive(Clone, Debug, Default)]
pub struct ConnectOpts {
    pub host: String,
    pub port: u16,
    pub key: String,
    pub timeout: Duration,
}

impl<'lua> FromLua<'lua> for ConnectOpts {
    fn from_lua(lua_value: LuaValue<'lua>, _: &'lua Lua) -> LuaResult<Self> {
        match lua_value {
            LuaValue::Table(tbl) => Ok(Self {
                host: tbl.get("host")?,
                port: tbl.get("port")?,
                key: tbl.get("key")?,
                timeout: {
                    let milliseconds: Option<u64> = tbl.get("timeout")?;
                    Duration::from_millis(milliseconds.unwrap_or(TIMEOUT_MILLIS))
                },
            }),
            LuaValue::Nil => Err(LuaError::FromLuaConversionError {
                from: "Nil",
                to: "ConnectOpts",
                message: None,
            }),
            LuaValue::Boolean(_) => Err(LuaError::FromLuaConversionError {
                from: "Boolean",
                to: "ConnectOpts",
                message: None,
            }),
            LuaValue::LightUserData(_) => Err(LuaError::FromLuaConversionError {
                from: "LightUserData",
                to: "ConnectOpts",
                message: None,
            }),
            LuaValue::Integer(_) => Err(LuaError::FromLuaConversionError {
                from: "Integer",
                to: "ConnectOpts",
                message: None,
            }),
            LuaValue::Number(_) => Err(LuaError::FromLuaConversionError {
                from: "Number",
                to: "ConnectOpts",
                message: None,
            }),
            LuaValue::String(_) => Err(LuaError::FromLuaConversionError {
                from: "String",
                to: "ConnectOpts",
                message: None,
            }),
            LuaValue::Function(_) => Err(LuaError::FromLuaConversionError {
                from: "Function",
                to: "ConnectOpts",
                message: None,
            }),
            LuaValue::Thread(_) => Err(LuaError::FromLuaConversionError {
                from: "Thread",
                to: "ConnectOpts",
                message: None,
            }),
            LuaValue::UserData(_) => Err(LuaError::FromLuaConversionError {
                from: "UserData",
                to: "ConnectOpts",
                message: None,
            }),
            LuaValue::Error(_) => Err(LuaError::FromLuaConversionError {
                from: "Error",
                to: "ConnectOpts",
                message: None,
            }),
        }
    }
}

#[derive(Default)]
pub struct LaunchOpts<'a> {
    /// Host to connect to remotely (e.g. example.com)
    pub host: String,

    /// Mode to use for communication (ssh or distant server)
    pub mode: Mode,

    /// Callbacks to be triggered on various authentication events
    pub handler: Ssh2AuthHandler<'a>,

    /// Miscellaneous ssh configuration options
    pub ssh: Ssh2SessionOpts,

    /// Options specific to launching the distant binary on the remote machine
    pub distant: LaunchDistantOpts,

    /// Maximum time to wait for launch to complete
    pub timeout: Duration,
}

impl fmt::Debug for LaunchOpts<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LaunchOpts")
            .field("host", &self.host)
            .field("mode", &self.mode)
            .field("handler", &"...")
            .field("ssh", &self.ssh)
            .field("distant", &self.distant)
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl<'lua> FromLua<'lua> for LaunchOpts<'lua> {
    fn from_lua(lua_value: LuaValue<'lua>, lua: &'lua Lua) -> LuaResult<Self> {
        let Ssh2AuthHandler {
            on_authenticate,
            on_banner,
            on_error,
            on_host_verify,
        } = Default::default();

        match lua_value {
            LuaValue::Table(tbl) => Ok(Self {
                host: tbl.get("host")?,
                mode: {
                    let mode: Option<LuaValue> = tbl.get("mode")?;
                    match mode {
                        Some(value) => lua.from_value(value)?,
                        None => Default::default(),
                    }
                },
                handler: Ssh2AuthHandler {
                    on_authenticate: {
                        let f: Option<LuaFunction> = tbl.get("on_authenticate")?;
                        match f {
                            Some(f) => Box::new(move |ev| {
                                let value = to_value!(lua, &ev)
                                    .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?;
                                f.call::<LuaValue, Vec<String>>(value)
                                    .map_err(|x| io::Error::new(io::ErrorKind::Other, x))
                            }),
                            None => on_authenticate,
                        }
                    },
                    on_banner: {
                        let f: Option<LuaFunction> = tbl.get("on_banner")?;
                        match f {
                            Some(f) => Box::new(move |banner| {
                                let _ = f.call::<String, ()>(banner.to_string());
                            }),
                            None => on_banner,
                        }
                    },
                    on_host_verify: {
                        let f: Option<LuaFunction> = tbl.get("on_host_verify")?;
                        match f {
                            Some(f) => Box::new(move |host| {
                                f.call::<String, bool>(host.to_string())
                                    .map_err(|x| io::Error::new(io::ErrorKind::Other, x))
                            }),
                            None => on_host_verify,
                        }
                    },
                    on_error: {
                        let f: Option<LuaFunction> = tbl.get("on_error")?;
                        match f {
                            Some(f) => Box::new(move |err| {
                                let _ = f.call::<String, ()>(err.to_string());
                            }),
                            None => on_error,
                        }
                    },
                },
                ssh: {
                    let ssh_tbl: Option<LuaValue> = tbl.get("ssh")?;
                    match ssh_tbl {
                        Some(value) => lua.from_value(value)?,
                        None => Default::default(),
                    }
                },
                distant: {
                    let distant_tbl: Option<LuaValue> = tbl.get("distant")?;
                    match distant_tbl {
                        Some(value) => LaunchDistantOpts::from_lua(value, lua)?,
                        None => Default::default(),
                    }
                },
                timeout: {
                    let milliseconds: Option<u64> = tbl.get("timeout")?;
                    Duration::from_millis(milliseconds.unwrap_or(TIMEOUT_MILLIS))
                },
            }),
            LuaValue::Nil => Err(LuaError::FromLuaConversionError {
                from: "Nil",
                to: "LaunchOpts",
                message: None,
            }),
            LuaValue::Boolean(_) => Err(LuaError::FromLuaConversionError {
                from: "Boolean",
                to: "LaunchOpts",
                message: None,
            }),
            LuaValue::LightUserData(_) => Err(LuaError::FromLuaConversionError {
                from: "LightUserData",
                to: "LaunchOpts",
                message: None,
            }),
            LuaValue::Integer(_) => Err(LuaError::FromLuaConversionError {
                from: "Integer",
                to: "LaunchOpts",
                message: None,
            }),
            LuaValue::Number(_) => Err(LuaError::FromLuaConversionError {
                from: "Number",
                to: "LaunchOpts",
                message: None,
            }),
            LuaValue::String(_) => Err(LuaError::FromLuaConversionError {
                from: "String",
                to: "LaunchOpts",
                message: None,
            }),
            LuaValue::Function(_) => Err(LuaError::FromLuaConversionError {
                from: "Function",
                to: "LaunchOpts",
                message: None,
            }),
            LuaValue::Thread(_) => Err(LuaError::FromLuaConversionError {
                from: "Thread",
                to: "LaunchOpts",
                message: None,
            }),
            LuaValue::UserData(_) => Err(LuaError::FromLuaConversionError {
                from: "UserData",
                to: "LaunchOpts",
                message: None,
            }),
            LuaValue::Error(_) => Err(LuaError::FromLuaConversionError {
                from: "Error",
                to: "LaunchOpts",
                message: None,
            }),
        }
    }
}

#[derive(Debug)]
pub struct LaunchDistantOpts {
    /// Binary representing the distant server on the remote machine
    pub bin: String,

    /// Additional CLI options to pass to the distant server when starting
    pub args: String,

    /// If true, will run distant via `echo <CMD> | $SHELL -l`, which will spawn a login shell to
    /// execute distant
    pub use_login_shell: bool,
}

impl Default for LaunchDistantOpts {
    /// Create default options
    ///
    /// * bin = "distant"
    /// * args = ""
    /// * use_login_shell = false
    fn default() -> Self {
        Self {
            bin: String::from("distant"),
            args: String::new(),
            use_login_shell: false,
        }
    }
}

impl<'lua> FromLua<'lua> for LaunchDistantOpts {
    fn from_lua(lua_value: LuaValue<'lua>, lua: &'lua Lua) -> LuaResult<Self> {
        let LaunchDistantOpts {
            bin: default_bin,
            args: default_args,
            use_login_shell: default_use_login_shell,
        } = Default::default();

        match lua_value {
            LuaValue::Table(tbl) => Ok(Self {
                bin: {
                    let bin: Option<String> = tbl.get("bin")?;
                    bin.unwrap_or(default_bin)
                },

                // Allows "--some --args" or {"--some", "--args"}
                args: {
                    let value: LuaValue = tbl.get("args")?;
                    match value {
                        LuaValue::Nil => default_args,
                        LuaValue::String(args) => args.to_str()?.to_string(),
                        x => {
                            let args: Vec<String> = lua.from_value(x)?;
                            args.join(" ")
                        }
                    }
                },

                use_login_shell: tbl
                    .get::<_, Option<bool>>("use_login_shell")?
                    .unwrap_or(default_use_login_shell),
            }),
            LuaValue::Nil => Err(LuaError::FromLuaConversionError {
                from: "Nil",
                to: "LaunchDistantOpts",
                message: None,
            }),
            LuaValue::Boolean(_) => Err(LuaError::FromLuaConversionError {
                from: "Boolean",
                to: "LaunchDistantOpts",
                message: None,
            }),
            LuaValue::LightUserData(_) => Err(LuaError::FromLuaConversionError {
                from: "LightUserData",
                to: "LaunchDistantOpts",
                message: None,
            }),
            LuaValue::Integer(_) => Err(LuaError::FromLuaConversionError {
                from: "Integer",
                to: "LaunchDistantOpts",
                message: None,
            }),
            LuaValue::Number(_) => Err(LuaError::FromLuaConversionError {
                from: "Number",
                to: "LaunchDistantOpts",
                message: None,
            }),
            LuaValue::String(_) => Err(LuaError::FromLuaConversionError {
                from: "String",
                to: "LaunchDistantOpts",
                message: None,
            }),
            LuaValue::Function(_) => Err(LuaError::FromLuaConversionError {
                from: "Function",
                to: "LaunchDistantOpts",
                message: None,
            }),
            LuaValue::Thread(_) => Err(LuaError::FromLuaConversionError {
                from: "Thread",
                to: "LaunchDistantOpts",
                message: None,
            }),
            LuaValue::UserData(_) => Err(LuaError::FromLuaConversionError {
                from: "UserData",
                to: "LaunchDistantOpts",
                message: None,
            }),
            LuaValue::Error(_) => Err(LuaError::FromLuaConversionError {
                from: "Error",
                to: "LaunchDistantOpts",
                message: None,
            }),
        }
    }
}

#[derive(Copy, Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Distant,
    Ssh,
}

impl Default for Mode {
    fn default() -> Self {
        Self::Distant
    }
}
