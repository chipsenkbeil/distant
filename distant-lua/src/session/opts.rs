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
    pub host: String,
    pub mode: Mode,
    pub handler: Ssh2AuthHandler<'a>,
    pub ssh: Ssh2SessionOpts,
    pub timeout: Duration,
}

impl fmt::Debug for LaunchOpts<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LaunchOpts")
            .field("host", &self.host)
            .field("mode", &self.mode)
            .field("handler", &"...")
            .field("ssh", &self.ssh)
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
