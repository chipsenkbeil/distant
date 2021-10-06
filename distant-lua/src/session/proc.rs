use crate::runtime;
use distant_core::{
    RemoteLspProcess as DistantRemoteLspProcess, RemoteProcess as DistantRemoteProcess,
};
use mlua::{prelude::*, UserData, UserDataFields, UserDataMethods};
use once_cell::sync::Lazy;
use std::{collections::HashMap, io};
use tokio::sync::RwLock;

/// Contains mapping of id -> remote process for use in maintaining active processes
static PROC_MAP: Lazy<RwLock<HashMap<usize, DistantRemoteProcess>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

/// Contains mapping of id -> remote lsp process for use in maintaining active processes
static LSP_PROC_MAP: Lazy<RwLock<HashMap<usize, DistantRemoteLspProcess>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

macro_rules! with_proc {
    ($map_name:ident, $id:expr, $proc:ident -> $f:expr) => {{
        let id = $id;
        let mut lock = runtime::get_runtime()?.block_on($map_name.write());
        let $proc = lock.get_mut(&id).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("No remote process found with id {}", id),
            )
            .to_lua_err()
        })?;
        $f
    }};
}

macro_rules! with_proc_async {
    ($map_name:ident, $id:expr, $proc:ident -> $f:expr) => {{
        let id = $id;
        let mut lock = $map_name.write().await;
        let $proc = lock.get_mut(&id).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("No remote process found with id {}", id),
            )
            .to_lua_err()
        })?;
        $f
    }};
}

macro_rules! impl_process {
    ($name:ident, $type:ty, $map_name:ident) => {
        #[derive(Copy, Clone, Debug)]
        pub struct $name {
            id: usize,
        }

        impl $name {
            pub fn new(id: usize) -> Self {
                Self { id }
            }

            pub fn from_distant(proc: $type) -> LuaResult<Self> {
                let id = proc.id();
                runtime::get_runtime()?.block_on($map_name.write()).insert(id, proc);
                Ok(Self::new(id))
            }

            fn is_active(id: usize) -> LuaResult<bool> {
                Ok(runtime::get_runtime()?.block_on($map_name.read()).contains_key(&id))
            }

            fn write_stdin(id: usize, data: String) -> LuaResult<()> {
                runtime::block_on(Self::write_stdin_async(id, data))
            }

            async fn write_stdin_async(id: usize, data: String) -> LuaResult<()> {
                with_proc_async!($map_name, id, proc -> {
                    proc.stdin
                        .as_mut()
                        .ok_or_else(|| {
                            io::Error::new(io::ErrorKind::BrokenPipe, "Stdin closed").to_lua_err()
                        })?
                        .write(data.as_str())
                        .await
                        .to_lua_err()
                })
            }

            fn close_stdin(id: usize) -> LuaResult<()> {
                with_proc!($map_name, id, proc -> {
                    let _ = proc.stdin.take();
                    Ok(())
                })
            }

            fn read_stdout(id: usize) -> LuaResult<Option<String>> {
                with_proc!($map_name, id, proc -> {
                    proc.stdout
                        .as_mut()
                        .ok_or_else(|| {
                            io::Error::new(io::ErrorKind::BrokenPipe, "Stdout closed").to_lua_err()
                        })?
                        .try_read()
                        .to_lua_err()
                })
            }

            async fn read_stdout_async(id: usize) -> LuaResult<String> {
                with_proc_async!($map_name, id, proc -> {
                    proc.stdout
                        .as_mut()
                        .ok_or_else(|| {
                            io::Error::new(io::ErrorKind::BrokenPipe, "Stdout closed").to_lua_err()
                        })?
                        .read()
                        .await
                        .to_lua_err()
                })
            }

            fn read_stderr(id: usize) -> LuaResult<Option<String>> {
                with_proc!($map_name, id, proc -> {
                    proc.stderr
                        .as_mut()
                        .ok_or_else(|| {
                            io::Error::new(io::ErrorKind::BrokenPipe, "Stderr closed").to_lua_err()
                        })?
                        .try_read()
                        .to_lua_err()
                })
            }

            async fn read_stderr_async(id: usize) -> LuaResult<String> {
                with_proc_async!($map_name, id, proc -> {
                    proc.stderr
                        .as_mut()
                        .ok_or_else(|| {
                            io::Error::new(io::ErrorKind::BrokenPipe, "Stderr closed").to_lua_err()
                        })?
                        .read()
                        .await
                        .to_lua_err()
                })
            }

            fn kill(id: usize) -> LuaResult<()> {
                runtime::block_on(Self::kill_async(id))
            }

            async fn kill_async(id: usize) -> LuaResult<()> {
                with_proc_async!($map_name, id, proc -> {
                    proc.kill().await.to_lua_err()
                })
            }

            fn abort(id: usize) -> LuaResult<()> {
                with_proc!($map_name, id, proc -> {
                    Ok(proc.abort())
                })
            }
        }

        impl UserData for $name {
            fn add_fields<'lua, F: UserDataFields<'lua, Self>>(fields: &mut F) {
                fields.add_field_method_get("id", |_, this| Ok(this.id));
            }

            fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
                methods.add_method("is_active", |_, this, ()| Self::is_active(this.id));
                methods.add_method("close_stdin", |_, this, ()| Self::close_stdin(this.id));
                methods.add_method("write_stdin", |_, this, data: String| {
                    Self::write_stdin(this.id, data)
                });
                methods.add_async_method("write_stdin_async", |_, this, data: String| {
                    runtime::spawn(Self::write_stdin_async(this.id, data))
                });
                methods.add_method("read_stdout", |_, this, ()| Self::read_stdout(this.id));
                methods.add_async_method("read_stdout_async", |_, this, ()| {
                    runtime::spawn(Self::read_stdout_async(this.id))
                });
                methods.add_method("read_stderr", |_, this, ()| Self::read_stderr(this.id));
                methods.add_async_method("read_stderr_async", |_, this, ()| {
                    runtime::spawn(Self::read_stderr_async(this.id))
                });
                methods.add_method("kill", |_, this, ()| Self::kill(this.id));
                methods.add_async_method("kill_async", |_, this, ()| {
                    runtime::spawn(Self::kill_async(this.id))
                });
                methods.add_method("abort", |_, this, ()| Self::abort(this.id));
            }
        }
    };
}

impl_process!(RemoteProcess, DistantRemoteProcess, PROC_MAP);
impl_process!(RemoteLspProcess, DistantRemoteLspProcess, LSP_PROC_MAP);
