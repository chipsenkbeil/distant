use std::path::{Path, PathBuf};
use std::time::SystemTime;
use std::{env, io};

use async_trait::async_trait;
use distant_core::protocol::{
    semver, ChangeKind, ChangeKindSet, DirEntry, Environment, FileType, Metadata, Permissions,
    ProcessId, PtySize, SearchId, SearchQuery, SetPermissionsOptions, SystemInfo, Version,
    PROTOCOL_VERSION,
};
use distant_core::{DistantApi, DistantCtx};
use ignore::{DirEntry as WalkDirEntry, WalkBuilder};
use log::*;
use tokio::io::AsyncWriteExt;
use walkdir::WalkDir;

use crate::config::Config;

mod process;
mod state;
use state::*;

/// Represents an implementation of [`DistantApi`] that works with the local machine
/// where the server using this api is running. In other words, this is a direct
/// impementation of the API instead of a proxy to another machine as seen with
/// implementations on top of SSH and other protocol.
pub struct Api {
    state: GlobalState,
}

impl Api {
    /// Initialize the api instance
    pub fn initialize(config: Config) -> io::Result<Self> {
        Ok(Self {
            state: GlobalState::initialize(config)?,
        })
    }
}

#[async_trait]
impl DistantApi for Api {
    async fn read_file(&self, ctx: DistantCtx, path: PathBuf) -> io::Result<Vec<u8>> {
        debug!(
            "[Conn {}] Reading bytes from file {:?}",
            ctx.connection_id, path
        );

        tokio::fs::read(path).await
    }

    async fn read_file_text(&self, ctx: DistantCtx, path: PathBuf) -> io::Result<String> {
        debug!(
            "[Conn {}] Reading text from file {:?}",
            ctx.connection_id, path
        );

        tokio::fs::read_to_string(path).await
    }

    async fn write_file(&self, ctx: DistantCtx, path: PathBuf, data: Vec<u8>) -> io::Result<()> {
        debug!(
            "[Conn {}] Writing bytes to file {:?}",
            ctx.connection_id, path
        );

        tokio::fs::write(path, data).await
    }

    async fn write_file_text(
        &self,
        ctx: DistantCtx,
        path: PathBuf,
        data: String,
    ) -> io::Result<()> {
        debug!(
            "[Conn {}] Writing text to file {:?}",
            ctx.connection_id, path
        );

        tokio::fs::write(path, data).await
    }

    async fn append_file(&self, ctx: DistantCtx, path: PathBuf, data: Vec<u8>) -> io::Result<()> {
        debug!(
            "[Conn {}] Appending bytes to file {:?}",
            ctx.connection_id, path
        );

        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await?;
        file.write_all(data.as_ref()).await
    }

    async fn append_file_text(
        &self,
        ctx: DistantCtx,
        path: PathBuf,
        data: String,
    ) -> io::Result<()> {
        debug!(
            "[Conn {}] Appending text to file {:?}",
            ctx.connection_id, path
        );

        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await?;
        file.write_all(data.as_ref()).await
    }

    async fn read_dir(
        &self,
        ctx: DistantCtx,
        path: PathBuf,
        depth: usize,
        absolute: bool,
        canonicalize: bool,
        include_root: bool,
    ) -> io::Result<(Vec<DirEntry>, Vec<io::Error>)> {
        debug!(
            "[Conn {}] Reading directory {:?} {{depth: {}, absolute: {}, canonicalize: {}, include_root: {}}}",
            ctx.connection_id, path, depth, absolute, canonicalize, include_root
        );

        // Canonicalize our provided path to ensure that it is exists, not a loop, and absolute
        let root_path = tokio::fs::canonicalize(path).await?;

        // Traverse, but don't include root directory in entries (hence min depth 1), unless indicated
        // to do so (min depth 0)
        let dir = WalkDir::new(root_path.as_path())
            .min_depth(usize::from(!include_root))
            .sort_by_file_name();

        // If depth > 0, will recursively traverse to specified max depth, otherwise
        // performs infinite traversal
        let dir = if depth > 0 { dir.max_depth(depth) } else { dir };

        // Determine our entries and errors
        let mut entries = Vec::new();
        let mut errors = Vec::new();

        #[inline]
        fn map_file_type(ft: std::fs::FileType) -> FileType {
            if ft.is_dir() {
                FileType::Dir
            } else if ft.is_file() {
                FileType::File
            } else {
                FileType::Symlink
            }
        }

        for entry in dir {
            match entry.map_err(io::Error::from) {
                // For entries within the root, we want to transform the path based on flags
                Ok(e) if e.depth() > 0 => {
                    // Canonicalize the path if specified, otherwise just return
                    // the path as is
                    let mut path = if canonicalize {
                        match tokio::fs::canonicalize(e.path()).await {
                            Ok(path) => path,
                            Err(x) => {
                                errors.push(x);
                                continue;
                            }
                        }
                    } else {
                        e.path().to_path_buf()
                    };

                    // Strip the path of its prefix based if not flagged as absolute
                    if !absolute {
                        // NOTE: In the situation where we canonicalized the path earlier,
                        //       there is no guarantee that our root path is still the
                        //       parent of the symlink's destination; so, in that case we MUST just
                        //       return the path if the strip_prefix fails
                        path = path
                            .strip_prefix(root_path.as_path())
                            .map(Path::to_path_buf)
                            .unwrap_or(path);
                    };

                    entries.push(DirEntry {
                        path,
                        file_type: map_file_type(e.file_type()),
                        depth: e.depth(),
                    });
                }

                // For the root, we just want to echo back the entry as is
                Ok(e) => {
                    entries.push(DirEntry {
                        path: e.path().to_path_buf(),
                        file_type: map_file_type(e.file_type()),
                        depth: e.depth(),
                    });
                }

                Err(x) => errors.push(x),
            }
        }

        Ok((entries, errors))
    }

    async fn create_dir(&self, ctx: DistantCtx, path: PathBuf, all: bool) -> io::Result<()> {
        debug!(
            "[Conn {}] Creating directory {:?} {{all: {}}}",
            ctx.connection_id, path, all
        );
        if all {
            tokio::fs::create_dir_all(path).await
        } else {
            tokio::fs::create_dir(path).await
        }
    }

    async fn remove(&self, ctx: DistantCtx, path: PathBuf, force: bool) -> io::Result<()> {
        debug!(
            "[Conn {}] Removing {:?} {{force: {}}}",
            ctx.connection_id, path, force
        );
        let path_metadata = tokio::fs::metadata(path.as_path()).await?;
        if path_metadata.is_dir() {
            if force {
                tokio::fs::remove_dir_all(path).await
            } else {
                tokio::fs::remove_dir(path).await
            }
        } else {
            tokio::fs::remove_file(path).await
        }
    }

    async fn copy(&self, ctx: DistantCtx, src: PathBuf, dst: PathBuf) -> io::Result<()> {
        debug!(
            "[Conn {}] Copying {:?} to {:?}",
            ctx.connection_id, src, dst
        );
        let src_metadata = tokio::fs::metadata(src.as_path()).await?;
        if src_metadata.is_dir() {
            // Create the destination directory first, regardless of if anything
            // is in the source directory
            tokio::fs::create_dir_all(dst.as_path()).await?;

            for entry in WalkDir::new(src.as_path())
                .min_depth(1)
                .follow_links(false)
                .into_iter()
                .filter_entry(|e| {
                    e.file_type().is_file() || e.file_type().is_dir() || e.path_is_symlink()
                })
            {
                let entry = entry?;

                // Get unique portion of path relative to src
                // NOTE: Because we are traversing files that are all within src, this
                //       should always succeed
                let local_src = entry.path().strip_prefix(src.as_path()).unwrap();

                // Get the file without any directories
                let local_src_file_name = local_src.file_name().unwrap();

                // Get the directory housing the file
                // NOTE: Because we enforce files/symlinks, there will always be a parent
                let local_src_dir = local_src.parent().unwrap();

                // Map out the path to the destination
                let dst_parent_dir = dst.join(local_src_dir);

                // Create the destination directory for the file when copying
                tokio::fs::create_dir_all(dst_parent_dir.as_path()).await?;

                let dst_path = dst_parent_dir.join(local_src_file_name);

                // Perform copying from entry to destination (if a file/symlink)
                if !entry.file_type().is_dir() {
                    tokio::fs::copy(entry.path(), dst_path).await?;

                // Otherwise, if a directory, create it
                } else {
                    tokio::fs::create_dir(dst_path).await?;
                }
            }
        } else {
            tokio::fs::copy(src, dst).await?;
        }

        Ok(())
    }

    async fn rename(&self, ctx: DistantCtx, src: PathBuf, dst: PathBuf) -> io::Result<()> {
        debug!(
            "[Conn {}] Renaming {:?} to {:?}",
            ctx.connection_id, src, dst
        );
        tokio::fs::rename(src, dst).await
    }

    async fn watch(
        &self,
        ctx: DistantCtx,
        path: PathBuf,
        recursive: bool,
        only: Vec<ChangeKind>,
        except: Vec<ChangeKind>,
    ) -> io::Result<()> {
        let only = only.into_iter().collect::<ChangeKindSet>();
        let except = except.into_iter().collect::<ChangeKindSet>();
        debug!(
            "[Conn {}] Watching {:?} {{recursive: {}, only: {}, except: {}}}",
            ctx.connection_id, path, recursive, only, except
        );

        let path = RegisteredPath::register(
            ctx.connection_id,
            path.as_path(),
            recursive,
            only,
            except,
            ctx.reply,
        )
        .await?;

        self.state.watcher.watch(path).await?;

        Ok(())
    }

