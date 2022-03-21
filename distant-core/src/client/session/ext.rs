use crate::{
    client::{
        RemoteLspProcess, RemoteProcess, RemoteProcessError, SessionChannel, UnwatchError,
        WatchError, Watcher,
    },
    data::{
        ChangeKindSet, DirEntry, Error as Failure, Metadata, PtySize, Request, RequestData,
        ResponseData, SystemInfo,
    },
    net::TransportError,
};
use derive_more::{Display, Error, From};
use std::{future::Future, path::PathBuf, pin::Pin};

/// Represents an error that can occur related to convenience functions tied to a
/// [`SessionChannel`] through [`SessionChannelExt`]
#[derive(Debug, Display, Error, From)]
pub enum SessionChannelExtError {
    /// Occurs when the remote action fails
    Failure(#[error(not(source))] Failure),

    /// Occurs when a transport error is encountered
    TransportError(TransportError),

    /// Occurs when receiving a response that was not expected
    MismatchedResponse,
}

pub type AsyncReturn<'a, T, E = SessionChannelExtError> =
    Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'a>>;

/// Provides convenience functions on top of a [`SessionChannel`]
pub trait SessionChannelExt {
    /// Appends to a remote file using the data from a collection of bytes
    fn append_file(
        &mut self,
        tenant: impl Into<String>,
        path: impl Into<PathBuf>,
        data: impl Into<Vec<u8>>,
    ) -> AsyncReturn<'_, ()>;

    /// Appends to a remote file using the data from a string
    fn append_file_text(
        &mut self,
        tenant: impl Into<String>,
        path: impl Into<PathBuf>,
        data: impl Into<String>,
    ) -> AsyncReturn<'_, ()>;

    /// Copies a remote file or directory from src to dst
    fn copy(
        &mut self,
        tenant: impl Into<String>,
        src: impl Into<PathBuf>,
        dst: impl Into<PathBuf>,
    ) -> AsyncReturn<'_, ()>;

    /// Creates a remote directory, optionally creating all parent components if specified
    fn create_dir(
        &mut self,
        tenant: impl Into<String>,
        path: impl Into<PathBuf>,
        all: bool,
    ) -> AsyncReturn<'_, ()>;

    /// Checks if a path exists on a remote machine
    fn exists(
        &mut self,
        tenant: impl Into<String>,
        path: impl Into<PathBuf>,
    ) -> AsyncReturn<'_, bool>;

    /// Retrieves metadata about a path on a remote machine
    fn metadata(
        &mut self,
        tenant: impl Into<String>,
        path: impl Into<PathBuf>,
        canonicalize: bool,
        resolve_file_type: bool,
    ) -> AsyncReturn<'_, Metadata>;

    /// Reads entries from a directory, returning a tuple of directory entries and failures
    fn read_dir(
        &mut self,
        tenant: impl Into<String>,
        path: impl Into<PathBuf>,
        depth: usize,
        absolute: bool,
        canonicalize: bool,
        include_root: bool,
    ) -> AsyncReturn<'_, (Vec<DirEntry>, Vec<Failure>)>;

    /// Reads a remote file as a collection of bytes
    fn read_file(
        &mut self,
        tenant: impl Into<String>,
        path: impl Into<PathBuf>,
    ) -> AsyncReturn<'_, Vec<u8>>;

    /// Returns a remote file as a string
    fn read_file_text(
        &mut self,
        tenant: impl Into<String>,
        path: impl Into<PathBuf>,
    ) -> AsyncReturn<'_, String>;

    /// Removes a remote file or directory, supporting removal of non-empty directories if
    /// force is true
    fn remove(
        &mut self,
        tenant: impl Into<String>,
        path: impl Into<PathBuf>,
        force: bool,
    ) -> AsyncReturn<'_, ()>;

    /// Renames a remote file or directory from src to dst
    fn rename(
        &mut self,
        tenant: impl Into<String>,
        src: impl Into<PathBuf>,
        dst: impl Into<PathBuf>,
    ) -> AsyncReturn<'_, ()>;

    /// Watches a remote file or directory
    fn watch(
        &mut self,
        tenant: impl Into<String>,
        path: impl Into<PathBuf>,
        recursive: bool,
        only: impl Into<ChangeKindSet>,
        except: impl Into<ChangeKindSet>,
    ) -> AsyncReturn<'_, Watcher, WatchError>;

    /// Unwatches a remote file or directory
    fn unwatch(
        &mut self,
        tenant: impl Into<String>,
        path: impl Into<PathBuf>,
    ) -> AsyncReturn<'_, (), UnwatchError>;

    /// Spawns a process on the remote machine
    fn spawn(
        &mut self,
        tenant: impl Into<String>,
        cmd: impl Into<String>,
        args: Vec<impl Into<String>>,
        persist: bool,
        pty: Option<PtySize>,
    ) -> AsyncReturn<'_, RemoteProcess, RemoteProcessError>;

