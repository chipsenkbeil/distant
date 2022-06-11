use crate::{
    client::{
        RemoteLspProcess, RemoteProcess, RemoteProcessError, SessionChannel, UnwatchError,
        WatchError, Watcher,
    },
    data::{
        ChangeKindSet, DirEntry, DistantRequestData, DistantResponseData, Error as Failure,
        Metadata, PtySize, SystemInfo,
    },
};
use async_trait::async_trait;
use derive_more::{Display, Error, From};
use distant_net::{Request, TransportError};
use std::path::PathBuf;

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

pub type SessionChannelExtResult<T> = Result<T, SessionChannelExtError>;

/// Provides convenience functions on top of a [`SessionChannel`]
#[async_trait]
pub trait SessionChannelExt {
    /// Appends to a remote file using the data from a collection of bytes
    async fn append_file(
        &mut self,
        path: impl Into<PathBuf>,
        data: impl Into<Vec<u8>>,
    ) -> SessionChannelExtResult<()>;

    /// Appends to a remote file using the data from a string
    fn append_file_text(
        &mut self,
        path: impl Into<PathBuf>,
        data: impl Into<String>,
    ) -> SessionChannelExtResult<()>;

    /// Copies a remote file or directory from src to dst
    fn copy(
        &mut self,
        src: impl Into<PathBuf>,
        dst: impl Into<PathBuf>,
    ) -> SessionChannelExtResult<()>;

    /// Creates a remote directory, optionally creating all parent components if specified
    fn create_dir(&mut self, path: impl Into<PathBuf>, all: bool) -> SessionChannelExtResult<()>;

    /// Checks if a path exists on a remote machine
    fn exists(&mut self, path: impl Into<PathBuf>) -> SessionChannelExtResult<bool>;

    /// Retrieves metadata about a path on a remote machine
    fn metadata(
        &mut self,
        path: impl Into<PathBuf>,
        canonicalize: bool,
        resolve_file_type: bool,
    ) -> SessionChannelExtResult<Metadata>;

    /// Reads entries from a directory, returning a tuple of directory entries and failures
    fn read_dir(
        &mut self,
        path: impl Into<PathBuf>,
        depth: usize,
        absolute: bool,
        canonicalize: bool,
        include_root: bool,
    ) -> SessionChannelExtResult<(Vec<DirEntry>, Vec<Failure>)>;

    /// Reads a remote file as a collection of bytes
    fn read_file(&mut self, path: impl Into<PathBuf>) -> SessionChannelExtResult<Vec<u8>>;

    /// Returns a remote file as a string
    fn read_file_text(&mut self, path: impl Into<PathBuf>) -> SessionChannelExtResult<String>;

    /// Removes a remote file or directory, supporting removal of non-empty directories if
    /// force is true
    fn remove(&mut self, path: impl Into<PathBuf>, force: bool) -> SessionChannelExtResult<()>;

    /// Renames a remote file or directory from src to dst
    fn rename(
        &mut self,
        src: impl Into<PathBuf>,
        dst: impl Into<PathBuf>,
    ) -> SessionChannelExtResult<()>;

    /// Watches a remote file or directory
    fn watch(
        &mut self,
        path: impl Into<PathBuf>,
        recursive: bool,
        only: impl Into<ChangeKindSet>,
        except: impl Into<ChangeKindSet>,
    ) -> Result<Watcher, WatchError>;

    /// Unwatches a remote file or directory
    fn unwatch(&mut self, path: impl Into<PathBuf>) -> Result<(), UnwatchError>;

    /// Spawns a process on the remote machine
    fn spawn(
        &mut self,
        cmd: impl Into<String>,
        persist: bool,
        pty: Option<PtySize>,
    ) -> Result<RemoteProcess, RemoteProcessError>;

    /// Spawns an LSP process on the remote machine
    fn spawn_lsp(
        &mut self,
        cmd: impl Into<String>,
        persist: bool,
        pty: Option<PtySize>,
    ) -> Result<RemoteLspProcess, RemoteProcessError>;

    /// Retrieves information about the remote system
    fn system_info(&mut self) -> SessionChannelExtResult<SystemInfo>;

