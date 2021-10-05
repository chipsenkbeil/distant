use crate::runtime;
use distant_core::{
    RemoteLspProcess as DistantRemoteLspProcess, RemoteProcess as DistantRemoteProcess,
};
use mlua::{prelude::*, UserData, UserDataFields, UserDataMethods};
use once_cell::sync::Lazy;
use std::{collections::HashMap, io, sync::RwLock};

/// Contains mapping of id -> remote process for use in maintaining active processes
static PROC_MAP: Lazy<RwLock<HashMap<usize, DistantRemoteProcess>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

/// Contains mapping of id -> remote lsp process for use in maintaining active processes
static LSP_PROC_MAP: Lazy<RwLock<HashMap<usize, DistantRemoteLspProcess>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

macro_rules! with_proc {
    ($map_name:ident, $id:expr, $proc:ident -> $f:expr) => {{
        let id = $id;
        let mut lock = $map_name.write().map_err(|x| x.to_string().to_lua_err())?;
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
                $map_name
                    .write()
                    .map_err(|x| x.to_string().to_lua_err())?
                    .insert(id, proc);
                Ok(Self::new(id))
            }

            pub fn is_active(&self) -> LuaResult<bool> {
                Ok($map_name
                    .read()
                    .map_err(|x| x.to_string().to_lua_err())?
                    .contains_key(&self.id))
            }

            pub fn write_stdin(&self, data: String) -> LuaResult<()> {
                runtime::block_on(self.write_stdin_async(data))
            }

            async fn write_stdin_async(&self, data: String) -> LuaResult<()> {
                with_proc!($map_name, self.id, proc -> {
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

            pub fn close_stdin(&self) -> LuaResult<()> {
                with_proc!($map_name, self.id, proc -> {
                    let _ = proc.stdin.take();
                    Ok(())
                })
            }

            pub fn read_stdout(&self) -> LuaResult<Option<String>> {
                with_proc!($map_name, self.id, proc -> {
                    proc.stdout
                        .as_mut()
                        .ok_or_else(|| {
                            io::Error::new(io::ErrorKind::BrokenPipe, "Stdout closed").to_lua_err()
                        })?
                        .try_read()
                        .to_lua_err()
                })
            }

            pub fn read_stderr(&self) -> LuaResult<Option<String>> {
                with_proc!($map_name, self.id, proc -> {
                    proc.stderr
                        .as_mut()
                        .ok_or_else(|| {
                            io::Error::new(io::ErrorKind::BrokenPipe, "Stderr closed").to_lua_err()
                        })?
                        .try_read()
                        .to_lua_err()
                })
            }

            pub fn kill(&self) -> LuaResult<()> {
                runtime::block_on(self.kill_async())
            }

            async fn kill_async(&self) -> LuaResult<()> {
                with_proc!($map_name, self.id, proc -> {
                    proc.kill().await.to_lua_err()
                })
            }

            pub fn abort(&self) -> LuaResult<()> {
                with_proc!($map_name, self.id, proc -> {
                    Ok(proc.abort())
                })
            }
        }

        impl UserData for $name {
            fn add_fields<'lua, F: UserDataFields<'lua, Self>>(fields: &mut F) {
                fields.add_field_method_get("id", |_, this| Ok(this.id));
            }

            fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
                methods.add_method("is_active", |_, this, ()| this.is_active());
                methods.add_method("close_stdin", |_, this, ()| this.close_stdin());
                methods.add_method("write_stdin", |_, this, data: String| {
                    this.write_stdin(data)
                });
                methods.add_method("read_stdout", |_, this, ()| this.read_stdout());
                methods.add_method("read_stderr", |_, this, ()| this.read_stderr());
                methods.add_method("kill", |_, this, ()| this.kill());
                methods.add_method("abort", |_, this, ()| this.abort());
            }
        }
    };
}

impl_process!(RemoteProcess, DistantRemoteProcess, PROC_MAP);
impl_process!(RemoteLspProcess, DistantRemoteLspProcess, LSP_PROC_MAP);
