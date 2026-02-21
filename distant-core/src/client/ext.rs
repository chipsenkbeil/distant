use std::future::Future;
use std::io;
use std::path::PathBuf;
use std::pin::Pin;

use crate::net::client::Channel;
use crate::net::common::Request;

use crate::client::{
    RemoteCommand, RemoteLspCommand, RemoteLspProcess, RemoteOutput, RemoteProcess, Searcher,
    Watcher,
};
use crate::protocol::{
    self, ChangeKindSet, DirEntry, Environment, Error as Failure, Metadata, Permissions, PtySize,
    SearchId, SearchQuery, SetPermissionsOptions, SystemInfo, Version,
};

pub type AsyncReturn<'a, T, E = io::Error> =
    Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'a>>;

fn mismatched_response() -> io::Error {
    io::Error::other("Mismatched response")
}

/// Provides convenience functions on top of a [`Channel`]
pub trait DistantChannelExt {
    /// Appends to a remote file using the data from a collection of bytes
    fn append_file(
        &mut self,
        path: impl Into<PathBuf>,
        data: impl Into<Vec<u8>>,
    ) -> AsyncReturn<'_, ()>;

    /// Appends to a remote file using the data from a string
    fn append_file_text(
        &mut self,
        path: impl Into<PathBuf>,
        data: impl Into<String>,
    ) -> AsyncReturn<'_, ()>;

    /// Copies a remote file or directory from src to dst
    fn copy(&mut self, src: impl Into<PathBuf>, dst: impl Into<PathBuf>) -> AsyncReturn<'_, ()>;

    /// Creates a remote directory, optionally creating all parent components if specified
    fn create_dir(&mut self, path: impl Into<PathBuf>, all: bool) -> AsyncReturn<'_, ()>;

    /// Checks whether the `path` exists on the remote machine
    fn exists(&mut self, path: impl Into<PathBuf>) -> AsyncReturn<'_, bool>;

    /// Checks whether this client is compatible with the remote server
    fn is_compatible(&mut self) -> AsyncReturn<'_, bool>;

    /// Retrieves metadata about a path on a remote machine
    fn metadata(
        &mut self,
        path: impl Into<PathBuf>,
        canonicalize: bool,
        resolve_file_type: bool,
    ) -> AsyncReturn<'_, Metadata>;

    /// Sets permissions for a path on a remote machine
    fn set_permissions(
        &mut self,
        path: impl Into<PathBuf>,
        permissions: Permissions,
        options: SetPermissionsOptions,
    ) -> AsyncReturn<'_, ()>;

    /// Perform a search
    fn search(&mut self, query: impl Into<SearchQuery>) -> AsyncReturn<'_, Searcher>;

    /// Cancel an active search query
    fn cancel_search(&mut self, id: SearchId) -> AsyncReturn<'_, ()>;

    /// Reads entries from a directory, returning a tuple of directory entries and failures
    fn read_dir(
        &mut self,
        path: impl Into<PathBuf>,
        depth: usize,
        absolute: bool,
        canonicalize: bool,
        include_root: bool,
    ) -> AsyncReturn<'_, (Vec<DirEntry>, Vec<Failure>)>;

    /// Reads a remote file as a collection of bytes
    fn read_file(&mut self, path: impl Into<PathBuf>) -> AsyncReturn<'_, Vec<u8>>;

    /// Returns a remote file as a string
    fn read_file_text(&mut self, path: impl Into<PathBuf>) -> AsyncReturn<'_, String>;

    /// Removes a remote file or directory, supporting removal of non-empty directories if
    /// force is true
    fn remove(&mut self, path: impl Into<PathBuf>, force: bool) -> AsyncReturn<'_, ()>;

    /// Renames a remote file or directory from src to dst
    fn rename(&mut self, src: impl Into<PathBuf>, dst: impl Into<PathBuf>) -> AsyncReturn<'_, ()>;