    /// Spawns an LSP process on the remote machine
    fn spawn_lsp(
        &mut self,
        tenant: impl Into<String>,
        cmd: impl Into<String>,
        args: Vec<impl Into<String>>,
        persist: bool,
        pty: Option<PtySize>,
    ) -> AsyncReturn<'_, RemoteLspProcess, RemoteProcessError>;

    /// Retrieves information about the remote system
    fn system_info(&mut self, tenant: impl Into<String>) -> AsyncReturn<'_, SystemInfo>;

    /// Writes a remote file with the data from a collection of bytes
    fn write_file(
        &mut self,
        tenant: impl Into<String>,
        path: impl Into<PathBuf>,
        data: impl Into<Vec<u8>>,
    ) -> AsyncReturn<'_, ()>;

    /// Writes a remote file with the data from a string
    fn write_file_text(
        &mut self,
        tenant: impl Into<String>,
        path: impl Into<PathBuf>,
        data: impl Into<String>,
    ) -> AsyncReturn<'_, ()>;
}

macro_rules! make_body {
    ($self:expr, $tenant:expr, $data:expr, @ok) => {
        make_body!($self, $tenant, $data, |data| {
            match data {
                ResponseData::Ok => Ok(()),
                ResponseData::Error(x) => Err(SessionChannelExtError::Failure(x)),
                _ => Err(SessionChannelExtError::MismatchedResponse),
            }
        })
    };

    ($self:expr, $tenant:expr, $data:expr, $and_then:expr) => {{
        let req = Request::new($tenant, vec![$data]);
        Box::pin(async move {
            $self
                .send(req)
                .await
                .map_err(SessionChannelExtError::from)
                .and_then(|res| {
                    if res.payload.len() == 1 {
                        Ok(res.payload.into_iter().next().unwrap())
                    } else {
                        Err(SessionChannelExtError::MismatchedResponse)
                    }
                })
                .and_then($and_then)
        })
    }};
}

impl SessionChannelExt for SessionChannel {
    fn append_file(
        &mut self,
        tenant: impl Into<String>,
        path: impl Into<PathBuf>,
        data: impl Into<Vec<u8>>,
    ) -> AsyncReturn<'_, ()> {
        make_body!(
            self,
            tenant,
            RequestData::FileAppend { path: path.into(), data: data.into() },
            @ok
        )
    }

