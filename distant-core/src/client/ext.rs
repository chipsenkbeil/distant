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
pub trait ChannelExt {
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

impl ChannelExt for Channel<protocol::Msg<protocol::Request>, protocol::Msg<protocol::Response>> {
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::net::common::{FramedTransport, InmemoryTransport, Request, Response};
    use crate::protocol::{
        self, DirEntry, FileType, Metadata, Permissions, SetPermissionsOptions, SystemInfo, Version,
    };
    use crate::Client;
    use test_log::test;

    use super::*;

    fn make_session() -> (FramedTransport<InmemoryTransport>, Client) {
        let (t1, t2) = FramedTransport::pair(100);
        (t1, Client::spawn_inmemory(t2, Default::default()))
    }

    // ------------------------------------------------------------------
    // append_file
    // ------------------------------------------------------------------

    #[test(tokio::test)]
    async fn append_file_should_send_correct_request_and_return_ok() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task =
            tokio::spawn(async move { channel.append_file("/test/path", vec![1, 2, 3]).await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            protocol::Request::FileAppend { path, data } => {
                assert_eq!(path, PathBuf::from("/test/path"));
                assert_eq!(data, [1, 2, 3]);
            }
            x => panic!("Unexpected request: {:?}", x),
        }

        transport
            .write_frame_for(&Response::new(req.id, protocol::Response::Ok))
            .await
            .unwrap();

        task.await.unwrap().unwrap();
    }