    /// Watches a remote file or directory
    fn watch(
        &mut self,
        path: impl Into<PathBuf>,
        recursive: bool,
        only: impl Into<ChangeKindSet>,
        except: impl Into<ChangeKindSet>,
    ) -> AsyncReturn<'_, Watcher>;

    /// Unwatches a remote file or directory
    fn unwatch(&mut self, path: impl Into<PathBuf>) -> AsyncReturn<'_, ()>;

    /// Spawns a process on the remote machine
    fn spawn(
        &mut self,
        cmd: impl Into<String>,
        environment: Environment,
        current_dir: Option<PathBuf>,
        pty: Option<PtySize>,
    ) -> AsyncReturn<'_, RemoteProcess>;

    /// Spawns an LSP process on the remote machine
    fn spawn_lsp(
        &mut self,
        cmd: impl Into<String>,
        environment: Environment,
        current_dir: Option<PathBuf>,
        pty: Option<PtySize>,
    ) -> AsyncReturn<'_, RemoteLspProcess>;

    /// Spawns a process on the remote machine and wait for it to complete
    fn output(
        &mut self,
        cmd: impl Into<String>,
        environment: Environment,
        current_dir: Option<PathBuf>,
        pty: Option<PtySize>,
    ) -> AsyncReturn<'_, RemoteOutput>;

    /// Retrieves information about the remote system
    fn system_info(&mut self) -> AsyncReturn<'_, SystemInfo>;

    /// Retrieves server version information
    fn version(&mut self) -> AsyncReturn<'_, Version>;

    /// Returns version of protocol that the client uses
    fn protocol_version(&self) -> protocol::semver::Version;

    /// Writes a remote file with the data from a collection of bytes
    fn write_file(
        &mut self,
        path: impl Into<PathBuf>,
        data: impl Into<Vec<u8>>,
    ) -> AsyncReturn<'_, ()>;

    /// Writes a remote file with the data from a string
    fn write_file_text(
        &mut self,
        path: impl Into<PathBuf>,
        data: impl Into<String>,
    ) -> AsyncReturn<'_, ()>;
}

macro_rules! make_body {
    ($self:expr, $data:expr, @ok) => {
        make_body!($self, $data, |data| {
            match data {
                protocol::Response::Ok => Ok(()),
                protocol::Response::Error(x) => Err(io::Error::from(x)),
                _ => Err(mismatched_response()),
            }
        })
    };

    ($self:expr, $data:expr, $and_then:expr) => {{
        let req = Request::new(protocol::Msg::Single($data));
        Box::pin(async move {
            $self
                .send(req)
                .await
                .and_then(|res| match res.payload {
                    protocol::Msg::Single(x) => Ok(x),
                    _ => Err(mismatched_response()),
                })
                .and_then($and_then)
        })
    }};
}