    async fn unwatch(&self, ctx: DistantCtx, path: PathBuf) -> io::Result<()> {
        debug!("[Conn {}] Unwatching {:?}", ctx.connection_id, path);

        self.state
            .watcher
            .unwatch(ctx.connection_id, path.as_path())
            .await?;
        Ok(())
    }

    async fn exists(&self, ctx: DistantCtx, path: PathBuf) -> io::Result<bool> {
        debug!("[Conn {}] Checking if {:?} exists", ctx.connection_id, path);

        // Following experimental `std::fs::try_exists`, which checks the error kind of the
        // metadata lookup to see if it is not found and filters accordingly
        match tokio::fs::metadata(path.as_path()).await {
            Ok(_) => Ok(true),
            Err(x) if x.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(x) => return Err(x),
        }
    }

    async fn metadata(
        &self,
        ctx: DistantCtx,
        path: PathBuf,
        canonicalize: bool,
        resolve_file_type: bool,
    ) -> io::Result<Metadata> {
        debug!(
            "[Conn {}] Reading metadata for {:?} {{canonicalize: {}, resolve_file_type: {}}}",
            ctx.connection_id, path, canonicalize, resolve_file_type
        );
        let metadata = tokio::fs::symlink_metadata(path.as_path()).await?;
        let canonicalized_path = if canonicalize {
            Some(tokio::fs::canonicalize(path.as_path()).await?)
        } else {
            None
        };

        // If asking for resolved file type and current type is symlink, then we want to refresh
        // our metadata to get the filetype for the resolved link
        let file_type = if resolve_file_type && metadata.file_type().is_symlink() {
            tokio::fs::metadata(path).await?.file_type()
        } else {
            metadata.file_type()
        };

        Ok(Metadata {
            canonicalized_path,
            accessed: metadata
                .accessed()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs()),
            created: metadata
                .created()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs()),
            modified: metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs()),
            len: metadata.len(),
            readonly: metadata.permissions().readonly(),
            file_type: if file_type.is_dir() {
                FileType::Dir
            } else if file_type.is_file() {
                FileType::File
            } else {
                FileType::Symlink
            },

            #[cfg(unix)]
            unix: Some({
                use std::os::unix::prelude::*;
                let mode = metadata.mode();
                distant_core::protocol::UnixMetadata::from(mode)
            }),
            #[cfg(not(unix))]
            unix: None,