    #[test(tokio::test)]
    async fn append_file_should_return_error_on_error_response() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task =
            tokio::spawn(async move { channel.append_file("/test/path", vec![1, 2, 3]).await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        transport
            .write_frame_for(&Response::new(
                req.id,
                protocol::Response::Error(protocol::Error {
                    kind: protocol::ErrorKind::NotFound,
                    description: String::from("file not found"),
                }),
            ))
            .await
            .unwrap();

        let err = task.await.unwrap().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test(tokio::test)]
    async fn append_file_should_return_error_on_mismatched_response() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task =
            tokio::spawn(async move { channel.append_file("/test/path", vec![1, 2, 3]).await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        transport
            .write_frame_for(&Response::new(
                req.id,
                protocol::Response::Exists { value: true },
            ))
            .await
            .unwrap();

        let err = task.await.unwrap().unwrap_err();
        assert_eq!(err.to_string(), "Mismatched response");
    }

    // ------------------------------------------------------------------
    // append_file_text
    // ------------------------------------------------------------------

    #[test(tokio::test)]
    async fn append_file_text_should_send_correct_request_and_return_ok() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task =
            tokio::spawn(async move { channel.append_file_text("/test/path", "hello").await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            protocol::Request::FileAppendText { path, text } => {
                assert_eq!(path, PathBuf::from("/test/path"));
                assert_eq!(text, "hello");
            }
            x => panic!("Unexpected request: {:?}", x),
        }

        transport
            .write_frame_for(&Response::new(req.id, protocol::Response::Ok))
            .await
            .unwrap();

        task.await.unwrap().unwrap();
    }

    // ------------------------------------------------------------------
    // copy
    // ------------------------------------------------------------------

    #[test(tokio::test)]
    async fn copy_should_send_correct_request_and_return_ok() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task = tokio::spawn(async move { channel.copy("/src", "/dst").await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            protocol::Request::Copy { src, dst } => {
                assert_eq!(src, PathBuf::from("/src"));
                assert_eq!(dst, PathBuf::from("/dst"));
            }
            x => panic!("Unexpected request: {:?}", x),
        }

        transport
            .write_frame_for(&Response::new(req.id, protocol::Response::Ok))
            .await
            .unwrap();

        task.await.unwrap().unwrap();
    }

    // ------------------------------------------------------------------
    // create_dir
    // ------------------------------------------------------------------

    #[test(tokio::test)]
    async fn create_dir_should_send_correct_request_and_return_ok() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task = tokio::spawn(async move { channel.create_dir("/test/dir", true).await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            protocol::Request::DirCreate { path, all } => {
                assert_eq!(path, PathBuf::from("/test/dir"));
                assert!(all);
            }
            x => panic!("Unexpected request: {:?}", x),
        }

        transport
            .write_frame_for(&Response::new(req.id, protocol::Response::Ok))
            .await
            .unwrap();

        task.await.unwrap().unwrap();
    }

    // ------------------------------------------------------------------
    // exists
    // ------------------------------------------------------------------

    #[test(tokio::test)]
    async fn exists_should_return_true_when_response_is_true() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task = tokio::spawn(async move { channel.exists("/test/path").await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            protocol::Request::Exists { path } => {
                assert_eq!(path, PathBuf::from("/test/path"));
            }
            x => panic!("Unexpected request: {:?}", x),
        }

        transport
            .write_frame_for(&Response::new(
                req.id,
                protocol::Response::Exists { value: true },
            ))
            .await
            .unwrap();

        assert!(task.await.unwrap().unwrap());
    }

    #[test(tokio::test)]
    async fn exists_should_return_false_when_response_is_false() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task = tokio::spawn(async move { channel.exists("/test/path").await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        transport
            .write_frame_for(&Response::new(
                req.id,
                protocol::Response::Exists { value: false },
            ))
            .await
            .unwrap();

        assert!(!task.await.unwrap().unwrap());
    }

    #[test(tokio::test)]
    async fn exists_should_return_error_on_mismatched_response() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task = tokio::spawn(async move { channel.exists("/test/path").await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        transport
            .write_frame_for(&Response::new(req.id, protocol::Response::Ok))
            .await
            .unwrap();

        let err = task.await.unwrap().unwrap_err();
        assert_eq!(err.to_string(), "Mismatched response");
    }

    // ------------------------------------------------------------------
    // is_compatible
    // ------------------------------------------------------------------

    #[test(tokio::test)]
    async fn is_compatible_should_return_true_for_matching_protocol_version() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task = tokio::spawn(async move { channel.is_compatible().await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        transport
            .write_frame_for(&Response::new(
                req.id,
                protocol::Response::Version(Version {
                    server_version: protocol::PROTOCOL_VERSION.clone(),
                    protocol_version: protocol::PROTOCOL_VERSION.clone(),
                    capabilities: Vec::new(),
                }),
            ))
            .await
            .unwrap();

        assert!(task.await.unwrap().unwrap());
    }

    #[test(tokio::test)]
    async fn is_compatible_should_return_false_for_incompatible_version() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task = tokio::spawn(async move { channel.is_compatible().await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        // Use a wildly different major version
        let incompatible = protocol::semver::Version::new(999, 0, 0);
        transport
            .write_frame_for(&Response::new(
                req.id,
                protocol::Response::Version(Version {
                    server_version: incompatible.clone(),
                    protocol_version: incompatible,
                    capabilities: Vec::new(),
                }),
            ))
            .await
            .unwrap();

        assert!(!task.await.unwrap().unwrap());
    }

    #[test(tokio::test)]
    async fn is_compatible_should_return_error_on_error_response() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task = tokio::spawn(async move { channel.is_compatible().await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        transport
            .write_frame_for(&Response::new(
                req.id,
                protocol::Response::Error(protocol::Error {
                    kind: protocol::ErrorKind::Other,
                    description: String::from("version check failed"),
                }),
            ))
            .await
            .unwrap();

        let err = task.await.unwrap().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
    }

    // ------------------------------------------------------------------
    // metadata
    // ------------------------------------------------------------------

    #[test(tokio::test)]
    async fn metadata_should_send_correct_request_and_return_metadata() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task = tokio::spawn(async move { channel.metadata("/test/path", true, false).await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            protocol::Request::Metadata {
                path,
                canonicalize,
                resolve_file_type,
            } => {
                assert_eq!(path, PathBuf::from("/test/path"));
                assert!(canonicalize);
                assert!(!resolve_file_type);
            }
            x => panic!("Unexpected request: {:?}", x),
        }

        let expected_metadata = Metadata {
            canonicalized_path: Some(PathBuf::from("/test/path")),
            file_type: FileType::File,
            len: 1024,
            readonly: false,
            accessed: None,
            created: None,
            modified: None,
            unix: None,
            windows: None,
        };

        transport
            .write_frame_for(&Response::new(
                req.id,
                protocol::Response::Metadata(expected_metadata.clone()),
            ))
            .await
            .unwrap();

        let result = task.await.unwrap().unwrap();
        assert_eq!(result, expected_metadata);
    }

    #[test(tokio::test)]
    async fn metadata_should_return_error_on_error_response() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task = tokio::spawn(async move { channel.metadata("/test/path", false, false).await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        transport
            .write_frame_for(&Response::new(
                req.id,
                protocol::Response::Error(protocol::Error {
                    kind: protocol::ErrorKind::NotFound,
                    description: String::from("not found"),
                }),
            ))
            .await
            .unwrap();

        let err = task.await.unwrap().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    // ------------------------------------------------------------------
    // set_permissions
    // ------------------------------------------------------------------

    #[test(tokio::test)]
    async fn set_permissions_should_send_correct_request_and_return_ok() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let permissions = Permissions {
            owner_read: Some(true),
            ..Default::default()
        };
        let options = SetPermissionsOptions::default();

        let task = tokio::spawn(async move {
            channel
                .set_permissions("/test/path", permissions, options)
                .await
        });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            protocol::Request::SetPermissions {
                path,
                permissions: req_perms,
                options: req_opts,
            } => {
                assert_eq!(path, PathBuf::from("/test/path"));
                assert_eq!(req_perms, permissions);
                assert_eq!(req_opts, options);
            }
            x => panic!("Unexpected request: {:?}", x),
        }

        transport
            .write_frame_for(&Response::new(req.id, protocol::Response::Ok))
            .await
            .unwrap();

        task.await.unwrap().unwrap();
    }

    // ------------------------------------------------------------------
    // cancel_search
    // ------------------------------------------------------------------

    #[test(tokio::test)]
    async fn cancel_search_should_send_correct_request_and_return_ok() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task = tokio::spawn(async move { channel.cancel_search(42).await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            protocol::Request::CancelSearch { id } => {
                assert_eq!(id, 42);
            }
            x => panic!("Unexpected request: {:?}", x),
        }

        transport
            .write_frame_for(&Response::new(req.id, protocol::Response::Ok))
            .await
            .unwrap();

        task.await.unwrap().unwrap();
    }

    #[test(tokio::test)]
    async fn cancel_search_should_return_error_on_error_response() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task = tokio::spawn(async move { channel.cancel_search(99).await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        transport
            .write_frame_for(&Response::new(
                req.id,
                protocol::Response::Error(protocol::Error {
                    kind: protocol::ErrorKind::Other,
                    description: String::from("no such search"),
                }),
            ))
            .await
            .unwrap();

        let err = task.await.unwrap().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
    }

    // ------------------------------------------------------------------
    // read_dir
    // ------------------------------------------------------------------

    #[test(tokio::test)]
    async fn read_dir_should_send_correct_request_and_return_entries() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task =
            tokio::spawn(async move { channel.read_dir("/test/dir", 1, true, false, true).await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            protocol::Request::DirRead {
                path,
                depth,
                absolute,
                canonicalize,
                include_root,
            } => {
                assert_eq!(path, PathBuf::from("/test/dir"));
                assert_eq!(depth, 1);
                assert!(absolute);
                assert!(!canonicalize);
                assert!(include_root);
            }
            x => panic!("Unexpected request: {:?}", x),
        }

        let entries = vec![DirEntry {
            path: PathBuf::from("/test/dir/file.txt"),
            file_type: FileType::File,
            depth: 1,
        }];

        transport
            .write_frame_for(&Response::new(
                req.id,
                protocol::Response::DirEntries {
                    entries: entries.clone(),
                    errors: Vec::new(),
                },
            ))
            .await
            .unwrap();

        let (result_entries, result_errors) = task.await.unwrap().unwrap();
        assert_eq!(result_entries, entries);
        assert!(result_errors.is_empty());
    }

    #[test(tokio::test)]
    async fn read_dir_should_return_error_on_mismatched_response() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task =
            tokio::spawn(async move { channel.read_dir("/test/dir", 1, true, false, true).await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        transport
            .write_frame_for(&Response::new(req.id, protocol::Response::Ok))
            .await
            .unwrap();

        let err = task.await.unwrap().unwrap_err();
        assert_eq!(err.to_string(), "Mismatched response");
    }

    // ------------------------------------------------------------------
    // read_file
    // ------------------------------------------------------------------

    #[test(tokio::test)]
    async fn read_file_should_send_correct_request_and_return_data() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task = tokio::spawn(async move { channel.read_file("/test/file").await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            protocol::Request::FileRead { path } => {
                assert_eq!(path, PathBuf::from("/test/file"));
            }
            x => panic!("Unexpected request: {:?}", x),
        }

        transport
            .write_frame_for(&Response::new(
                req.id,
                protocol::Response::Blob {
                    data: vec![10, 20, 30],
                },
            ))
            .await
            .unwrap();

        let result = task.await.unwrap().unwrap();
        assert_eq!(result, [10, 20, 30]);
    }

    #[test(tokio::test)]
    async fn read_file_should_return_error_on_error_response() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task = tokio::spawn(async move { channel.read_file("/test/file").await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        transport
            .write_frame_for(&Response::new(
                req.id,
                protocol::Response::Error(protocol::Error {
                    kind: protocol::ErrorKind::PermissionDenied,
                    description: String::from("denied"),
                }),
            ))
            .await
            .unwrap();

        let err = task.await.unwrap().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    // ------------------------------------------------------------------
    // read_file_text
    // ------------------------------------------------------------------

    #[test(tokio::test)]
    async fn read_file_text_should_send_correct_request_and_return_string() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task = tokio::spawn(async move { channel.read_file_text("/test/file").await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            protocol::Request::FileReadText { path } => {
                assert_eq!(path, PathBuf::from("/test/file"));
            }
            x => panic!("Unexpected request: {:?}", x),
        }

        transport
            .write_frame_for(&Response::new(
                req.id,
                protocol::Response::Text {
                    data: String::from("hello world"),
                },
            ))
            .await
            .unwrap();

        let result = task.await.unwrap().unwrap();
        assert_eq!(result, "hello world");
    }

    #[test(tokio::test)]
    async fn read_file_text_should_return_error_on_mismatched_response() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task = tokio::spawn(async move { channel.read_file_text("/test/file").await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        transport
            .write_frame_for(&Response::new(
                req.id,
                protocol::Response::Blob {
                    data: vec![1, 2, 3],
                },
            ))
            .await
            .unwrap();

        let err = task.await.unwrap().unwrap_err();
        assert_eq!(err.to_string(), "Mismatched response");
    }

    // ------------------------------------------------------------------
    // remove
    // ------------------------------------------------------------------

    #[test(tokio::test)]
    async fn remove_should_send_correct_request_and_return_ok() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task = tokio::spawn(async move { channel.remove("/test/path", true).await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            protocol::Request::Remove { path, force } => {
                assert_eq!(path, PathBuf::from("/test/path"));
                assert!(force);
            }
            x => panic!("Unexpected request: {:?}", x),
        }

        transport
            .write_frame_for(&Response::new(req.id, protocol::Response::Ok))
            .await
            .unwrap();

        task.await.unwrap().unwrap();
    }

    // ------------------------------------------------------------------
    // rename
    // ------------------------------------------------------------------

    #[test(tokio::test)]
    async fn rename_should_send_correct_request_and_return_ok() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task = tokio::spawn(async move { channel.rename("/old", "/new").await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            protocol::Request::Rename { src, dst } => {
                assert_eq!(src, PathBuf::from("/old"));
                assert_eq!(dst, PathBuf::from("/new"));
            }
            x => panic!("Unexpected request: {:?}", x),
        }

        transport
            .write_frame_for(&Response::new(req.id, protocol::Response::Ok))
            .await
            .unwrap();

        task.await.unwrap().unwrap();
    }

    // ------------------------------------------------------------------
    // unwatch
    // ------------------------------------------------------------------

    #[test(tokio::test)]
    async fn unwatch_should_send_correct_request_and_return_ok() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task = tokio::spawn(async move { channel.unwatch("/test/path").await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            protocol::Request::Unwatch { path } => {
                assert_eq!(path, PathBuf::from("/test/path"));
            }
            x => panic!("Unexpected request: {:?}", x),
        }

        transport
            .write_frame_for(&Response::new(req.id, protocol::Response::Ok))
            .await
            .unwrap();

        task.await.unwrap().unwrap();
    }

    #[test(tokio::test)]
    async fn unwatch_should_return_error_on_error_response() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task = tokio::spawn(async move { channel.unwatch("/test/path").await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        transport
            .write_frame_for(&Response::new(
                req.id,
                protocol::Response::Error(protocol::Error {
                    kind: protocol::ErrorKind::NotFound,
                    description: String::from("not watched"),
                }),
            ))
            .await
            .unwrap();

        let err = task.await.unwrap().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    // ------------------------------------------------------------------
    // system_info
    // ------------------------------------------------------------------

    #[test(tokio::test)]
    async fn system_info_should_return_system_info_on_success() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task = tokio::spawn(async move { channel.system_info().await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            protocol::Request::SystemInfo {} => {}
            x => panic!("Unexpected request: {:?}", x),
        }

        let expected = SystemInfo {
            family: String::from("unix"),
            os: String::from("macos"),
            arch: String::from("aarch64"),
            current_dir: PathBuf::from("/home/user"),
            main_separator: '/',
            username: String::from("user"),
            shell: String::from("/bin/bash"),
        };

        transport
            .write_frame_for(&Response::new(
                req.id,
                protocol::Response::SystemInfo(expected.clone()),
            ))
            .await
            .unwrap();

        let result = task.await.unwrap().unwrap();
        assert_eq!(result, expected);
    }

    #[test(tokio::test)]
    async fn system_info_should_return_error_on_error_response() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task = tokio::spawn(async move { channel.system_info().await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        transport
            .write_frame_for(&Response::new(
                req.id,
                protocol::Response::Error(protocol::Error {
                    kind: protocol::ErrorKind::Other,
                    description: String::from("system info failed"),
                }),
            ))
            .await
            .unwrap();

        let err = task.await.unwrap().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
    }

    #[test(tokio::test)]
    async fn system_info_should_return_error_on_mismatched_response() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task = tokio::spawn(async move { channel.system_info().await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        transport
            .write_frame_for(&Response::new(req.id, protocol::Response::Ok))
            .await
            .unwrap();

        let err = task.await.unwrap().unwrap_err();
        assert_eq!(err.to_string(), "Mismatched response");
    }

    // ------------------------------------------------------------------
    // version
    // ------------------------------------------------------------------

    #[test(tokio::test)]
    async fn version_should_return_version_on_success() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task = tokio::spawn(async move { channel.version().await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            protocol::Request::Version {} => {}
            x => panic!("Unexpected request: {:?}", x),
        }

        let expected = Version {
            server_version: protocol::semver::Version::new(1, 2, 3),
            protocol_version: protocol::PROTOCOL_VERSION.clone(),
            capabilities: vec![String::from("exec")],
        };

        transport
            .write_frame_for(&Response::new(
                req.id,
                protocol::Response::Version(expected.clone()),
            ))
            .await
            .unwrap();

        let result = task.await.unwrap().unwrap();
        assert_eq!(result, expected);
    }

    #[test(tokio::test)]
    async fn version_should_return_error_on_error_response() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task = tokio::spawn(async move { channel.version().await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        transport
            .write_frame_for(&Response::new(
                req.id,
                protocol::Response::Error(protocol::Error {
                    kind: protocol::ErrorKind::Other,
                    description: String::from("version failed"),
                }),
            ))
            .await
            .unwrap();

        let err = task.await.unwrap().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
    }

    #[test(tokio::test)]
    async fn version_should_return_error_on_mismatched_response() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task = tokio::spawn(async move { channel.version().await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        transport
            .write_frame_for(&Response::new(req.id, protocol::Response::Ok))
            .await
            .unwrap();

        let err = task.await.unwrap().unwrap_err();
        assert_eq!(err.to_string(), "Mismatched response");
    }

    // ------------------------------------------------------------------
    // protocol_version
    // ------------------------------------------------------------------

    #[test(tokio::test)]
    async fn protocol_version_should_return_current_protocol_version() {
        let (_transport, session) = make_session();
        let channel = session.clone_channel();

        let version = channel.protocol_version();
        assert_eq!(version, protocol::PROTOCOL_VERSION);
    }

    // ------------------------------------------------------------------
    // write_file
    // ------------------------------------------------------------------

    #[test(tokio::test)]
    async fn write_file_should_send_correct_request_and_return_ok() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task =
            tokio::spawn(async move { channel.write_file("/test/file", vec![4, 5, 6]).await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            protocol::Request::FileWrite { path, data } => {
                assert_eq!(path, PathBuf::from("/test/file"));
                assert_eq!(data, [4, 5, 6]);
            }
            x => panic!("Unexpected request: {:?}", x),
        }

        transport
            .write_frame_for(&Response::new(req.id, protocol::Response::Ok))
            .await
            .unwrap();

        task.await.unwrap().unwrap();
    }

    #[test(tokio::test)]
    async fn write_file_should_return_error_on_error_response() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task =
            tokio::spawn(async move { channel.write_file("/test/file", vec![4, 5, 6]).await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        transport
            .write_frame_for(&Response::new(
                req.id,
                protocol::Response::Error(protocol::Error {
                    kind: protocol::ErrorKind::PermissionDenied,
                    description: String::from("denied"),
                }),
            ))
            .await
            .unwrap();

        let err = task.await.unwrap().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    // ------------------------------------------------------------------
    // write_file_text
    // ------------------------------------------------------------------

    #[test(tokio::test)]
    async fn write_file_text_should_send_correct_request_and_return_ok() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task =
            tokio::spawn(async move { channel.write_file_text("/test/file", "hello world").await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            protocol::Request::FileWriteText { path, text } => {
                assert_eq!(path, PathBuf::from("/test/file"));
                assert_eq!(text, "hello world");
            }
            x => panic!("Unexpected request: {:?}", x),
        }

        transport
            .write_frame_for(&Response::new(req.id, protocol::Response::Ok))
            .await
            .unwrap();

        task.await.unwrap().unwrap();
    }

    #[test(tokio::test)]
    async fn write_file_text_should_return_error_on_mismatched_response() {
        let (mut transport, session) = make_session();
        let mut channel = session.clone_channel();

        let task =
            tokio::spawn(async move { channel.write_file_text("/test/file", "hello").await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        transport
            .write_frame_for(&Response::new(
                req.id,
                protocol::Response::Exists { value: true },
            ))
            .await
            .unwrap();

        let err = task.await.unwrap().unwrap_err();
        assert_eq!(err.to_string(), "Mismatched response");
    }
}
