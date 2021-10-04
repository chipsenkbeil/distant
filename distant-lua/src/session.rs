use crate::{runtime::get_runtime, utils};
use async_compat::CompatExt;
use distant_core::{
    SecretKey32, Session as DistantSession, SessionChannel, XChaCha20Poly1305Codec,
};
use distant_ssh2::{IntoDistantSessionOpts, Ssh2Session};
use mlua::{prelude::*, LuaSerdeExt, UserData, UserDataFields, UserDataMethods};
use once_cell::sync::Lazy;
use paste::paste;
use std::{collections::HashMap, io, sync::RwLock};

/// Makes a Lua table containing the session functions
pub fn make_session_tbl(lua: &Lua) -> LuaResult<LuaTable> {
    let tbl = lua.create_table()?;

    // get_by_id(id: usize) -> Option<Session>
    tbl.set(
        "get_by_id",
        lua.create_function(|_, id: usize| {
            let exists = has_session(id)?;
            if exists {
                Ok(Some(Session::new(id)))
            } else {
                Ok(None)
            }
        })?,
    )?;

    // launch(opts: LaunchOpts) -> Future<Session>
    tbl.set(
        "launch",
        lua.create_async_function(|lua, opts: LuaValue| async move {
            let opts = LaunchOpts::from_lua(opts, lua)?;
            Session::launch(opts).compat().await
        })?,
    )?;

    // launch_sync(opts: LaunchOpts) -> Session
    tbl.set(
        "launch_sync",
        lua.create_function(|lua, opts: LuaValue| {
            let opts = LaunchOpts::from_lua(opts, lua)?;
            get_runtime()?.block_on(Session::launch(opts))
        })?,
    )?;

    // connect(opts: ConnectOpts) -> Future<Session>
    tbl.set(
        "connect",
        lua.create_async_function(|lua, opts: LuaValue| async move {
            let opts = ConnectOpts::from_lua(opts, lua)?;
            Session::connect(opts).compat().await
        })?,
    )?;

    // connect_sync(opts: ConnectOpts) -> Session
    tbl.set(
        "connect_sync",
        lua.create_function(|lua, opts: LuaValue| {
            let opts = ConnectOpts::from_lua(opts, lua)?;
            get_runtime()?.block_on(Session::connect(opts))
        })?,
    )?;

    Ok(tbl)
}

/// try_timeout!(timeout: Duration, Future<Output = Result<T, E>>) -> LuaResult<T>
macro_rules! try_timeout {
    ($timeout:expr, $f:expr) => {{
        use async_compat::CompatExt;
        use futures::future::FutureExt;
        use mlua::prelude::*;
        let timeout: std::time::Duration = $timeout;
        let fut = ($f).fuse().compat();
        let sleep = tokio::time::sleep(timeout).fuse().compat();

        tokio::select! {
            _ = sleep => {
                let err = std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!("Reached timeout of {}s", timeout.as_secs_f32())
                );
                Err(err.to_lua_err())
            }
            res = fut => {
                res.to_lua_err()
            }
        }
    }};
}

mod api;
mod opts;
mod proc;

use opts::Mode;
pub use opts::{ConnectOpts, LaunchOpts};
use proc::{RemoteLspProcess, RemoteProcess};

/// Contains mapping of id -> session for use in maintaining active sessions
static SESSION_MAP: Lazy<RwLock<HashMap<usize, DistantSession>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

fn has_session(id: usize) -> LuaResult<bool> {
    Ok(SESSION_MAP
        .read()
        .map_err(|x| x.to_string().to_lua_err())?
        .contains_key(&id))
}

fn get_session_channel(id: usize) -> LuaResult<SessionChannel> {
    let lock = SESSION_MAP.read().map_err(|x| x.to_string().to_lua_err())?;
    let session = lock.get(&id).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotConnected,
            format!("No session connected with id {}", id),
        )
        .to_lua_err()
    })?;

    Ok(session.clone_channel())
}

/// Holds a reference to the session to perform remote operations
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Session {
    id: usize,
}

impl Session {
    /// Creates a new session referencing the given distant session with the specified id
    pub fn new(id: usize) -> Self {
        Self { id }
    }