            #[cfg(windows)]
            windows: Some({
                use std::os::windows::prelude::*;
                let attributes = metadata.file_attributes();
                distant_core::protocol::WindowsMetadata::from(attributes)
            }),
            #[cfg(not(windows))]
            windows: None,
        })
    }

    async fn set_permissions(
        &self,
        _ctx: DistantCtx,
        path: PathBuf,
        permissions: Permissions,
        options: SetPermissionsOptions,
    ) -> io::Result<()> {
        /// Builds permissions from the metadata of `entry`, failing if metadata was unavailable.
        fn build_permissions(
            entry: &WalkDirEntry,
            permissions: &Permissions,
        ) -> io::Result<std::fs::Permissions> {
            // Load up our std permissions so we can modify them
            let mut std_permissions = entry
                .metadata()
                .map_err(|x| match x.io_error() {
                    Some(x) => io::Error::new(x.kind(), format!("(Read permissions failed) {x}")),
                    None => io::Error::other(format!("(Read permissions failed) {x}")),
                })?
                .permissions();

            // Apply the readonly flag for all platforms but junix
            if !cfg!(unix) {
                if let Some(readonly) = permissions.is_readonly() {
                    std_permissions.set_readonly(readonly);
                }
            }

            // On Unix platforms, we can apply a bitset change
            #[cfg(unix)]
            {
                use std::os::unix::prelude::*;
                let mut current = Permissions::from(std_permissions.clone());
                current.apply_from(permissions);

                let mode = current.to_unix_mode();
                std_permissions.set_mode(mode);
            }

            Ok(std_permissions)
        }

        async fn set_permissions_impl(
            entry: &WalkDirEntry,
            permissions: &Permissions,
        ) -> io::Result<()> {
            let permissions = match permissions.is_complete() {
                // If we are on a Unix platform and we have a full permission set, we do not need
                // to retrieve the permissions to modify them and can instead produce a new
                // permission set purely from the permissions
                #[cfg(unix)]
                true => std::fs::Permissions::from(*permissions),

                // Otherwise, we have to load in the permissions from metadata and merge with our
                // changes
                _ => build_permissions(entry, permissions)?,
            };

            if log_enabled!(Level::Trace) {
                let mut output = String::new();
                output.push_str("readonly = ");
                output.push_str(if permissions.readonly() {
                    "true"
                } else {
                    "false"
                });

                #[cfg(unix)]
                {
                    use std::os::unix::prelude::*;
                    output.push_str(&format!(", mode = {:#o}", permissions.mode()));
                }

                trace!("Setting {:?} permissions to ({})", entry.path(), output);
            }

            tokio::fs::set_permissions(entry.path(), permissions)
                .await
                .map_err(|x| io::Error::new(x.kind(), format!("(Set permissions failed) {x}")))
        }

        // NOTE: On Unix platforms, setting permissions would automatically resolve the symlink,
        // but on Windows this is not the case. So, on Windows, we need to resolve our path by
        // following the symlink prior to feeding it to the walk builder because it does not appear
        // to resolve the symlink itself.
        //
        // We do this by canonicalizing the path if following symlinks is enabled.
        let path = if options.follow_symlinks {
            tokio::fs::canonicalize(path).await?
        } else {
            path
        };

        let walk = WalkBuilder::new(path)
            .follow_links(options.follow_symlinks)
            .max_depth(if options.recursive { None } else { Some(0) })
            .standard_filters(false)
            .skip_stdout(true)
            .build();

        // Process as much as possible and then fail with an error
        let mut errors = Vec::new();
        for entry in walk {
            match entry {
                Ok(entry) if entry.path_is_symlink() && options.exclude_symlinks => {}
                Ok(entry) => {
                    if let Err(x) = set_permissions_impl(&entry, &permissions).await {
                        errors.push(format!("{:?}: {x}", entry.path()));
                    }
                }
                Err(x) => {
                    errors.push(x.to_string());
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                errors
                    .into_iter()
                    .map(|x| format!("* {x}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ))
        }
    }

    async fn search(&self, ctx: DistantCtx, query: SearchQuery) -> io::Result<SearchId> {
        debug!(
            "[Conn {}] Performing search via {query:?}",
            ctx.connection_id,
        );

        self.state.search.start(query, ctx.reply).await
    }

    async fn cancel_search(&self, ctx: DistantCtx, id: SearchId) -> io::Result<()> {
        debug!("[Conn {}] Cancelling search {id}", ctx.connection_id,);

        self.state.search.cancel(id).await
    }

    async fn proc_spawn(
        &self,
        ctx: DistantCtx,
        cmd: String,
        environment: Environment,
        current_dir: Option<PathBuf>,
        pty: Option<PtySize>,
    ) -> io::Result<ProcessId> {
        debug!(
            "[Conn {}] Spawning {} {{environment: {:?}, current_dir: {:?}, pty: {:?}}}",
            ctx.connection_id, cmd, environment, current_dir, pty
        );
        self.state
            .process
            .spawn(cmd, environment, current_dir, pty, ctx.reply)
            .await
    }

    async fn proc_kill(&self, ctx: DistantCtx, id: ProcessId) -> io::Result<()> {
        debug!("[Conn {}] Killing process {}", ctx.connection_id, id);
        self.state.process.kill(id).await
    }

    async fn proc_stdin(&self, ctx: DistantCtx, id: ProcessId, data: Vec<u8>) -> io::Result<()> {
        debug!(
            "[Conn {}] Sending stdin to process {}",
            ctx.connection_id, id
        );
        self.state.process.send_stdin(id, data).await
    }

    async fn proc_resize_pty(
        &self,
        ctx: DistantCtx,
        id: ProcessId,
        size: PtySize,
    ) -> io::Result<()> {
        debug!(
            "[Conn {}] Resizing pty of process {} to {}",
            ctx.connection_id, id, size
        );
        self.state.process.resize_pty(id, size).await
    }

    async fn system_info(&self, ctx: DistantCtx) -> io::Result<SystemInfo> {
        debug!("[Conn {}] Reading system information", ctx.connection_id);
        Ok(SystemInfo {
            family: env::consts::FAMILY.to_string(),
            os: env::consts::OS.to_string(),
            arch: env::consts::ARCH.to_string(),
            current_dir: env::current_dir().unwrap_or_default(),
            main_separator: std::path::MAIN_SEPARATOR,
            username: whoami::username(),
            shell: if cfg!(windows) {
                env::var("ComSpec").unwrap_or_else(|_| String::from("cmd.exe"))
            } else {
                env::var("SHELL").unwrap_or_else(|_| String::from("/bin/sh"))
            },
        })
    }

    async fn version(&self, ctx: DistantCtx) -> io::Result<Version> {
        debug!("[Conn {}] Querying version", ctx.connection_id);

        // Parse our server's version
        let mut server_version: semver::Version = env!("CARGO_PKG_VERSION")
            .parse()
            .map_err(io::Error::other)?;

        // Add the package name to the version information
        if server_version.build.is_empty() {
            server_version.build =
                semver::BuildMetadata::new(env!("CARGO_PKG_NAME")).map_err(io::Error::other)?;
        } else {
            let raw_build_str = format!(
                "{}.{}",
                server_version.build.as_str(),
                env!("CARGO_PKG_NAME")
            );
            server_version.build =
                semver::BuildMetadata::new(&raw_build_str).map_err(io::Error::other)?;
        }

        Ok(Version {
            server_version,
            protocol_version: PROTOCOL_VERSION,
            capabilities: Version::capabilities()
                .iter()
                .map(ToString::to_string)
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use assert_fs::prelude::*;
    use distant_core::net::server::Reply;
    use distant_core::protocol::Response;
    use once_cell::sync::Lazy;
    use predicates::prelude::*;
    use test_log::test;
    use tokio::sync::mpsc;

    use super::*;
    use crate::config::WatchConfig;

    static TEMP_SCRIPT_DIR: Lazy<assert_fs::TempDir> =
        Lazy::new(|| assert_fs::TempDir::new().unwrap());
    static SCRIPT_RUNNER: Lazy<String> = Lazy::new(|| String::from("bash"));

    static ECHO_ARGS_TO_STDOUT_SH: Lazy<assert_fs::fixture::ChildPath> = Lazy::new(|| {
        let script = TEMP_SCRIPT_DIR.child("echo_args_to_stdout.sh");
        script
            .write_str(indoc::indoc!(
                r#"
                #/usr/bin/env bash
                printf "%s" "$*"
            "#
            ))
            .unwrap();
        script
    });

    static ECHO_ARGS_TO_STDERR_SH: Lazy<assert_fs::fixture::ChildPath> = Lazy::new(|| {
        let script = TEMP_SCRIPT_DIR.child("echo_args_to_stderr.sh");
        script
            .write_str(indoc::indoc!(
                r#"
                #/usr/bin/env bash
                printf "%s" "$*" 1>&2
            "#
            ))
            .unwrap();
        script
    });

    static ECHO_STDIN_TO_STDOUT_SH: Lazy<assert_fs::fixture::ChildPath> = Lazy::new(|| {
        let script = TEMP_SCRIPT_DIR.child("echo_stdin_to_stdout.sh");
        script
            .write_str(indoc::indoc!(
                r#"
                #/usr/bin/env bash
                while IFS= read; do echo "$REPLY"; done
            "#
            ))
            .unwrap();
        script
    });

    static SLEEP_SH: Lazy<assert_fs::fixture::ChildPath> = Lazy::new(|| {
        let script = TEMP_SCRIPT_DIR.child("sleep.sh");
        script
            .write_str(indoc::indoc!(
                r#"
                #!/usr/bin/env bash
                sleep "$1"
            "#
            ))
            .unwrap();
        script
    });

    static DOES_NOT_EXIST_BIN: Lazy<assert_fs::fixture::ChildPath> =
        Lazy::new(|| TEMP_SCRIPT_DIR.child("does_not_exist_bin"));

    const DEBOUNCE_TIMEOUT: Duration = Duration::from_millis(100);

    async fn setup() -> (Api, DistantCtx, mpsc::UnboundedReceiver<Response>) {
        let api = Api::initialize(Config {
            watch: WatchConfig {
                debounce_timeout: DEBOUNCE_TIMEOUT,
                ..Default::default()
            },
        })
        .unwrap();
        let (reply, rx) = make_reply();
        let connection_id = rand::random();

        DistantApi::on_connect(&api, connection_id).await.unwrap();
        let ctx = DistantCtx {
            connection_id,
            reply,
        };
        (api, ctx, rx)
    }

    fn make_reply() -> (
        Box<dyn Reply<Data = Response>>,
        mpsc::UnboundedReceiver<Response>,
    ) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Box::new(tx), rx)
    }

    #[test(tokio::test)]
    async fn read_file_should_fail_if_file_missing() {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();
        let path = temp.child("missing-file").path().to_path_buf();

        let _ = api.read_file(ctx, path).await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn read_file_should_send_blob_with_file_contents() {
        let (api, ctx, _rx) = setup().await;

        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("test-file");
        file.write_str("some file contents").unwrap();

        let bytes = api.read_file(ctx, file.path().to_path_buf()).await.unwrap();
        assert_eq!(bytes, b"some file contents");
    }

    #[test(tokio::test)]
    async fn read_file_text_should_send_error_if_fails_to_read_file() {
        let (api, ctx, _rx) = setup().await;

        let temp = assert_fs::TempDir::new().unwrap();
        let path = temp.child("missing-file").path().to_path_buf();

        let _ = api.read_file_text(ctx, path).await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn read_file_text_should_send_text_with_file_contents() {
        let (api, ctx, _rx) = setup().await;

        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("test-file");
        file.write_str("some file contents").unwrap();

        let text = api
            .read_file_text(ctx, file.path().to_path_buf())
            .await
            .unwrap();
        assert_eq!(text, "some file contents");
    }

    #[test(tokio::test)]
    async fn write_file_should_send_error_if_fails_to_write_file() {
        let (api, ctx, _rx) = setup().await;

        // Create a temporary path and add to it to ensure that there are
        // extra components that don't exist to cause writing to fail
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("dir").child("test-file");

        let _ = api
            .write_file(ctx, file.path().to_path_buf(), b"some text".to_vec())
            .await
            .unwrap_err();

        // Also verify that we didn't actually create the file
        file.assert(predicate::path::missing());
    }

    #[test(tokio::test)]
    async fn write_file_should_send_ok_when_successful() {
        let (api, ctx, _rx) = setup().await;

        // Path should point to a file that does not exist, but all
        // other components leading up to it do
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("test-file");

        api.write_file(ctx, file.path().to_path_buf(), b"some text".to_vec())
            .await
            .unwrap();

        // Also verify that we actually did create the file
        // with the associated contents
        file.assert("some text");
    }

    #[test(tokio::test)]
    async fn write_file_text_should_send_error_if_fails_to_write_file() {
        let (api, ctx, _rx) = setup().await;

        // Create a temporary path and add to it to ensure that there are
        // extra components that don't exist to cause writing to fail
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("dir").child("test-file");

        api.write_file_text(ctx, file.path().to_path_buf(), "some text".to_string())
            .await
            .unwrap_err();

        // Also verify that we didn't actually create the file
        file.assert(predicate::path::missing());
    }

    #[test(tokio::test)]
    async fn write_file_text_should_send_ok_when_successful() {
        let (api, ctx, _rx) = setup().await;

        // Path should point to a file that does not exist, but all
        // other components leading up to it do
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("test-file");

        api.write_file_text(ctx, file.path().to_path_buf(), "some text".to_string())
            .await
            .unwrap();

        // Also verify that we actually did create the file
        // with the associated contents
        file.assert("some text");
    }

    #[test(tokio::test)]
    async fn append_file_should_send_error_if_fails_to_create_file() {
        let (api, ctx, _rx) = setup().await;

        // Create a temporary path and add to it to ensure that there are
        // extra components that don't exist to cause writing to fail
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("dir").child("test-file");

        api.append_file(
            ctx,
            file.path().to_path_buf(),
            b"some extra contents".to_vec(),
        )
        .await
        .unwrap_err();

        // Also verify that we didn't actually create the file
        file.assert(predicate::path::missing());
    }

    #[test(tokio::test)]
    async fn append_file_should_create_file_if_missing() {
        let (api, ctx, _rx) = setup().await;

        // Don't create the file directly, but define path
        // where the file should be
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("test-file");

        api.append_file(
            ctx,
            file.path().to_path_buf(),
            b"some extra contents".to_vec(),
        )
        .await
        .unwrap();

        // Yield to allow chance to finish appending to file
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Also verify that we actually did create to the file
        file.assert("some extra contents");
    }

    #[test(tokio::test)]
    async fn append_file_should_send_ok_when_successful() {
        let (api, ctx, _rx) = setup().await;

        // Create a temporary file and fill it with some contents
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("test-file");
        file.write_str("some file contents").unwrap();

        api.append_file(
            ctx,
            file.path().to_path_buf(),
            b"some extra contents".to_vec(),
        )
        .await
        .unwrap();

        // Yield to allow chance to finish appending to file
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Also verify that we actually did append to the file
        file.assert("some file contentssome extra contents");
    }

    #[test(tokio::test)]
    async fn append_file_text_should_send_error_if_fails_to_create_file() {
        let (api, ctx, _rx) = setup().await;

        // Create a temporary path and add to it to ensure that there are
        // extra components that don't exist to cause writing to fail
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("dir").child("test-file");

        let _ = api
            .append_file_text(
                ctx,
                file.path().to_path_buf(),
                "some extra contents".to_string(),
            )
            .await
            .unwrap_err();

        // Also verify that we didn't actually create the file
        file.assert(predicate::path::missing());
    }

    #[test(tokio::test)]
    async fn append_file_text_should_create_file_if_missing() {
        let (api, ctx, _rx) = setup().await;

        // Don't create the file directly, but define path
        // where the file should be
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("test-file");

        api.append_file_text(
            ctx,
            file.path().to_path_buf(),
            "some extra contents".to_string(),
        )
        .await
        .unwrap();

        // Yield to allow chance to finish appending to file
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Also verify that we actually did create to the file
        file.assert("some extra contents");
    }

    #[test(tokio::test)]
    async fn append_file_text_should_send_ok_when_successful() {
        let (api, ctx, _rx) = setup().await;

        // Create a temporary file and fill it with some contents
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("test-file");
        file.write_str("some file contents").unwrap();

        api.append_file_text(
            ctx,
            file.path().to_path_buf(),
            "some extra contents".to_string(),
        )
        .await
        .unwrap();

        // Yield to allow chance to finish appending to file
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Also verify that we actually did append to the file
        file.assert("some file contentssome extra contents");
    }

    #[test(tokio::test)]
    async fn dir_read_should_send_error_if_directory_does_not_exist() {
        let (api, ctx, _rx) = setup().await;

        let temp = assert_fs::TempDir::new().unwrap();
        let dir = temp.child("test-dir");

        let _ = api
            .read_dir(
                ctx,
                dir.path().to_path_buf(),
                /* depth */ 0,
                /* absolute */ false,
                /* canonicalize */ false,
                /* include_root */ false,
            )
            .await
            .unwrap_err();
    }

    // /root/
    // /root/file1
    // /root/link1 -> /root/sub1/file2
    // /root/sub1/
    // /root/sub1/file2
    async fn setup_dir() -> assert_fs::TempDir {
        let root_dir = assert_fs::TempDir::new().unwrap();
        root_dir.child("file1").touch().unwrap();

        let sub1 = root_dir.child("sub1");
        sub1.create_dir_all().unwrap();

        let file2 = sub1.child("file2");
        file2.touch().unwrap();

        let link1 = root_dir.child("link1");
        link1.symlink_to_file(file2.path()).unwrap();

        root_dir
    }

    #[test(tokio::test)]
    async fn dir_read_should_support_depth_limits() {
        let (api, ctx, _rx) = setup().await;

        // Create directory with some nested items
        let root_dir = setup_dir().await;

        let (entries, _) = api
            .read_dir(
                ctx,
                root_dir.path().to_path_buf(),
                /* depth */ 1,
                /* absolute */ false,
                /* canonicalize */ false,
                /* include_root */ false,
            )
            .await
            .unwrap();

        assert_eq!(entries.len(), 3, "Wrong number of entries found");

        assert_eq!(entries[0].file_type, FileType::File);
        assert_eq!(entries[0].path, Path::new("file1"));
        assert_eq!(entries[0].depth, 1);

        assert_eq!(entries[1].file_type, FileType::Symlink);
        assert_eq!(entries[1].path, Path::new("link1"));
        assert_eq!(entries[1].depth, 1);

        assert_eq!(entries[2].file_type, FileType::Dir);
        assert_eq!(entries[2].path, Path::new("sub1"));
        assert_eq!(entries[2].depth, 1);
    }

    #[test(tokio::test)]
    async fn dir_read_should_support_unlimited_depth_using_zero() {
        let (api, ctx, _rx) = setup().await;

        // Create directory with some nested items
        let root_dir = setup_dir().await;

        let (entries, _) = api
            .read_dir(
                ctx,
                root_dir.path().to_path_buf(),
                /* depth */ 0,
                /* absolute */ false,
                /* canonicalize */ false,
                /* include_root */ false,
            )
            .await
            .unwrap();

        assert_eq!(entries.len(), 4, "Wrong number of entries found");

        assert_eq!(entries[0].file_type, FileType::File);
        assert_eq!(entries[0].path, Path::new("file1"));
        assert_eq!(entries[0].depth, 1);

        assert_eq!(entries[1].file_type, FileType::Symlink);
        assert_eq!(entries[1].path, Path::new("link1"));
        assert_eq!(entries[1].depth, 1);

        assert_eq!(entries[2].file_type, FileType::Dir);
        assert_eq!(entries[2].path, Path::new("sub1"));
        assert_eq!(entries[2].depth, 1);

        assert_eq!(entries[3].file_type, FileType::File);
        assert_eq!(entries[3].path, Path::new("sub1").join("file2"));
        assert_eq!(entries[3].depth, 2);
    }

    #[test(tokio::test)]
    async fn dir_read_should_support_including_directory_in_returned_entries() {
        let (api, ctx, _rx) = setup().await;

        // Create directory with some nested items
        let root_dir = setup_dir().await;

        let (entries, _) = api
            .read_dir(
                ctx,
                root_dir.path().to_path_buf(),
                /* depth */ 1,
                /* absolute */ false,
                /* canonicalize */ false,
                /* include_root */ true,
            )
            .await
            .unwrap();

        assert_eq!(entries.len(), 4, "Wrong number of entries found");

        // NOTE: Root entry is always absolute, resolved path
        assert_eq!(entries[0].file_type, FileType::Dir);
        assert_eq!(entries[0].path, root_dir.path().canonicalize().unwrap());
        assert_eq!(entries[0].depth, 0);

        assert_eq!(entries[1].file_type, FileType::File);
        assert_eq!(entries[1].path, Path::new("file1"));
        assert_eq!(entries[1].depth, 1);

        assert_eq!(entries[2].file_type, FileType::Symlink);
        assert_eq!(entries[2].path, Path::new("link1"));
        assert_eq!(entries[2].depth, 1);

        assert_eq!(entries[3].file_type, FileType::Dir);
        assert_eq!(entries[3].path, Path::new("sub1"));
        assert_eq!(entries[3].depth, 1);
    }

    #[test(tokio::test)]
    async fn dir_read_should_support_returning_absolute_paths() {
        let (api, ctx, _rx) = setup().await;

        // Create directory with some nested items
        let root_dir = setup_dir().await;

        let (entries, _) = api
            .read_dir(
                ctx,
                root_dir.path().to_path_buf(),
                /* depth */ 1,
                /* absolute */ true,
                /* canonicalize */ false,
                /* include_root */ false,
            )
            .await
            .unwrap();

        assert_eq!(entries.len(), 3, "Wrong number of entries found");
        let root_path = root_dir.path().canonicalize().unwrap();

        assert_eq!(entries[0].file_type, FileType::File);
        assert_eq!(entries[0].path, root_path.join("file1"));
        assert_eq!(entries[0].depth, 1);

        assert_eq!(entries[1].file_type, FileType::Symlink);
        assert_eq!(entries[1].path, root_path.join("link1"));
        assert_eq!(entries[1].depth, 1);

        assert_eq!(entries[2].file_type, FileType::Dir);
        assert_eq!(entries[2].path, root_path.join("sub1"));
        assert_eq!(entries[2].depth, 1);
    }

    #[test(tokio::test)]
    async fn dir_read_should_support_returning_canonicalized_paths() {
        let (api, ctx, _rx) = setup().await;

        // Create directory with some nested items
        let root_dir = setup_dir().await;

        let (entries, _) = api
            .read_dir(
                ctx,
                root_dir.path().to_path_buf(),
                /* depth */ 1,
                /* absolute */ false,
                /* canonicalize */ true,
                /* include_root */ false,
            )
            .await
            .unwrap();

        assert_eq!(entries.len(), 3, "Wrong number of entries found");

        assert_eq!(entries[0].file_type, FileType::File);
        assert_eq!(entries[0].path, Path::new("file1"));
        assert_eq!(entries[0].depth, 1);

        // Symlink should be resolved from $ROOT/link1 -> $ROOT/sub1/file2
        assert_eq!(entries[1].file_type, FileType::Symlink);
        assert_eq!(entries[1].path, Path::new("sub1").join("file2"));
        assert_eq!(entries[1].depth, 1);

        assert_eq!(entries[2].file_type, FileType::Dir);
        assert_eq!(entries[2].path, Path::new("sub1"));
        assert_eq!(entries[2].depth, 1);
    }

    #[test(tokio::test)]
    async fn create_dir_should_send_error_if_fails() {
        let (api, ctx, _rx) = setup().await;

        // Make a path that has multiple non-existent components
        // so the creation will fail
        let root_dir = setup_dir().await;
        let path = root_dir.path().join("nested").join("new-dir");

        let _ = api
            .create_dir(ctx, path.to_path_buf(), /* all */ false)
            .await
            .unwrap_err();

        // Also verify that the directory was not actually created
        assert!(!path.exists(), "Path unexpectedly exists");
    }

    #[test(tokio::test)]
    async fn create_dir_should_send_ok_when_successful() {
        let (api, ctx, _rx) = setup().await;
        let root_dir = setup_dir().await;
        let path = root_dir.path().join("new-dir");

        api.create_dir(ctx, path.to_path_buf(), /* all */ false)
            .await
            .unwrap();

        // Also verify that the directory was actually created
        assert!(path.exists(), "Directory not created");
    }

    #[test(tokio::test)]
    async fn create_dir_should_support_creating_multiple_dir_components() {
        let (api, ctx, _rx) = setup().await;
        let root_dir = setup_dir().await;
        let path = root_dir.path().join("nested").join("new-dir");

        api.create_dir(ctx, path.to_path_buf(), /* all */ true)
            .await
            .unwrap();

        // Also verify that the directory was actually created
        assert!(path.exists(), "Directory not created");
    }

    #[test(tokio::test)]
    async fn remove_should_send_error_on_failure() {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("missing-file");

        let _ = api
            .remove(ctx, file.path().to_path_buf(), /* false */ false)
            .await
            .unwrap_err();

        // Also, verify that path does not exist
        file.assert(predicate::path::missing());
    }

    #[test(tokio::test)]
    async fn remove_should_support_deleting_a_directory() {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();
        let dir = temp.child("dir");
        dir.create_dir_all().unwrap();

        api.remove(ctx, dir.path().to_path_buf(), /* false */ false)
            .await
            .unwrap();

        // Also, verify that path does not exist
        dir.assert(predicate::path::missing());
    }

    #[test(tokio::test)]
    async fn remove_should_delete_nonempty_directory_if_force_is_true() {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();
        let dir = temp.child("dir");
        dir.create_dir_all().unwrap();
        dir.child("file").touch().unwrap();

        api.remove(ctx, dir.path().to_path_buf(), /* false */ true)
            .await
            .unwrap();

        // Also, verify that path does not exist
        dir.assert(predicate::path::missing());
    }

    #[test(tokio::test)]
    async fn remove_should_support_deleting_a_single_file() {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("some-file");
        file.touch().unwrap();

        api.remove(ctx, file.path().to_path_buf(), /* false */ false)
            .await
            .unwrap();

        // Also, verify that path does not exist
        file.assert(predicate::path::missing());
    }

    #[test(tokio::test)]
    async fn copy_should_send_error_on_failure() {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();
        let src = temp.child("src");
        let dst = temp.child("dst");

        let _ = api
            .copy(ctx, src.path().to_path_buf(), dst.path().to_path_buf())
            .await
            .unwrap_err();

        // Also, verify that destination does not exist
        dst.assert(predicate::path::missing());
    }

    #[test(tokio::test)]
    async fn copy_should_support_copying_an_entire_directory() {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();

        let src = temp.child("src");
        src.create_dir_all().unwrap();
        let src_file = src.child("file");
        src_file.write_str("some contents").unwrap();

        let dst = temp.child("dst");
        let dst_file = dst.child("file");

        api.copy(ctx, src.path().to_path_buf(), dst.path().to_path_buf())
            .await
            .unwrap();

        // Verify that we have source and destination directories and associated contents
        src.assert(predicate::path::is_dir());
        src_file.assert(predicate::path::is_file());
        dst.assert(predicate::path::is_dir());
        dst_file.assert(predicate::path::eq_file(src_file.path()));
    }

    #[test(tokio::test)]
    async fn copy_should_support_copying_an_empty_directory() {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();
        let src = temp.child("src");
        src.create_dir_all().unwrap();
        let dst = temp.child("dst");

        api.copy(ctx, src.path().to_path_buf(), dst.path().to_path_buf())
            .await
            .unwrap();

        // Verify that we still have source and destination directories
        src.assert(predicate::path::is_dir());
        dst.assert(predicate::path::is_dir());
    }

    #[test(tokio::test)]
    async fn copy_should_support_copying_a_directory_that_only_contains_directories() {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();

        let src = temp.child("src");
        src.create_dir_all().unwrap();
        let src_dir = src.child("dir");
        src_dir.create_dir_all().unwrap();

        let dst = temp.child("dst");
        let dst_dir = dst.child("dir");

        api.copy(ctx, src.path().to_path_buf(), dst.path().to_path_buf())
            .await
            .unwrap();

        // Verify that we have source and destination directories and associated contents
        src.assert(predicate::path::is_dir().name("src"));
        src_dir.assert(predicate::path::is_dir().name("src/dir"));
        dst.assert(predicate::path::is_dir().name("dst"));
        dst_dir.assert(predicate::path::is_dir().name("dst/dir"));
    }

    #[test(tokio::test)]
    async fn copy_should_support_copying_a_single_file() {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();
        let src = temp.child("src");
        src.write_str("some text").unwrap();
        let dst = temp.child("dst");

        api.copy(ctx, src.path().to_path_buf(), dst.path().to_path_buf())
            .await
            .unwrap();

        // Verify that we still have source and that destination has source's contents
        src.assert(predicate::path::is_file());
        dst.assert(predicate::path::eq_file(src.path()));
    }

    #[test(tokio::test)]
    async fn rename_should_fail_if_path_missing() {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();
        let src = temp.child("src");
        let dst = temp.child("dst");

        let _ = api
            .rename(ctx, src.path().to_path_buf(), dst.path().to_path_buf())
            .await
            .unwrap_err();

        // Also, verify that destination does not exist
        dst.assert(predicate::path::missing());
    }

    #[test(tokio::test)]
    async fn rename_should_support_renaming_an_entire_directory() {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();

        let src = temp.child("src");
        src.create_dir_all().unwrap();
        let src_file = src.child("file");
        src_file.write_str("some contents").unwrap();

        let dst = temp.child("dst");
        let dst_file = dst.child("file");

        api.rename(ctx, src.path().to_path_buf(), dst.path().to_path_buf())
            .await
            .unwrap();

        // Verify that we moved the contents
        src.assert(predicate::path::missing());
        src_file.assert(predicate::path::missing());
        dst.assert(predicate::path::is_dir());
        dst_file.assert("some contents");
    }

    #[test(tokio::test)]
    async fn rename_should_support_renaming_a_single_file() {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();
        let src = temp.child("src");
        src.write_str("some text").unwrap();
        let dst = temp.child("dst");

        api.rename(ctx, src.path().to_path_buf(), dst.path().to_path_buf())
            .await
            .unwrap();

        // Verify that we moved the file
        src.assert(predicate::path::missing());
        dst.assert("some text");
    }

    /// Validates a response as being a series of changes that include the provided paths
    fn validate_changed_path(data: &Response, expected_path: &Path, should_panic: bool) -> bool {
        match data {
            Response::Changed(change) if should_panic => {
                let path = change.path.canonicalize().unwrap();
                assert_eq!(path, expected_path, "Wrong path reported: {:?}", change);

                true
            }
            Response::Changed(change) => {
                let path = change.path.canonicalize().unwrap();
                path == expected_path
            }
            x if should_panic => panic!("Unexpected response: {:?}", x),
            _ => false,
        }
    }

    #[test(tokio::test)]
    async fn watch_should_support_watching_a_single_file() {
        // NOTE: Supporting multiple replies being sent back as part of creating, modifying, etc.
        let (api, ctx, mut rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();

        let file = temp.child("file");
        file.touch().unwrap();

        api.watch(
            ctx,
            file.path().to_path_buf(),
            /* recursive */ false,
            /* only */ Default::default(),
            /* except */ Default::default(),
        )
        .await
        .unwrap();

        // Update the file and verify we get a notification
        file.write_str("some text").unwrap();

        let data = rx
            .recv()
            .await
            .expect("Channel closed before we got change");
        validate_changed_path(
            &data,
            &file.path().to_path_buf().canonicalize().unwrap(),
            /* should_panic */ true,
        );
    }

    #[test(tokio::test)]
    async fn watch_should_support_watching_a_directory_recursively() {
        // NOTE: Supporting multiple replies being sent back as part of creating, modifying, etc.
        let (api, ctx, mut rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();

        let file = temp.child("file");
        file.touch().unwrap();

        let dir = temp.child("dir");
        dir.create_dir_all().unwrap();

        api.watch(
            ctx,
            temp.path().to_path_buf(),
            /* recursive */ true,
            /* only */ Default::default(),
            /* except */ Default::default(),
        )
        .await
        .unwrap();

        // Update the file and verify we get a notification
        file.write_str("some text").unwrap();

        // Create a nested file and verify we get a notification
        let nested_file = dir.child("nested-file");
        nested_file.write_str("some text").unwrap();

        // Sleep a bit to give time to get all changes happening
        // TODO: Can we slim down this sleep? Or redesign test in some other way?
        tokio::time::sleep(DEBOUNCE_TIMEOUT + Duration::from_millis(100)).await;

        // Collect all responses, as we may get multiple for interactions within a directory
        let mut responses = Vec::new();
        while let Ok(res) = rx.try_recv() {
            responses.push(res);
        }

        // Validate that we have at least one change reported for each of our paths
        assert!(
            responses.len() >= 2,
            "Less than expected total responses: {:?}",
            responses
        );

        let path = file.path().to_path_buf();
        assert!(
            responses.iter().any(|res| validate_changed_path(
                res,
                &file.path().to_path_buf().canonicalize().unwrap(),
                /* should_panic */ false,
            )),
            "Missing {:?} in {:?}",
            path,
            responses
                .iter()
                .map(|x| format!("{:?}", x))
                .collect::<Vec<String>>(),
        );

        let path = nested_file.path().to_path_buf();
        assert!(
            responses.iter().any(|res| validate_changed_path(
                res,
                &file.path().to_path_buf().canonicalize().unwrap(),
                /* should_panic */ false,
            )),
            "Missing {:?} in {:?}",
            path,
            responses
                .iter()
                .map(|x| format!("{:?}", x))
                .collect::<Vec<String>>(),
        );
    }

    #[test(tokio::test)]
    async fn watch_should_report_changes_using_the_ctx_replies() {
        // NOTE: Supporting multiple replies being sent back as part of creating, modifying, etc.
        let (api, ctx_1, mut rx_1) = setup().await;
        let (ctx_2, mut rx_2) = {
            let (reply, rx) = make_reply();
            let ctx = DistantCtx {
                connection_id: ctx_1.connection_id,
                reply,
            };
            (ctx, rx)
        };

        let temp = assert_fs::TempDir::new().unwrap();

        let file_1 = temp.child("file_1");
        file_1.touch().unwrap();

        let file_2 = temp.child("file_2");
        file_2.touch().unwrap();

        // Sleep a bit to give time to get all changes happening
        // TODO: Can we slim down this sleep? Or redesign test in some other way?
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Initialize watch on file 1
        api.watch(
            ctx_1,
            file_1.path().to_path_buf(),
            /* recursive */ false,
            /* only */ Default::default(),
            /* except */ Default::default(),
        )
        .await
        .unwrap();

        // Initialize watch on file 2
        api.watch(
            ctx_2,
            file_2.path().to_path_buf(),
            /* recursive */ false,
            /* only */ Default::default(),
            /* except */ Default::default(),
        )
        .await
        .unwrap();

        // Update the files and verify we get notifications from different origins
        file_1.write_str("some text").unwrap();
        let data = rx_1
            .recv()
            .await
            .expect("Channel closed before we got change");
        validate_changed_path(
            &data,
            &file_1.path().to_path_buf().canonicalize().unwrap(),
            /* should_panic */ true,
        );

        // Update the files and verify we get notifications from different origins
        file_2.write_str("some text").unwrap();
        let data = rx_2
            .recv()
            .await
            .expect("Channel closed before we got change");
        validate_changed_path(
            &data,
            &file_2.path().to_path_buf().canonicalize().unwrap(),
            /* should_panic */ true,
        );
    }

    #[test(tokio::test)]
    async fn exists_should_send_true_if_path_exists() {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file");
        file.touch().unwrap();

        let exists = api.exists(ctx, file.path().to_path_buf()).await.unwrap();
        assert!(exists, "Expected exists to be true, but was false");
    }

    #[test(tokio::test)]
    async fn exists_should_send_false_if_path_does_not_exist() {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file");

        let exists = api.exists(ctx, file.path().to_path_buf()).await.unwrap();
        assert!(!exists, "Expected exists to be false, but was true");
    }

    #[test(tokio::test)]
    async fn metadata_should_send_error_on_failure() {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file");

        let _ = api
            .metadata(
                ctx,
                file.path().to_path_buf(),
                /* canonicalize */ false,
                /* resolve_file_type */ false,
            )
            .await
            .unwrap_err();
    }

    #[test(tokio::test)]
    async fn metadata_should_send_back_metadata_on_file_if_exists() {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file");
        file.write_str("some text").unwrap();

        let metadata = api
            .metadata(
                ctx,
                file.path().to_path_buf(),
                /* canonicalize */ false,
                /* resolve_file_type */ false,
            )
            .await
            .unwrap();

        assert!(
            matches!(
                metadata,
                Metadata {
                    canonicalized_path: None,
                    file_type: FileType::File,
                    len: 9,
                    readonly: false,
                    ..
                }
            ),
            "{:?}",
            metadata
        );
    }

    #[cfg(unix)]
    #[test(tokio::test)]
    async fn metadata_should_include_unix_specific_metadata_on_unix_platform() {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file");
        file.write_str("some text").unwrap();

        let metadata = api
            .metadata(
                ctx,
                file.path().to_path_buf(),
                /* canonicalize */ false,
                /* resolve_file_type */ false,
            )
            .await
            .unwrap();

        #[allow(clippy::match_single_binding)]
        match metadata {
            Metadata { unix, windows, .. } => {
                assert!(unix.is_some(), "Unexpectedly missing unix metadata on unix");
                assert!(
                    windows.is_none(),
                    "Unexpectedly got windows metadata on unix"
                );
            }
        }
    }

    #[cfg(windows)]
    #[test(tokio::test)]
    async fn metadata_should_include_windows_specific_metadata_on_windows_platform() {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file");
        file.write_str("some text").unwrap();

        let metadata = api
            .metadata(
                ctx,
                file.path().to_path_buf(),
                /* canonicalize */ false,
                /* resolve_file_type */ false,
            )
            .await
            .unwrap();

        #[allow(clippy::match_single_binding)]
        match metadata {
            Metadata { unix, windows, .. } => {
                assert!(
                    windows.is_some(),
                    "Unexpectedly missing windows metadata on windows"
                );
                assert!(unix.is_none(), "Unexpectedly got unix metadata on windows");
            }
        }
    }

    #[test(tokio::test)]
    async fn metadata_should_send_back_metadata_on_dir_if_exists() {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();
        let dir = temp.child("dir");
        dir.create_dir_all().unwrap();

        let metadata = api
            .metadata(
                ctx,
                dir.path().to_path_buf(),
                /* canonicalize */ false,
                /* resolve_file_type */ false,
            )
            .await
            .unwrap();

        assert!(
            matches!(
                metadata,
                Metadata {
                    canonicalized_path: None,
                    file_type: FileType::Dir,
                    readonly: false,
                    ..
                }
            ),
            "{:?}",
            metadata
        );
    }

    #[test(tokio::test)]
    async fn metadata_should_send_back_metadata_on_symlink_if_exists() {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file");
        file.write_str("some text").unwrap();

        let symlink = temp.child("link");
        symlink.symlink_to_file(file.path()).unwrap();

        let metadata = api
            .metadata(
                ctx,
                symlink.path().to_path_buf(),
                /* canonicalize */ false,
                /* resolve_file_type */ false,
            )
            .await
            .unwrap();

        assert!(
            matches!(
                metadata,
                Metadata {
                    canonicalized_path: None,
                    file_type: FileType::Symlink,
                    readonly: false,
                    ..
                }
            ),
            "{:?}",
            metadata
        );
    }

    #[test(tokio::test)]
    async fn metadata_should_include_canonicalized_path_if_flag_specified() {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file");
        file.write_str("some text").unwrap();

        let symlink = temp.child("link");
        symlink.symlink_to_file(file.path()).unwrap();

        let metadata = api
            .metadata(
                ctx,
                symlink.path().to_path_buf(),
                /* canonicalize */ true,
                /* resolve_file_type */ false,
            )
            .await
            .unwrap();

        match metadata {
            Metadata {
                canonicalized_path: Some(path),
                file_type: FileType::Symlink,
                readonly: false,
                ..
            } => assert_eq!(
                path,
                file.path().canonicalize().unwrap(),
                "Symlink canonicalized path does not match referenced file"
            ),
            x => panic!("Unexpected response: {:?}", x),
        }
    }

    #[test(tokio::test)]
    async fn metadata_should_resolve_file_type_of_symlink_if_flag_specified() {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file");
        file.write_str("some text").unwrap();

        let symlink = temp.child("link");
        symlink.symlink_to_file(file.path()).unwrap();

        let metadata = api
            .metadata(
                ctx,
                symlink.path().to_path_buf(),
                /* canonicalize */ false,
                /* resolve_file_type */ true,
            )
            .await
            .unwrap();

        assert!(
            matches!(
                metadata,
                Metadata {
                    file_type: FileType::File,
                    ..
                }
            ),
            "{:?}",
            metadata
        );
    }

    #[test(tokio::test)]
    async fn set_permissions_should_set_readonly_flag_if_specified() {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file");
        file.write_str("some text").unwrap();

        // Verify that not readonly by default
        let permissions = tokio::fs::symlink_metadata(file.path())
            .await
            .unwrap()
            .permissions();
        assert!(!permissions.readonly(), "File is already set to readonly");

        // Change the file permissions
        api.set_permissions(
            ctx,
            file.path().to_path_buf(),
            Permissions::readonly(),
            Default::default(),
        )
        .await
        .unwrap();

        // Retrieve permissions to verify set
        let permissions = tokio::fs::symlink_metadata(file.path())
            .await
            .unwrap()
            .permissions();
        assert!(permissions.readonly(), "File not set to readonly");
    }

    #[test(tokio::test)]
    #[cfg_attr(not(unix), ignore)]
    async fn set_permissions_should_set_unix_permissions_if_on_unix_platform() {
        #[cfg(unix)]
        {
            use std::os::unix::prelude::*;

            let (api, ctx, _rx) = setup().await;
            let temp = assert_fs::TempDir::new().unwrap();
            let file = temp.child("file");
            file.write_str("some text").unwrap();

            // Verify that permissions do not match our readonly state
            let permissions = tokio::fs::symlink_metadata(file.path())
                .await
                .unwrap()
                .permissions();
            let mode = permissions.mode() & 0o777;
            assert_ne!(mode, 0o400, "File is already set to 0o400");

            // Change the file permissions
            api.set_permissions(
                ctx,
                file.path().to_path_buf(),
                Permissions::from_unix_mode(0o400),
                Default::default(),
            )
            .await
            .unwrap();

            // Retrieve file permissions to verify set
            let permissions = tokio::fs::symlink_metadata(file.path())
                .await
                .unwrap()
                .permissions();

            // Drop the upper bits that mode can have (only care about read/write/exec)
            let mode = permissions.mode() & 0o777;

            assert_eq!(mode, 0o400, "Wrong permissions on file: {:o}", mode);
        }
        #[cfg(not(unix))]
        {
            unreachable!();
        }
    }

    #[test(tokio::test)]
    #[cfg_attr(unix, ignore)]
    async fn set_permissions_should_set_readonly_flag_if_not_on_unix_platform() {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file");
        file.write_str("some text").unwrap();

        // Verify that not readonly by default
        let permissions = tokio::fs::symlink_metadata(file.path())
            .await
            .unwrap()
            .permissions();
        assert!(!permissions.readonly(), "File is already set to readonly");

        // Change the file permissions to be readonly (in general)
        api.set_permissions(
            ctx,
            file.path().to_path_buf(),
            Permissions::from_unix_mode(0o400),
            Default::default(),
        )
        .await
        .unwrap();

        #[cfg(not(unix))]
        {
            // Retrieve file permissions to verify set
            let permissions = tokio::fs::symlink_metadata(file.path())
                .await
                .unwrap()
                .permissions();

            assert!(permissions.readonly(), "File not marked as readonly");
        }
        #[cfg(unix)]
        {
            unreachable!();
        }
    }

    #[test(tokio::test)]
    async fn set_permissions_should_not_recurse_if_option_false() {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file");
        file.write_str("some text").unwrap();

        let symlink = temp.child("link");
        symlink.symlink_to_file(file.path()).unwrap();

        // Verify that dir is not readonly by default
        let permissions = tokio::fs::symlink_metadata(temp.path())
            .await
            .unwrap()
            .permissions();
        assert!(
            !permissions.readonly(),
            "Temp dir is already set to readonly"
        );

        // Verify that file is not readonly by default
        let permissions = tokio::fs::symlink_metadata(file.path())
            .await
            .unwrap()
            .permissions();
        assert!(!permissions.readonly(), "File is already set to readonly");

        // Verify that symlink is not readonly by default
        let permissions = tokio::fs::symlink_metadata(symlink.path())
            .await
            .unwrap()
            .permissions();
        assert!(
            !permissions.readonly(),
            "Symlink is already set to readonly"
        );

        // Change the permissions of the directory and not the contents underneath
        api.set_permissions(
            ctx,
            temp.path().to_path_buf(),
            Permissions::readonly(),
            SetPermissionsOptions {
                recursive: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // Retrieve permissions of the file, symlink, and directory to verify set
        let permissions = tokio::fs::symlink_metadata(temp.path())
            .await
            .unwrap()
            .permissions();
        assert!(permissions.readonly(), "Temp directory not set to readonly");

        let permissions = tokio::fs::symlink_metadata(file.path())
            .await
            .unwrap()
            .permissions();
        assert!(!permissions.readonly(), "File unexpectedly set to readonly");

        let permissions = tokio::fs::symlink_metadata(symlink.path())
            .await
            .unwrap()
            .permissions();
        assert!(
            !permissions.readonly(),
            "Symlink unexpectedly set to readonly"
        );
    }

    #[test(tokio::test)]
    async fn set_permissions_should_traverse_symlinks_while_recursing_if_following_symlinks_enabled(
    ) {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file");
        file.write_str("some text").unwrap();

        let temp2 = assert_fs::TempDir::new().unwrap();
        let file2 = temp2.child("file");
        file2.write_str("some text").unwrap();

        let symlink = temp.child("link");
        symlink.symlink_to_dir(temp2.path()).unwrap();

        // Verify that symlink is not readonly by default
        let permissions = tokio::fs::symlink_metadata(file2.path())
            .await
            .unwrap()
            .permissions();
        assert!(!permissions.readonly(), "File2 is already set to readonly");

        // Change the main directory permissions
        api.set_permissions(
            ctx,
            temp.path().to_path_buf(),
            Permissions::readonly(),
            SetPermissionsOptions {
                follow_symlinks: true,
                recursive: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // Retrieve permissions referenced by another directory
        let permissions = tokio::fs::symlink_metadata(file2.path())
            .await
            .unwrap()
            .permissions();
        assert!(permissions.readonly(), "File2 not set to readonly");
    }

    #[test(tokio::test)]
    async fn set_permissions_should_not_traverse_symlinks_while_recursing_if_following_symlinks_disabled(
    ) {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file");
        file.write_str("some text").unwrap();

        let temp2 = assert_fs::TempDir::new().unwrap();
        let file2 = temp2.child("file");
        file2.write_str("some text").unwrap();

        let symlink = temp.child("link");
        symlink.symlink_to_dir(temp2.path()).unwrap();

        // Verify that symlink is not readonly by default
        let permissions = tokio::fs::symlink_metadata(file2.path())
            .await
            .unwrap()
            .permissions();
        assert!(!permissions.readonly(), "File2 is already set to readonly");

        // Change the main directory permissions
        api.set_permissions(
            ctx,
            temp.path().to_path_buf(),
            Permissions::readonly(),
            SetPermissionsOptions {
                follow_symlinks: false,
                recursive: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // Retrieve permissions referenced by another directory
        let permissions = tokio::fs::symlink_metadata(file2.path())
            .await
            .unwrap()
            .permissions();
        assert!(
            !permissions.readonly(),
            "File2 unexpectedly set to readonly"
        );
    }

    #[test(tokio::test)]
    async fn set_permissions_should_skip_symlinks_if_exclude_symlinks_enabled() {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file");
        file.write_str("some text").unwrap();

        let symlink = temp.child("link");
        symlink.symlink_to_file(file.path()).unwrap();

        // Verify that symlink is not readonly by default
        let permissions = tokio::fs::symlink_metadata(symlink.path())
            .await
            .unwrap()
            .permissions();
        assert!(
            !permissions.readonly(),
            "Symlink is already set to readonly"
        );

        // Change the symlink permissions
        api.set_permissions(
            ctx,
            symlink.path().to_path_buf(),
            Permissions::readonly(),
            SetPermissionsOptions {
                exclude_symlinks: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // Retrieve permissions to verify not set
        let permissions = tokio::fs::symlink_metadata(symlink.path())
            .await
            .unwrap()
            .permissions();
        assert!(
            !permissions.readonly(),
            "Symlink (or file underneath) set to readonly"
        );
    }

    #[test(tokio::test)]
    async fn set_permissions_should_support_recursive_if_option_specified() {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file");
        file.write_str("some text").unwrap();

        // Verify that dir is not readonly by default
        let permissions = tokio::fs::symlink_metadata(temp.path())
            .await
            .unwrap()
            .permissions();
        assert!(
            !permissions.readonly(),
            "Temp dir is already set to readonly"
        );

        // Verify that file is not readonly by default
        let permissions = tokio::fs::symlink_metadata(file.path())
            .await
            .unwrap()
            .permissions();
        assert!(!permissions.readonly(), "File is already set to readonly");

        // Change the permissions of the file pointed to by the symlink
        api.set_permissions(
            ctx,
            temp.path().to_path_buf(),
            Permissions::readonly(),
            SetPermissionsOptions {
                recursive: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // Retrieve permissions of the file, symlink, and directory to verify set
        let permissions = tokio::fs::symlink_metadata(temp.path())
            .await
            .unwrap()
            .permissions();
        assert!(permissions.readonly(), "Temp directory not set to readonly");

        let permissions = tokio::fs::symlink_metadata(file.path())
            .await
            .unwrap()
            .permissions();
        assert!(permissions.readonly(), "File not set to readonly");
    }

    #[test(tokio::test)]
    async fn set_permissions_should_support_following_explicit_symlink_if_option_specified() {
        let (api, ctx, _rx) = setup().await;
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file");
        file.write_str("some text").unwrap();

        let symlink = temp.child("link");
        symlink.symlink_to_file(file.path()).unwrap();

        // Verify that file is not readonly by default
        let permissions = tokio::fs::symlink_metadata(file.path())
            .await
            .unwrap()
            .permissions();
        assert!(!permissions.readonly(), "File is already set to readonly");

        // Verify that symlink is not readonly by default
        let permissions = tokio::fs::symlink_metadata(symlink.path())
            .await
            .unwrap()
            .permissions();
        assert!(
            !permissions.readonly(),
            "Symlink is already set to readonly"
        );

        // Change the permissions of the file pointed to by the symlink
        api.set_permissions(
            ctx,
            symlink.path().to_path_buf(),
            Permissions::readonly(),
            SetPermissionsOptions {
                follow_symlinks: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // Retrieve permissions of the file and symlink to verify set
        let permissions = tokio::fs::symlink_metadata(file.path())
            .await
            .unwrap()
            .permissions();
        assert!(permissions.readonly(), "File not set to readonly");

        let permissions = tokio::fs::symlink_metadata(symlink.path())
            .await
            .unwrap()
            .permissions();
        assert!(
            !permissions.readonly(),
            "Symlink unexpectedly set to readonly"
        );
    }

    // NOTE: Ignoring on windows because it's using WSL which wants a Linux path
    //       with / but thinks it's on windows and is providing \
    #[test(tokio::test)]
    #[cfg_attr(windows, ignore)]
    async fn proc_spawn_should_send_error_on_failure() {
        let (api, ctx, _rx) = setup().await;

        let _ = api
            .proc_spawn(
                ctx,
                /* cmd */ DOES_NOT_EXIST_BIN.to_str().unwrap().to_string(),
                /* environment */ Environment::new(),
                /* current_dir */ None,
                /* pty */ None,
            )
            .await
            .unwrap_err();
    }

    // NOTE: Ignoring on windows because it's using WSL which wants a Linux path
    //       with / but thinks it's on windows and is providing \
    #[test(tokio::test)]
    #[cfg_attr(windows, ignore)]
    async fn proc_spawn_should_return_id_of_spawned_process() {
        let (api, ctx, _rx) = setup().await;

        let id = api
            .proc_spawn(
                ctx,
                /* cmd */
                format!(
                    "{} {}",
                    *SCRIPT_RUNNER,
                    ECHO_ARGS_TO_STDOUT_SH.to_str().unwrap()
                ),
                /* environment */ Environment::new(),
                /* current_dir */ None,
                /* pty */ None,
            )
            .await
            .unwrap();
        assert!(id > 0);
    }

    // NOTE: Ignoring on windows because it's using WSL which wants a Linux path
    //       with / but thinks it's on windows and is providing \
    #[test(tokio::test)]
    #[cfg_attr(windows, ignore)]
    async fn proc_spawn_should_send_back_stdout_periodically_when_available() {
        let (api, ctx, mut rx) = setup().await;

        let proc_id = api
            .proc_spawn(
                ctx,
                /* cmd */
                format!(
                    "{} {} some stdout",
                    *SCRIPT_RUNNER,
                    ECHO_ARGS_TO_STDOUT_SH.to_str().unwrap()
                ),
                /* environment */ Environment::new(),
                /* current_dir */ None,
                /* pty */ None,
            )
            .await
            .unwrap();

        // Gather two additional responses:
        //
        // 1. An indirect response for stdout
        // 2. An indirect response that is proc completing
        //
        // Note that order is not a guarantee, so we have to check that
        // we get one of each type of response
        let data_1 = rx.recv().await.expect("Missing first response");
        let data_2 = rx.recv().await.expect("Missing second response");

        let mut got_stdout = false;
        let mut got_done = false;

        let mut check_data = |data: &Response| match data {
            Response::ProcStdout { id, data } => {
                assert_eq!(
                    *id, proc_id,
                    "Got {}, but expected {} as process id",
                    id, proc_id
                );
                assert_eq!(data, b"some stdout", "Got wrong stdout");
                got_stdout = true;
            }
            Response::ProcDone { id, success, .. } => {
                assert_eq!(
                    *id, proc_id,
                    "Got {}, but expected {} as process id",
                    id, proc_id
                );
                assert!(success, "Process should have completed successfully");
                got_done = true;
            }
            x => panic!("Unexpected response: {:?}", x),
        };

        check_data(&data_1);
        check_data(&data_2);
        assert!(got_stdout, "Missing stdout response");
        assert!(got_done, "Missing done response");
    }

    // NOTE: Ignoring on windows because it's using WSL which wants a Linux path
    //       with / but thinks it's on windows and is providing \
    #[test(tokio::test)]
    #[cfg_attr(windows, ignore)]
    async fn proc_spawn_should_send_back_stderr_periodically_when_available() {
        let (api, ctx, mut rx) = setup().await;

        let proc_id = api
            .proc_spawn(
                ctx,
                /* cmd */
                format!(
                    "{} {} some stderr",
                    *SCRIPT_RUNNER,
                    ECHO_ARGS_TO_STDERR_SH.to_str().unwrap()
                ),
                /* environment */ Environment::new(),
                /* current_dir */ None,
                /* pty */ None,
            )
            .await
            .unwrap();

        // Gather two additional responses:
        //
        // 1. An indirect response for stderr
        // 2. An indirect response that is proc completing
        //
        // Note that order is not a guarantee, so we have to check that
        // we get one of each type of response
        let data_1 = rx.recv().await.expect("Missing first response");
        let data_2 = rx.recv().await.expect("Missing second response");

        let mut got_stderr = false;
        let mut got_done = false;

        let mut check_data = |data: &Response| match data {
            Response::ProcStderr { id, data } => {
                assert_eq!(
                    *id, proc_id,
                    "Got {}, but expected {} as process id",
                    id, proc_id
                );
                assert_eq!(data, b"some stderr", "Got wrong stderr");
                got_stderr = true;
            }
            Response::ProcDone { id, success, .. } => {
                assert_eq!(
                    *id, proc_id,
                    "Got {}, but expected {} as process id",
                    id, proc_id
                );
                assert!(success, "Process should have completed successfully");
                got_done = true;
            }
            x => panic!("Unexpected response: {:?}", x),
        };

        check_data(&data_1);
        check_data(&data_2);
        assert!(got_stderr, "Missing stderr response");
        assert!(got_done, "Missing done response");
    }

    // NOTE: Ignoring on windows because it's using WSL which wants a Linux path
    //       with / but thinks it's on windows and is providing \
    #[test(tokio::test)]
    #[cfg_attr(windows, ignore)]
    async fn proc_spawn_should_send_done_signal_when_completed() {
        let (api, ctx, mut rx) = setup().await;

        let proc_id = api
            .proc_spawn(
                ctx,
                /* cmd */
                format!("{} {} 0.1", *SCRIPT_RUNNER, SLEEP_SH.to_str().unwrap()),
                /* environment */ Environment::new(),
                /* current_dir */ None,
                /* pty */ None,
            )
            .await
            .unwrap();

        // Wait for process to finish
        match rx.recv().await.unwrap() {
            Response::ProcDone { id, .. } => assert_eq!(
                id, proc_id,
                "Got {}, but expected {} as process id",
                id, proc_id
            ),
            x => panic!("Unexpected response: {:?}", x),
        }
    }

    // NOTE: Ignoring on windows because it's using WSL which wants a Linux path
    //       with / but thinks it's on windows and is providing \
    #[test(tokio::test)]
    #[cfg_attr(windows, ignore)]
    async fn proc_spawn_should_clear_process_from_state_when_killed() {
        let (api, ctx_1, mut rx) = setup().await;
        let (ctx_2, _rx) = {
            let (reply, rx) = make_reply();
            let ctx = DistantCtx {
                connection_id: ctx_1.connection_id,
                reply,
            };
            (ctx, rx)
        };

        let proc_id = api
            .proc_spawn(
                ctx_1,
                /* cmd */
                format!("{} {} 1", *SCRIPT_RUNNER, SLEEP_SH.to_str().unwrap()),
                /* environment */ Environment::new(),
                /* current_dir */ None,
                /* pty */ None,
            )
            .await
            .unwrap();

        // Send kill signal
        api.proc_kill(ctx_2, proc_id).await.unwrap();

        // Wait for the completion response to come in
        match rx.recv().await.unwrap() {
            Response::ProcDone { id, .. } => assert_eq!(
                id, proc_id,
                "Got {}, but expected {} as process id",
                id, proc_id
            ),
            x => panic!("Unexpected response: {:?}", x),
        }
    }

    #[test(tokio::test)]
    async fn proc_kill_should_fail_if_given_non_existent_process() {
        let (api, ctx, _rx) = setup().await;

        // Send kill to a non-existent process
        let _ = api.proc_kill(ctx, 0xDEADBEEF).await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn proc_stdin_should_fail_if_given_non_existent_process() {
        let (api, ctx, _rx) = setup().await;

        // Send stdin to a non-existent process
        let _ = api
            .proc_stdin(ctx, 0xDEADBEEF, b"some input".to_vec())
            .await
            .unwrap_err();
    }

    // NOTE: Ignoring on windows because it's using WSL which wants a Linux path
    //       with / but thinks it's on windows and is providing \
    #[test(tokio::test)]
    #[cfg_attr(windows, ignore)]
    async fn proc_stdin_should_send_stdin_to_process() {
        let (api, ctx_1, mut rx) = setup().await;
        let (ctx_2, _rx) = {
            let (reply, rx) = make_reply();
            let ctx = DistantCtx {
                connection_id: ctx_1.connection_id,
                reply,
            };
            (ctx, rx)
        };

        // First, run a program that listens for stdin
        let id = api
            .proc_spawn(
                ctx_1,
                /* cmd */
                format!(
                    "{} {}",
                    *SCRIPT_RUNNER,
                    ECHO_STDIN_TO_STDOUT_SH.to_str().unwrap()
                ),
                Environment::new(),
                /* current_dir */ None,
                /* pty */ None,
            )
            .await
            .unwrap();

        // Second, send stdin to the remote process
        api.proc_stdin(ctx_2, id, b"hello world\n".to_vec())
            .await
            .unwrap();

        // Third, check the async response of stdout to verify we got stdin
        match rx.recv().await.unwrap() {
            Response::ProcStdout { data, .. } => {
                assert_eq!(data, b"hello world\n", "Mirrored data didn't match");
            }
            x => panic!("Unexpected response: {:?}", x),
        }
    }

    #[test(tokio::test)]
    async fn system_info_should_return_system_info_based_on_binary() {
        let (api, ctx, _rx) = setup().await;

        let system_info = api.system_info(ctx).await.unwrap();
        assert_eq!(
            system_info,
            SystemInfo {
                family: std::env::consts::FAMILY.to_string(),
                os: std::env::consts::OS.to_string(),
                arch: std::env::consts::ARCH.to_string(),
                current_dir: std::env::current_dir().unwrap_or_default(),
                main_separator: std::path::MAIN_SEPARATOR,
                username: whoami::username(),
                shell: if cfg!(windows) {
                    std::env::var("ComSpec").unwrap_or_else(|_| String::from("cmd.exe"))
                } else {
                    std::env::var("SHELL").unwrap_or_else(|_| String::from("/bin/sh"))
                }
            }
        );
    }
}