    fn append_file_text(
        &mut self,
        tenant: impl Into<String>,
        path: impl Into<PathBuf>,
        data: impl Into<String>,
    ) -> AsyncReturn<'_, ()> {
        make_body!(
            self,
            tenant,
            RequestData::FileAppendText { path: path.into(), text: data.into() },
            @ok
        )
    }

    fn copy(
        &mut self,
        tenant: impl Into<String>,
        src: impl Into<PathBuf>,
        dst: impl Into<PathBuf>,
    ) -> AsyncReturn<'_, ()> {
        make_body!(
            self,
            tenant,
            RequestData::Copy { src: src.into(), dst: dst.into() },
            @ok
        )
    }

    fn create_dir(
        &mut self,
        tenant: impl Into<String>,
        path: impl Into<PathBuf>,
        all: bool,
    ) -> AsyncReturn<'_, ()> {
        make_body!(
            self,
            tenant,
            RequestData::DirCreate { path: path.into(), all },
            @ok
        )
    }

    fn exists(
        &mut self,
        tenant: impl Into<String>,
        path: impl Into<PathBuf>,
    ) -> AsyncReturn<'_, bool> {
        make_body!(
            self,
            tenant,
            RequestData::Exists { path: path.into() },
            |data| match data {
                ResponseData::Exists { value } => Ok(value),
                ResponseData::Error(x) => Err(SessionChannelExtError::Failure(x)),
                _ => Err(SessionChannelExtError::MismatchedResponse),
            }
        )
    }

    fn metadata(
        &mut self,
        tenant: impl Into<String>,
        path: impl Into<PathBuf>,
        canonicalize: bool,
        resolve_file_type: bool,
    ) -> AsyncReturn<'_, Metadata> {
        make_body!(
            self,
            tenant,
            RequestData::Metadata {
                path: path.into(),
                canonicalize,
                resolve_file_type
            },
            |data| match data {
                ResponseData::Metadata(x) => Ok(x),
                ResponseData::Error(x) => Err(SessionChannelExtError::Failure(x)),
                _ => Err(SessionChannelExtError::MismatchedResponse),
            }
        )
    }

    fn read_dir(
        &mut self,
        tenant: impl Into<String>,
        path: impl Into<PathBuf>,
        depth: usize,
        absolute: bool,
        canonicalize: bool,
        include_root: bool,
    ) -> AsyncReturn<'_, (Vec<DirEntry>, Vec<Failure>)> {
        make_body!(
            self,
            tenant,
            RequestData::DirRead {
                path: path.into(),
                depth,
                absolute,
                canonicalize,
                include_root
            },
            |data| match data {
                ResponseData::DirEntries { entries, errors } => Ok((entries, errors)),
                ResponseData::Error(x) => Err(SessionChannelExtError::Failure(x)),
                _ => Err(SessionChannelExtError::MismatchedResponse),
            }
        )
    }

    fn read_file(
        &mut self,
        tenant: impl Into<String>,
        path: impl Into<PathBuf>,
    ) -> AsyncReturn<'_, Vec<u8>> {
        make_body!(
            self,
            tenant,
            RequestData::FileRead { path: path.into() },
            |data| match data {
                ResponseData::Blob { data } => Ok(data),
                ResponseData::Error(x) => Err(SessionChannelExtError::Failure(x)),
                _ => Err(SessionChannelExtError::MismatchedResponse),
            }
        )
    }

    fn read_file_text(
        &mut self,
        tenant: impl Into<String>,
        path: impl Into<PathBuf>,
    ) -> AsyncReturn<'_, String> {
        make_body!(
            self,
            tenant,
            RequestData::FileReadText { path: path.into() },
            |data| match data {
                ResponseData::Text { data } => Ok(data),
                ResponseData::Error(x) => Err(SessionChannelExtError::Failure(x)),
                _ => Err(SessionChannelExtError::MismatchedResponse),
            }
        )
    }

    fn remove(
        &mut self,
        tenant: impl Into<String>,
        path: impl Into<PathBuf>,
        force: bool,
    ) -> AsyncReturn<'_, ()> {
        make_body!(
            self,
            tenant,
            RequestData::Remove { path: path.into(), force },
            @ok
        )
    }

    fn rename(
        &mut self,
        tenant: impl Into<String>,
        src: impl Into<PathBuf>,
        dst: impl Into<PathBuf>,
    ) -> AsyncReturn<'_, ()> {
        make_body!(
            self,
            tenant,
            RequestData::Rename { src: src.into(), dst: dst.into() },
            @ok
        )
    }

    fn watch(
        &mut self,
        tenant: impl Into<String>,
        path: impl Into<PathBuf>,
        recursive: bool,
        only: impl Into<ChangeKindSet>,
        except: impl Into<ChangeKindSet>,
    ) -> AsyncReturn<'_, Watcher, WatchError> {
        let tenant = tenant.into();
        let path = path.into();
        let only = only.into();
        let except = except.into();
        Box::pin(async move {
            Watcher::watch(tenant, self.clone(), path, recursive, only, except).await
        })
    }

    fn unwatch(
        &mut self,
        tenant: impl Into<String>,
        path: impl Into<PathBuf>,
    ) -> AsyncReturn<'_, (), UnwatchError> {
        fn inner_unwatch(
            channel: &mut SessionChannel,
            tenant: impl Into<String>,
            path: impl Into<PathBuf>,
        ) -> AsyncReturn<'_, ()> {
            make_body!(
                channel,
                tenant,
                RequestData::Unwatch { path: path.into() },
                @ok
            )
        }

        let tenant = tenant.into();
        let path = path.into();

        Box::pin(async move {
            inner_unwatch(self, tenant, path)
                .await
                .map_err(UnwatchError::from)
        })
    }

    fn spawn(
        &mut self,
        tenant: impl Into<String>,
        cmd: impl Into<String>,
        args: Vec<impl Into<String>>,
        persist: bool,
        pty: Option<PtySize>,
    ) -> AsyncReturn<'_, RemoteProcess, RemoteProcessError> {
        let tenant = tenant.into();
        let cmd = cmd.into();
        let args = args.into_iter().map(Into::into).collect();
        Box::pin(async move {
            RemoteProcess::spawn(tenant, self.clone(), cmd, args, persist, pty).await
        })
    }

    fn spawn_lsp(
        &mut self,
        tenant: impl Into<String>,
        cmd: impl Into<String>,
        args: Vec<impl Into<String>>,
        persist: bool,
        pty: Option<PtySize>,
    ) -> AsyncReturn<'_, RemoteLspProcess, RemoteProcessError> {
        let tenant = tenant.into();
        let cmd = cmd.into();
        let args = args.into_iter().map(Into::into).collect();
        Box::pin(async move {
            RemoteLspProcess::spawn(tenant, self.clone(), cmd, args, persist, pty).await
        })
    }

    fn system_info(&mut self, tenant: impl Into<String>) -> AsyncReturn<'_, SystemInfo> {
        make_body!(
            self,
            tenant,
            RequestData::SystemInfo {},
            |data| match data {
                ResponseData::SystemInfo(x) => Ok(x),
                ResponseData::Error(x) => Err(SessionChannelExtError::Failure(x)),
                _ => Err(SessionChannelExtError::MismatchedResponse),
            }
        )
    }

    fn write_file(
        &mut self,
        tenant: impl Into<String>,
        path: impl Into<PathBuf>,
        data: impl Into<Vec<u8>>,
    ) -> AsyncReturn<'_, ()> {
        make_body!(
            self,
            tenant,
            RequestData::FileWrite { path: path.into(), data: data.into() },
            @ok
        )
    }

    fn write_file_text(
        &mut self,
        tenant: impl Into<String>,
        path: impl Into<PathBuf>,
        data: impl Into<String>,
    ) -> AsyncReturn<'_, ()> {
        make_body!(
            self,
            tenant,
            RequestData::FileWriteText { path: path.into(), text: data.into() },
            @ok
        )
    }
}