    /// Launches a new distant session on a remote machine
    pub async fn launch(opts: LaunchOpts<'_>) -> LuaResult<Self> {
        let LaunchOpts {
            host,
            mode,
            handler,
            ssh,
            timeout,
        } = opts;

        // First, establish a connection to an SSH server
        let mut ssh_session = Ssh2Session::connect(host, ssh.into()).to_lua_err()?;

        // Second, authenticate with the server
        ssh_session
            .authenticate(handler)
            .compat()
            .await
            .to_lua_err()?;

        // Third, convert our ssh session into a distant session based on desired method
        let session = match mode {
            Mode::Distant => ssh_session
                .into_distant_session(IntoDistantSessionOpts {
                    timeout,
                    ..Default::default()
                })
                .compat()
                .await
                .to_lua_err()?,
            Mode::Ssh => ssh_session.into_ssh_client_session().to_lua_err()?,
        };

        // Fourth, store our current session in our global map and then return a reference
        let id = utils::rand_u32()? as usize;
        SESSION_MAP
            .write()
            .map_err(|x| x.to_string().to_lua_err())?
            .insert(id, session);
        Ok(Self::new(id))
    }

    /// Connects to an already-running remote distant server
    pub async fn connect(opts: ConnectOpts) -> LuaResult<Self> {
        let addr = tokio::net::lookup_host(format!("{}:{}", opts.host, opts.port))
            .compat()
            .await
            .to_lua_err()?
            .next()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::AddrNotAvailable,
                    "Failed to resolve host & port",
                )
            })
            .to_lua_err()?;

        let key: SecretKey32 = opts.key.parse().to_lua_err()?;
        let codec = XChaCha20Poly1305Codec::from(key);

        let session = DistantSession::tcp_connect_timeout(addr, codec, opts.timeout)
            .compat()
            .await
            .to_lua_err()?;

        let id = utils::rand_u32()? as usize;
        SESSION_MAP
            .write()
            .map_err(|x| x.to_string().to_lua_err())?
            .insert(id, session);
        Ok(Self::new(id))
    }
}

/// impl_methods!(methods: &mut M, name: Ident)
macro_rules! impl_methods {
    ($methods:expr, $name:ident) => {
        impl_methods!($methods, $name, |_lua, data| {Ok(data)});
    };
    ($methods:expr, $name:ident, |$lua:ident, $data:ident| $block:block) => {{
        paste! {
            $methods.add_method(stringify!([<$name:snake _sync>]), |$lua, this, params: LuaValue| {
                let params: api::[<$name:camel Params>] = $lua.from_value(params)?;
                let $data = api::[<$name:snake _sync>](get_session_channel(this.id)?, params)?;

                #[allow(unused_braces)]
                $block
            });
            $methods.add_async_method(stringify!([<$name:snake>]), |$lua, this, params: LuaValue| async move {
                use async_compat::CompatExt;
                let params: api::[<$name:camel Params>] = $lua.from_value(params)?;
                let $data = api::[<$name:snake>](get_session_channel(this.id)?, params).compat().await?;

                #[allow(unused_braces)]
                $block
            });
        }
    }};
}

impl UserData for Session {
    fn add_fields<'lua, F: UserDataFields<'lua, Self>>(fields: &mut F) {
        fields.add_field_method_get("id", |_, this| Ok(this.id));
    }

    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method("is_active", |_, this, _: LuaValue| {
            Ok(get_session_channel(this.id).is_ok())
        });

        impl_methods!(methods, append_file);
        impl_methods!(methods, append_file_text);
        impl_methods!(methods, copy);
        impl_methods!(methods, create_dir);
        impl_methods!(methods, exists);
        impl_methods!(methods, metadata, |lua, m| { lua.to_value(&m) });
        impl_methods!(methods, read_dir, |lua, results| {
            let (entries, errors) = results;
            let tbl = lua.create_table()?;
            tbl.set(
                "entries",
                entries
                    .iter()
                    .map(|x| lua.to_value(x))
                    .collect::<LuaResult<Vec<LuaValue>>>()?,
            )?;
            tbl.set(
                "errors",
                errors
                    .iter()
                    .map(|x| x.to_string())
                    .collect::<Vec<String>>(),
            )?;

            Ok(tbl)
        });
        impl_methods!(methods, read_file);
        impl_methods!(methods, read_file_text);
        impl_methods!(methods, remove);
        impl_methods!(methods, rename);
        impl_methods!(methods, spawn, |_lua, proc| {
            Ok(RemoteProcess::from_distant(proc))
        });
        impl_methods!(methods, spawn_lsp, |_lua, proc| {
            Ok(RemoteLspProcess::from_distant(proc))
        });
        impl_methods!(methods, write_file);
        impl_methods!(methods, write_file_text);
    }
}
