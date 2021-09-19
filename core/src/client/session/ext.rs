use crate::{
    client::{RemoteProcess, RemoteProcessError, Session},
    data::{DirEntry, Error as Failure, FileType, Request, RequestData, ResponseData},
    net::TransportError,
};
use derive_more::{Display, Error, From};
use std::{future::Future, path::PathBuf, pin::Pin};

/// Represents an error that can occur related to convenience functions tied to a [`Session`]
#[derive(Debug, Display, Error, From)]
pub enum SessionExtError {
    /// Occurs when the remote action fails
    Failure(#[error(not(source))] Failure),

    /// Occurs when a transport error is encountered
    TransportError(TransportError),

    /// Occurs when receiving a response that was not expected
    MismatchedResponse,
}

pub type AsyncReturn<'a, T, E = SessionExtError> =
    Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'a>>;

/// Represents metadata about some path on a remote machine
pub struct Metadata {
    pub file_type: FileType,
    pub len: u64,
    pub readonly: bool,

    pub canonicalized_path: Option<PathBuf>,

    pub accessed: Option<u128>,
    pub created: Option<u128>,
    pub modified: Option<u128>,
}

/// Provides convenience functions on top of a [`Session`]
pub trait SessionExt {
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

    /// Spawns a process on the remote machine
    fn spawn(
        &mut self,
        tenant: impl Into<String>,
        cmd: impl Into<String>,
        args: Vec<String>,
    ) -> AsyncReturn<'_, RemoteProcess, RemoteProcessError>;

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
            if data.is_ok() {
                Ok(())
            } else {
                Err(SessionExtError::MismatchedResponse)
            }
        })
    };

    ($self:expr, $tenant:expr, $data:expr, $and_then:expr) => {{
        let req = Request::new($tenant, vec![$data]);
        Box::pin(async move {
            $self
                .send(req)
                .await
                .map_err(SessionExtError::from)
                .and_then(|res| {
                    if res.payload.len() == 1 {
                        Ok(res.payload.into_iter().next().unwrap())
                    } else {
                        Err(SessionExtError::MismatchedResponse)
                    }
                })
                .and_then($and_then)
        })
    }};
}

impl SessionExt for Session {
    /// Appends to a remote file using the data from a collection of bytes
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

    /// Appends to a remote file using the data from a string
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

    /// Copies a remote file or directory from src to dst
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

    /// Creates a remote directory, optionally creating all parent components if specified
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

    /// Checks if a path exists on a remote machine
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
                ResponseData::Exists(x) => Ok(x),
                _ => Err(SessionExtError::MismatchedResponse),
            }
        )
    }

    /// Retrieves metadata about a path on a remote machine
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
                ResponseData::Metadata {
                    canonicalized_path,
                    file_type,
                    len,
                    readonly,
                    accessed,
                    created,
                    modified,
                } => Ok(Metadata {
                    canonicalized_path,
                    file_type,
                    len,
                    readonly,
                    accessed,
                    created,
                    modified,
                }),
                _ => Err(SessionExtError::MismatchedResponse),
            }
        )
    }

    /// Reads entries from a directory, returning a tuple of directory entries and failures
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
                _ => Err(SessionExtError::MismatchedResponse),
            }
        )
    }

    /// Reads a remote file as a collection of bytes
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
                _ => Err(SessionExtError::MismatchedResponse),
            }
        )
    }

    /// Returns a remote file as a string
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
                _ => Err(SessionExtError::MismatchedResponse),
            }
        )
    }

    /// Removes a remote file or directory, supporting removal of non-empty directories if
    /// force is true
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

    /// Renames a remote file or directory from src to dst
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

    /// Spawns a process on the remote machine
    fn spawn(
        &mut self,
        tenant: impl Into<String>,
        cmd: impl Into<String>,
        args: Vec<String>,
    ) -> AsyncReturn<'_, RemoteProcess, RemoteProcessError> {
        let tenant = tenant.into();
        let cmd = cmd.into();
        Box::pin(async move { RemoteProcess::spawn(tenant, self, cmd, args).await })
    }

    /// Writes a remote file with the data from a collection of bytes
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

    /// Writes a remote file with the data from a string
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