    /// Writes a remote file with the data from a collection of bytes
    fn write_file(
        &mut self,
        path: impl Into<PathBuf>,
        data: impl Into<Vec<u8>>,
    ) -> SessionChannelExtResult<()>;

    /// Writes a remote file with the data from a string
    fn write_file_text(
        &mut self,
        path: impl Into<PathBuf>,
        data: impl Into<String>,
    ) -> SessionChannelExtResult<()>;
}

macro_rules! make_body {
    ($self:expr, $data:expr, @ok) => {
        make_body!($self, $data, |data| {
            match data {
                ResponseData::Ok => Ok(()),
                ResponseData::Error(x) => Err(SessionChannelExtError::Failure(x)),
                _ => Err(SessionChannelExtError::MismatchedResponse),
            }
        })
    };

    ($self:expr, $data:expr, $and_then:expr) => {{
        let req = Request::new(vec![$data]);
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

#[async_trait]
impl<T> SessionChannelExt for SessionChannel<T> {
    fn append_file(
        &mut self,
        path: impl Into<PathBuf>,
        data: impl Into<Vec<u8>>,
    ) -> SessionChannelExtResult<()> {
        make_body!(
            self,
            DistantRequestData::FileAppend { path: path.into(), data: data.into() },
            @ok
        )
    }

    fn append_file_text(
        &mut self,
        path: impl Into<PathBuf>,
        data: impl Into<String>,
    ) -> SessionChannelExtResult<()> {
        make_body!(
            self,
            DistantRequestData::FileAppendText { path: path.into(), text: data.into() },
            @ok
        )
    }

    fn copy(
        &mut self,
        src: impl Into<PathBuf>,
        dst: impl Into<PathBuf>,
    ) -> SessionChannelExtResult<()> {
        make_body!(
            self,
            DistantRequestData::Copy { src: src.into(), dst: dst.into() },
            @ok
        )
    }

    fn create_dir(&mut self, path: impl Into<PathBuf>, all: bool) -> SessionChannelExtResult<()> {
        make_body!(
            self,
            DistantRequestData::DirCreate { path: path.into(), all },
            @ok
        )
    }

    fn exists(&mut self, path: impl Into<PathBuf>) -> SessionChannelExtResult<bool> {
        make_body!(
            self,
            DistantRequestData::Exists { path: path.into() },
            |data| match data {
                DistantResponseData::Exists { value } => Ok(value),
                DistantResponseData::Error(x) => Err(SessionChannelExtError::Failure(x)),
                _ => Err(SessionChannelExtError::MismatchedResponse),
            }
        )
    }

    fn metadata(
        &mut self,
        path: impl Into<PathBuf>,
        canonicalize: bool,
        resolve_file_type: bool,
    ) -> SessionChannelExtResult<Metadata> {
        make_body!(
            self,
            DistantRequestData::Metadata {
                path: path.into(),
                canonicalize,
                resolve_file_type
            },
            |data| match data {
                DistantResponseData::Metadata(x) => Ok(x),
                DistantResponseData::Error(x) => Err(SessionChannelExtError::Failure(x)),
                _ => Err(SessionChannelExtError::MismatchedResponse),
            }
        )
    }

    fn read_dir(
        &mut self,
        path: impl Into<PathBuf>,
        depth: usize,
        absolute: bool,
        canonicalize: bool,
        include_root: bool,
    ) -> SessionChannelExtResult<(Vec<DirEntry>, Vec<Failure>)> {
        make_body!(
            self,
            DistantRequestData::DirRead {
                path: path.into(),
                depth,
                absolute,
                canonicalize,
                include_root
            },
            |data| match data {
                DistantResponseData::DirEntries { entries, errors } => Ok((entries, errors)),
                DistantResponseData::Error(x) => Err(SessionChannelExtError::Failure(x)),
                _ => Err(SessionChannelExtError::MismatchedResponse),
            }
        )
    }

    fn read_file(&mut self, path: impl Into<PathBuf>) -> SessionChannelExtResult<Vec<u8>> {
        make_body!(
            self,
            DistantRequestData::FileRead { path: path.into() },
            |data| match data {
                DistantResponseData::Blob { data } => Ok(data),
                DistantResponseData::Error(x) => Err(SessionChannelExtError::Failure(x)),
                _ => Err(SessionChannelExtError::MismatchedResponse),
            }
        )
    }

    fn read_file_text(&mut self, path: impl Into<PathBuf>) -> SessionChannelExtResult<String> {
        make_body!(
            self,
            DistantRequestData::FileReadText { path: path.into() },
            |data| match data {
                DistantResponseData::Text { data } => Ok(data),
                DistantResponseData::Error(x) => Err(SessionChannelExtError::Failure(x)),
                _ => Err(SessionChannelExtError::MismatchedResponse),
            }
        )
    }

    fn remove(&mut self, path: impl Into<PathBuf>, force: bool) -> SessionChannelExtResult<()> {
        make_body!(
            self,
            DistantRequestData::Remove { path: path.into(), force },
            @ok
        )
    }

    fn rename(
        &mut self,
        src: impl Into<PathBuf>,
        dst: impl Into<PathBuf>,
    ) -> SessionChannelExtResult<()> {
        make_body!(
            self,
            DistantRequestData::Rename { src: src.into(), dst: dst.into() },
            @ok
        )
    }

    fn watch(
        &mut self,
        path: impl Into<PathBuf>,
        recursive: bool,
        only: impl Into<ChangeKindSet>,
        except: impl Into<ChangeKindSet>,
    ) -> Result<Watcher, WatchError> {
        let path = path.into();
        let only = only.into();
        let except = except.into();
        Box::pin(async move { Watcher::watch(self.clone(), path, recursive, only, except).await })
    }

    fn unwatch(&mut self, path: impl Into<PathBuf>) -> Result<(), UnwatchError> {
        fn inner_unwatch<T>(
            channel: &mut SessionChannel<T>,
            path: impl Into<PathBuf>,
        ) -> SessionChannelExtResult<()> {
            make_body!(
                channel,
                DistantRequestData::Unwatch { path: path.into() },
                @ok
            )
        }

        let path = path.into();

        Box::pin(async move { inner_unwatch(self, path).await.map_err(UnwatchError::from) })
    }

    fn spawn(
        &mut self,
        cmd: impl Into<String>,
        args: Vec<impl Into<String>>,
        persist: bool,
        pty: Option<PtySize>,
    ) -> Result<RemoteProcess, RemoteProcessError> {
        let cmd = cmd.into();
        let args = args.into_iter().map(Into::into).collect();
        Box::pin(async move { RemoteProcess::spawn(self.clone(), cmd, args, persist, pty).await })
    }

    fn spawn_lsp(
        &mut self,
        cmd: impl Into<String>,
        args: Vec<impl Into<String>>,
        persist: bool,
        pty: Option<PtySize>,
    ) -> Result<RemoteLspProcess, RemoteProcessError> {
        let cmd = cmd.into();
        let args = args.into_iter().map(Into::into).collect();
        Box::pin(
            async move { RemoteLspProcess::spawn(self.clone(), cmd, args, persist, pty).await },
        )
    }

    fn system_info(&mut self) -> SessionChannelExtResult<SystemInfo> {
        make_body!(self, DistantRequestData::SystemInfo {}, |data| match data {
            DistantResponseData::SystemInfo(x) => Ok(x),
            DistantResponseData::Error(x) => Err(SessionChannelExtError::Failure(x)),
            _ => Err(SessionChannelExtError::MismatchedResponse),
        })
    }

    fn write_file(
        &mut self,
        path: impl Into<PathBuf>,
        data: impl Into<Vec<u8>>,
    ) -> SessionChannelExtResult<()> {
        make_body!(
            self,
            DistantRequestData::FileWrite { path: path.into(), data: data.into() },
            @ok
        )
    }

    fn write_file_text(
        &mut self,
        path: impl Into<PathBuf>,
        data: impl Into<String>,
    ) -> SessionChannelExtResult<()> {
        make_body!(
            self,
            DistantRequestData::FileWriteText { path: path.into(), text: data.into() },
            @ok
        )
    }
}
