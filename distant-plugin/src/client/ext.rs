use async_trait::async_trait;

use std::io;
use std::path::PathBuf;

use super::{Client, ClientError, ClientResult};
use crate::common::{Request, RequestFlags, Response};
use crate::protocol;

macro_rules! impl_client_fn {
    ($this:ident, $res:expr, $payload:expr) => {{
        let id = rand::random();
        let request = Request {
            id,
            flags: RequestFlags {
                sequence: false,
                ..Default::default()
            },
            payload: protocol::Msg::Single($payload),
        };

        let response = $this.ask(request).await?;
        if response.origin != id {
            return Err(ClientError::WrongOrigin {
                expected: id,
                actual: response.origin,
            });
        }

        match response.payload {
            protocol::Msg::Single(protocol::Response::Error(x)) => Err(ClientError::Server(x)),
            protocol::Msg::Single(x) if protocol::ResponseKind::from(&x) == $res => Ok(()),
            protocol::Msg::Single(x) => Err(ClientError::WrongPayloadType {
                expected: &[$res.into()],
                actual: x.into(),
            }),
            protocol::Msg::Batch(_) => Err(ClientError::WrongPayloadFormat),
        }
    }};
}

/// Provides convenience functions on top of a [`Client`].
#[async_trait]
pub trait ClientExt: Client {
    /// Appends to a remote file using the data from a collection of bytes.
    async fn append_file(
        &mut self,
        path: impl Into<PathBuf> + Send,
        data: impl Into<Vec<u8>> + Send,
    ) -> ClientResult<()> {
        impl_client_fn!(
            self,
            protocol::ResponseKind::Ok,
            protocol::Request::FileAppend {
                path: path.into(),
                data: data.into(),
            }
        )
    }

    /// Appends to a remote file using the data from a string.
    async fn append_file_text(
        &mut self,
        path: impl Into<PathBuf> + Send,
        text: impl Into<String> + Send,
    ) -> ClientResult<()> {
        impl_client_fn!(
            self,
            protocol::ResponseKind::Ok,
            protocol::Request::FileAppendText {
                path: path.into(),
                text: text.into(),
            }
        )
    }

    /// Copies a remote file or directory from src to dst.
    async fn copy(
        &mut self,
        src: impl Into<PathBuf> + Send,
        dst: impl Into<PathBuf> + Send,
    ) -> ClientResult<()> {
        impl_client_fn!(
            self,
            protocol::ResponseKind::Ok,
            protocol::Request::Copy {
                src: src.into(),
                dst: dst.into(),
            }
        )
    }

    /// Creates a remote directory, optionally creating all parent components if specified.
    async fn create_dir(&mut self, path: impl Into<PathBuf> + Send, all: bool) -> ClientResult<()> {
        impl_client_fn!(
            self,
            protocol::ResponseKind::Ok,
            protocol::Request::DirCreate {
                path: path.into(),
                all,
            }
        )
    }

    /// Checks whether the `path` exists on the remote machine.
    async fn exists(&mut self, path: impl Into<PathBuf>) -> ClientResult<bool> {
        impl_client_fn!(
            self,
            protocol::ResponseKind::Exists,
            protocol::Request::Exists { path: path.into() }
        )
    }

    /// Checks whether this client is compatible with the remote server.
    async fn is_compatible(&mut self) -> io::Result<bool>;

    /// Retrieves metadata about a path on a remote machine.
    async fn metadata(
        &mut self,
        path: impl Into<PathBuf>,
        canonicalize: bool,
        resolve_file_type: bool,
    ) -> io::Result<protocol::Metadata>;

    /// Sets permissions for a path on a remote machine.
    async fn set_permissions(
        &mut self,
        path: impl Into<PathBuf>,
        permissions: protocol::Permissions,
        options: protocol::SetPermissionsOptions,
    ) -> io::Result<()>;

    /// Perform a search.
    async fn search(
        &mut self,
        query: impl Into<protocol::SearchQuery>,
    ) -> io::Result<protocol::Searcher>;

    /// Cancel an active search query.
    async fn cancel_search(&mut self, id: protocol::SearchId) -> io::Result<()>;

    /// Reads entries from a directory, returning a tuple of directory entries and errors.
    async fn read_dir(
        &mut self,
        path: impl Into<PathBuf>,
        depth: usize,
        absolute: bool,
        canonicalize: bool,
        include_root: bool,
    ) -> io::Result<(Vec<protocol::DirEntry>, Vec<protocol::Error>)>;

    /// Reads a remote file as a collection of bytes.
    async fn read_file(&mut self, path: impl Into<PathBuf>) -> io::Result<Vec<u8>>;

    /// Returns a remote file as a string.
    async fn read_file_text(&mut self, path: impl Into<PathBuf>) -> io::Result<String>;

    /// Removes a remote file or directory, supporting removal of non-empty directories if
    /// force is true.
    async fn remove(&mut self, path: impl Into<PathBuf>, force: bool) -> io::Result<()>;

    /// Renames a remote file or directory from src to dst.
    async fn rename(&mut self, src: impl Into<PathBuf>, dst: impl Into<PathBuf>) -> io::Result<()>;

    /// Watches a remote file or directory.
    async fn watch(
        &mut self,
        path: impl Into<PathBuf>,
        recursive: bool,
        only: impl Into<protocol::ChangeKindSet>,
        except: impl Into<protocol::ChangeKindSet>,
    ) -> io::Result<Watcher>;

    /// Unwatches a remote file or directory.
    async fn unwatch(&mut self, path: impl Into<PathBuf>) -> io::Result<()>;

    /// Spawns a process on the remote machine.
    async fn spawn(
        &mut self,
        cmd: impl Into<String>,
        environment: Environment,
        current_dir: Option<PathBuf>,
        pty: Option<protocol::PtySize>,
    ) -> io::Result<RemoteProcess>;

    /// Spawns an LSP process on the remote machine.
    async fn spawn_lsp(
        &mut self,
        cmd: impl Into<String>,
        environment: Environment,
        current_dir: Option<PathBuf>,
        pty: Option<protocol::PtySize>,
    ) -> io::Result<RemoteLspProcess>;

    /// Spawns a process on the remote machine and wait for it to complete.
    async fn output(
        &mut self,
        cmd: impl Into<String>,
        environment: Environment,
        current_dir: Option<PathBuf>,
        pty: Option<protocol::PtySize>,
    ) -> io::Result<RemoteOutput>;

    /// Retrieves information about the remote system.
    async fn system_info(&mut self) -> io::Result<protocol::SystemInfo>;

    /// Retrieves server version information.
    async fn version(&mut self) -> io::Result<protocol::Version>;

    /// Returns version of protocol that the client uses.
    async fn protocol_version(&self) -> protocol::semver::Version;

    /// Writes a remote file with the data from a collection of bytes.
    async fn write_file(
        &mut self,
        path: impl Into<PathBuf>,
        data: impl Into<Vec<u8>>,
    ) -> io::Result<()>;

    /// Writes a remote file with the data from a string.
    async fn write_file_text(
        &mut self,
        path: impl Into<PathBuf>,
        data: impl Into<String>,
    ) -> io::Result<()>;
}

impl<T: Client + ?Sized> ClientExt for T {}