impl DistantChannelExt
    for Channel<protocol::Msg<protocol::Request>, protocol::Msg<protocol::Response>>
{
    fn append_file(
        &mut self,
        path: impl Into<PathBuf>,
        data: impl Into<Vec<u8>>,
    ) -> AsyncReturn<'_, ()> {
        make_body!(
            self,
            protocol::Request::FileAppend { path: path.into(), data: data.into() },
            @ok
        )
    }

    fn append_file_text(
        &mut self,
        path: impl Into<PathBuf>,
        data: impl Into<String>,
    ) -> AsyncReturn<'_, ()> {
        make_body!(
            self,
            protocol::Request::FileAppendText { path: path.into(), text: data.into() },
            @ok
        )
    }

    fn copy(&mut self, src: impl Into<PathBuf>, dst: impl Into<PathBuf>) -> AsyncReturn<'_, ()> {
        make_body!(
            self,
            protocol::Request::Copy { src: src.into(), dst: dst.into() },
            @ok
        )
    }

    fn create_dir(&mut self, path: impl Into<PathBuf>, all: bool) -> AsyncReturn<'_, ()> {
        make_body!(
            self,
            protocol::Request::DirCreate { path: path.into(), all },
            @ok
        )
    }

    fn exists(&mut self, path: impl Into<PathBuf>) -> AsyncReturn<'_, bool> {
        make_body!(
            self,
            protocol::Request::Exists { path: path.into() },
            |data| match data {
                protocol::Response::Exists { value } => Ok(value),
                protocol::Response::Error(x) => Err(io::Error::from(x)),
                _ => Err(mismatched_response()),
            }
        )
    }

    fn is_compatible(&mut self) -> AsyncReturn<'_, bool> {
        make_body!(self, protocol::Request::Version {}, |data| match data {
            protocol::Response::Version(version) =>
                Ok(protocol::is_compatible_with(&version.protocol_version)),
            protocol::Response::Error(x) => Err(io::Error::from(x)),
            _ => Err(mismatched_response()),
        })
    }

    fn metadata(
        &mut self,
        path: impl Into<PathBuf>,
        canonicalize: bool,
        resolve_file_type: bool,
    ) -> AsyncReturn<'_, Metadata> {
        make_body!(
            self,
            protocol::Request::Metadata {
                path: path.into(),
                canonicalize,
                resolve_file_type
            },
            |data| match data {
                protocol::Response::Metadata(x) => Ok(x),
                protocol::Response::Error(x) => Err(io::Error::from(x)),
                _ => Err(mismatched_response()),
            }
        )
    }

    fn set_permissions(
        &mut self,
        path: impl Into<PathBuf>,
        permissions: Permissions,
        options: SetPermissionsOptions,
    ) -> AsyncReturn<'_, ()> {
        make_body!(
            self,
            protocol::Request::SetPermissions {
                path: path.into(),
                permissions,
                options,
            },
            @ok
        )
    }

    fn search(&mut self, query: impl Into<SearchQuery>) -> AsyncReturn<'_, Searcher> {
        let query = query.into();
        Box::pin(async move { Searcher::search(self.clone(), query).await })
    }

    fn cancel_search(&mut self, id: SearchId) -> AsyncReturn<'_, ()> {
        make_body!(
            self,
            protocol::Request::CancelSearch { id },
            @ok
        )
    }

    fn read_dir(
        &mut self,
        path: impl Into<PathBuf>,
        depth: usize,
        absolute: bool,
        canonicalize: bool,
        include_root: bool,
    ) -> AsyncReturn<'_, (Vec<DirEntry>, Vec<Failure>)> {
        make_body!(
            self,
            protocol::Request::DirRead {
                path: path.into(),
                depth,
                absolute,
                canonicalize,
                include_root
            },
            |data| match data {
                protocol::Response::DirEntries { entries, errors } => Ok((entries, errors)),
                protocol::Response::Error(x) => Err(io::Error::from(x)),
                _ => Err(mismatched_response()),
            }
        )
    }

    fn read_file(&mut self, path: impl Into<PathBuf>) -> AsyncReturn<'_, Vec<u8>> {
        make_body!(
            self,
            protocol::Request::FileRead { path: path.into() },
            |data| match data {
                protocol::Response::Blob { data } => Ok(data),
                protocol::Response::Error(x) => Err(io::Error::from(x)),
                _ => Err(mismatched_response()),
            }
        )
    }

    fn read_file_text(&mut self, path: impl Into<PathBuf>) -> AsyncReturn<'_, String> {
        make_body!(
            self,
            protocol::Request::FileReadText { path: path.into() },
            |data| match data {
                protocol::Response::Text { data } => Ok(data),
                protocol::Response::Error(x) => Err(io::Error::from(x)),
                _ => Err(mismatched_response()),
            }
        )
    }

    fn remove(&mut self, path: impl Into<PathBuf>, force: bool) -> AsyncReturn<'_, ()> {
        make_body!(
            self,
            protocol::Request::Remove { path: path.into(), force },
            @ok
        )
    }

    fn rename(&mut self, src: impl Into<PathBuf>, dst: impl Into<PathBuf>) -> AsyncReturn<'_, ()> {
        make_body!(
            self,
            protocol::Request::Rename { src: src.into(), dst: dst.into() },
            @ok
        )
    }

    fn watch(
        &mut self,
        path: impl Into<PathBuf>,
        recursive: bool,
        only: impl Into<ChangeKindSet>,
        except: impl Into<ChangeKindSet>,
    ) -> AsyncReturn<'_, Watcher> {
        let path = path.into();
        let only = only.into();
        let except = except.into();
        Box::pin(async move { Watcher::watch(self.clone(), path, recursive, only, except).await })
    }

    fn unwatch(&mut self, path: impl Into<PathBuf>) -> AsyncReturn<'_, ()> {
        fn inner_unwatch(
            channel: &mut Channel<
                protocol::Msg<protocol::Request>,
                protocol::Msg<protocol::Response>,
            >,
            path: impl Into<PathBuf>,
        ) -> AsyncReturn<'_, ()> {
            make_body!(
                channel,
                protocol::Request::Unwatch { path: path.into() },
                @ok
            )
        }

        let path = path.into();

        Box::pin(async move { inner_unwatch(self, path).await })
    }

    fn spawn(
        &mut self,
        cmd: impl Into<String>,
        environment: Environment,
        current_dir: Option<PathBuf>,
        pty: Option<PtySize>,
    ) -> AsyncReturn<'_, RemoteProcess> {
        let cmd = cmd.into();
        Box::pin(async move {
            RemoteCommand::new()
                .environment(environment)
                .current_dir(current_dir)
                .pty(pty)
                .spawn(self.clone(), cmd)
                .await
        })
    }

    fn spawn_lsp(
        &mut self,
        cmd: impl Into<String>,
        environment: Environment,
        current_dir: Option<PathBuf>,
        pty: Option<PtySize>,
    ) -> AsyncReturn<'_, RemoteLspProcess> {
        let cmd = cmd.into();
        Box::pin(async move {
            RemoteLspCommand::new()
                .environment(environment)
                .current_dir(current_dir)
                .pty(pty)
                .spawn(self.clone(), cmd)
                .await
        })
    }

    fn output(
        &mut self,
        cmd: impl Into<String>,
        environment: Environment,
        current_dir: Option<PathBuf>,
        pty: Option<PtySize>,
    ) -> AsyncReturn<'_, RemoteOutput> {
        let cmd = cmd.into();
        Box::pin(async move {
            RemoteCommand::new()
                .environment(environment)
                .current_dir(current_dir)
                .pty(pty)
                .spawn(self.clone(), cmd)
                .await?
                .output()
                .await
        })
    }

    fn system_info(&mut self) -> AsyncReturn<'_, SystemInfo> {
        make_body!(self, protocol::Request::SystemInfo {}, |data| match data {
            protocol::Response::SystemInfo(x) => Ok(x),
            protocol::Response::Error(x) => Err(io::Error::from(x)),
            _ => Err(mismatched_response()),
        })
    }

    fn version(&mut self) -> AsyncReturn<'_, Version> {
        make_body!(self, protocol::Request::Version {}, |data| match data {
            protocol::Response::Version(x) => Ok(x),
            protocol::Response::Error(x) => Err(io::Error::from(x)),
            _ => Err(mismatched_response()),
        })
    }

    fn protocol_version(&self) -> protocol::semver::Version {
        protocol::PROTOCOL_VERSION
    }

    fn write_file(
        &mut self,
        path: impl Into<PathBuf>,
        data: impl Into<Vec<u8>>,
    ) -> AsyncReturn<'_, ()> {
        make_body!(
            self,
            protocol::Request::FileWrite { path: path.into(), data: data.into() },
            @ok
        )
    }

    fn write_file_text(
        &mut self,
        path: impl Into<PathBuf>,
        data: impl Into<String>,
    ) -> AsyncReturn<'_, ()> {
        make_body!(
            self,
            protocol::Request::FileWriteText { path: path.into(), text: data.into() },
            @ok
        )
    }
}
