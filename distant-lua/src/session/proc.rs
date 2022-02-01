use crate::{constants::PROC_POLL_TIMEOUT, runtime};
use distant_core::{
    data::PtySize, RemoteLspProcess as DistantRemoteLspProcess,
    RemoteProcess as DistantRemoteProcess,
};
use mlua::{prelude::*, UserData, UserDataFields, UserDataMethods};
use once_cell::sync::Lazy;
use std::{collections::HashMap, io, time::Duration};
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
            pub(crate) id: usize,
        }

        impl $name {
            pub fn new(id: usize) -> Self {
                Self { id }
            }

            pub fn from_distant(proc: $type) -> LuaResult<Self> {
                runtime::get_runtime()?.block_on(Self::from_distant_async(proc))
            }

            pub async fn from_distant_async(proc: $type) -> LuaResult<Self> {
                let id = proc.id();
                $map_name.write().await.insert(id, proc);
                Ok(Self::new(id))
            }

            fn is_active(id: usize) -> LuaResult<bool> {
                Ok(runtime::get_runtime()?.block_on($map_name.read()).contains_key(&id))
            }

            fn write_stdin(id: usize, data: String) -> LuaResult<()> {
                runtime::block_on(Self::write_stdin_async(id, data))
            }

            async fn write_stdin_async(id: usize, data: String) -> LuaResult<()> {
                // NOTE: We must spawn a task that continually tries to send stdin as
                //       if we wait until successful then we hold the lock the entire time
                runtime::spawn(async move {
                    loop {
                        let is_done = with_proc_async!($map_name, id, proc -> {
                            let res = proc.stdin
                                .as_mut()
                                .ok_or_else(|| {
                                    io::Error::new(io::ErrorKind::BrokenPipe, "Stdin closed").to_lua_err()
                                })?
                                .try_write(data.as_bytes());
                            match res {
                                Ok(_) => Ok(true),
                                Err(x) if x.kind() == io::ErrorKind::WouldBlock => Ok(false),
                                Err(x) => Err(x),
                            }
                        })?;

                        if is_done {
                            break;
                        }

                        tokio::time::sleep(Duration::from_millis(PROC_POLL_TIMEOUT)).await;
                    }

                    Ok(())
                }).await
            }

            fn close_stdin(id: usize) -> LuaResult<()> {
                with_proc!($map_name, id, proc -> {
                    let _ = proc.stdin.take();
                    Ok(())
                })
            }

            fn read_stdout(id: usize) -> LuaResult<Option<Vec<u8>>> {
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

            async fn read_stdout_async(id: usize) -> LuaResult<Vec<u8>> {
                // NOTE: We must spawn a task that continually tries to read stdout as
                //       if we wait until successful then we hold the lock the entire time
                runtime::spawn(async move {
                    loop {
                        let data = with_proc_async!($map_name, id, proc -> {
                            proc.stdout
                                .as_mut()
                                .ok_or_else(|| {
                                    io::Error::new(io::ErrorKind::BrokenPipe, "Stdout closed").to_lua_err()
                                })?
                                .try_read()
                                .to_lua_err()?
                        });

                        if let Some(data) = data {
                            break Ok(data);
                        }
                    }
                }).await
            }

            fn read_stderr(id: usize) -> LuaResult<Option<Vec<u8>>> {
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

            async fn read_stderr_async(id: usize) -> LuaResult<Vec<u8>> {
                // NOTE: We must spawn a task that continually tries to read stdout as
                //       if we wait until successful then we hold the lock the entire time
                runtime::spawn(async move {
                    loop {
                        let data = with_proc_async!($map_name, id, proc -> {
                            proc.stderr
                                .as_mut()
                                .ok_or_else(|| {
                                    io::Error::new(io::ErrorKind::BrokenPipe, "Stderr closed").to_lua_err()
                                })?
                                .try_read()
                                .to_lua_err()?
                        });

                        if let Some(data) = data {
                            break Ok(data);
                        }
                    }
                }).await
            }

            fn resize(id: usize, size: PtySize) -> LuaResult<()> {
                runtime::block_on(Self::resize_async(id, size))
            }

            async fn resize_async(id: usize, size: PtySize) -> LuaResult<()> {
                with_proc_async!($map_name, id, proc -> {
                    proc.resize(size).await.to_lua_err()
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

            fn status(id: usize) -> LuaResult<Option<Status>> {
                runtime::block_on(Self::status_async(id))
            }

            async fn status_async(id: usize) -> LuaResult<Option<Status>> {
                let lock = $map_name.read().await;
                let proc = lock.get(&id).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("No remote process found with id {}", id),
                    )
                    .to_lua_err()
                })?;
                Ok(proc.status().await.map(|(success, exit_code)| Status {
                    success,
                    exit_code,
                }))
            }

            fn wait(id: usize) -> LuaResult<(bool, Option<i32>)> {
                runtime::block_on(Self::wait_async(id))
            }

            async fn wait_async(id: usize) -> LuaResult<(bool, Option<i32>)> {
                let proc = $map_name.write().await.remove(&id).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("No remote process found with id {}", id),
                    )
                    .to_lua_err()
                })?;

                proc.wait().await.to_lua_err()
            }

            fn output(id: usize) -> LuaResult<Output> {
                runtime::block_on(Self::output_async(id))
            }

            pub(crate) async fn output_async(id: usize) -> LuaResult<Output> {
                let mut proc = $map_name.write().await.remove(&id).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("No remote process found with id {}", id),
                    )
                    .to_lua_err()
                })?;

                // Remove the stdout and stderr streams before letting process run to completion
                let mut stdout = proc.stdout.take().unwrap();
                let mut stderr = proc.stderr.take().unwrap();

                // Gather stdout and stderr after process completes
                let (success, exit_code) = proc.wait().await.to_lua_err()?;

                let mut stdout_buf = Vec::new();
                while let Ok(Some(data)) = stdout.try_read() {
                    stdout_buf.extend(data);
                }

                let mut stderr_buf = Vec::new();
                while let Ok(Some(data)) = stderr.try_read() {
                    stderr_buf.extend(data);
                }

                Ok(Output {
                    success,
                    exit_code,
                    stdout: stdout_buf,
                    stderr: stderr_buf,
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
                methods.add_method("status", |_, this, ()| Self::status(this.id));
                methods.add_async_method("status_async", |_, this, ()| {
                    runtime::spawn(Self::status_async(this.id))
                });
                methods.add_method("wait", |_, this, ()| Self::wait(this.id));
                methods.add_async_method("wait_async", |_, this, ()| {
                    runtime::spawn(Self::wait_async(this.id))
                });
                methods.add_method("output", |_, this, ()| Self::output(this.id));
                methods.add_async_method("output_async", |_, this, ()| {
                    runtime::spawn(Self::output_async(this.id))
                });
                methods.add_method("resize", |lua, this, value: LuaValue| {
                    let size: PtySize = lua.from_value(value)?;
                    Self::resize(this.id, size)
                });
                methods.add_async_method("resize_async", |lua, this, value: LuaValue| {
                    let size: LuaResult<PtySize> = lua.from_value(value);
                    runtime::spawn(async move {
                        let size = size?;
                        Self::resize_async(this.id, size).await
                    })
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

/// Represents process status
#[derive(Clone, Debug)]
pub struct Status {
    pub success: bool,
    pub exit_code: Option<i32>,
}

impl UserData for Status {
    fn add_fields<'lua, F: UserDataFields<'lua, Self>>(fields: &mut F) {
        fields.add_field_method_get("success", |_, this| Ok(this.success));
        fields.add_field_method_get("exit_code", |_, this| Ok(this.exit_code));
    }

    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method("to_tbl", |lua, this, ()| {
            let tbl = lua.create_table()?;
            tbl.set("success", this.success)?;
            tbl.set("exit_code", this.exit_code)?;
            Ok(tbl)
        });
    }
}

/// Represents process output
#[derive(Clone, Debug)]
pub struct Output {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl UserData for Output {
    fn add_fields<'lua, F: UserDataFields<'lua, Self>>(fields: &mut F) {
        fields.add_field_method_get("success", |_, this| Ok(this.success));
        fields.add_field_method_get("exit_code", |_, this| Ok(this.exit_code));
        fields.add_field_method_get("stdout", |_, this| Ok(this.stdout.to_vec()));
        fields.add_field_method_get("stderr", |_, this| Ok(this.stderr.to_vec()));
    }

    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method("to_tbl", |lua, this, ()| {
            let tbl = lua.create_table()?;
            tbl.set("success", this.success)?;
            tbl.set("exit_code", this.exit_code)?;
            tbl.set("stdout", this.stdout.to_vec())?;
            tbl.set("stderr", this.stdout.to_vec())?;
            Ok(tbl)
        });
    }
}

impl_process!(RemoteProcess, DistantRemoteProcess, PROC_MAP);
impl_process!(RemoteLspProcess, DistantRemoteLspProcess, LSP_PROC_MAP);
