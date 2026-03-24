//! NFS mount backend using a localhost NFSv3 server.
//!
//! Starts a local NFSv3 server on a random port and uses OS-native
//! `mount_nfs` to attach it. Primarily targets OpenBSD and NetBSD
//! where FUSE is not available, but works on any Unix platform as
//! a fallback.

use std::io;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use log::debug;
use nfsserve::nfs::{
    fattr3, fileid3, filename3, ftype3, nfspath3, nfsstat3, nfstime3, sattr3, specdata3,
};
use nfsserve::tcp::{NFSTcp, NFSTcpListener};
use nfsserve::vfs::{DirEntry as NfsDirEntry, NFSFileSystem, ReadDirResult, VFSCapabilities};

use crate::RemoteFs;
use crate::cache::FileAttr;
use distant_core::protocol::FileType;

/// NFS filesystem handler that delegates all operations to [`RemoteFs`].
pub(crate) struct NfsHandler {
    fs: Arc<RemoteFs>,
}

impl NfsHandler {
    /// Creates a new handler backed by the given [`RemoteFs`].
    pub(crate) fn new(fs: Arc<RemoteFs>) -> Self {
        Self { fs }
    }
}

/// Converts a crate-level [`FileAttr`] into an [`fattr3`] for NFS responses.
fn to_nfs_attr(attr: &FileAttr) -> fattr3 {
    fattr3 {
        ftype: match attr.kind {
            FileType::Dir => ftype3::NF3DIR,
            FileType::File => ftype3::NF3REG,
            FileType::Symlink => ftype3::NF3LNK,
        },
        mode: attr.perm as u32,
        nlink: attr.nlink,
        uid: attr.uid,
        gid: attr.gid,
        size: attr.size,
        used: attr.blocks * 512,
        rdev: specdata3::default(),
        fsid: 0,
        fileid: attr.ino,
        atime: system_time_to_nfstime(attr.atime),
        mtime: system_time_to_nfstime(attr.mtime),
        ctime: system_time_to_nfstime(attr.ctime),
    }
}

/// Converts a [`std::time::SystemTime`] to an [`nfstime3`].
fn system_time_to_nfstime(time: std::time::SystemTime) -> nfstime3 {
    match time.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => nfstime3 {
            seconds: d.as_secs() as u32,
            nseconds: d.subsec_nanos(),
        },
        Err(_) => nfstime3 {
            seconds: 0,
            nseconds: 0,
        },
    }
}

/// Extracts a UTF-8 string from an NFS filename.
fn filename_to_str(name: &filename3) -> Result<&str, nfsstat3> {
    std::str::from_utf8(name.as_ref()).map_err(|_| nfsstat3::NFS3ERR_INVAL)
}

#[async_trait]
impl NFSFileSystem for NfsHandler {
    fn capabilities(&self) -> VFSCapabilities {
        VFSCapabilities::ReadWrite
    }

    fn root_dir(&self) -> fileid3 {
        1
    }

    async fn lookup(&self, dirid: fileid3, filename: &filename3) -> Result<fileid3, nfsstat3> {
        let name = filename_to_str(filename)?;
        debug!("nfs lookup dirid={} name={:?}", dirid, name);
        match self.fs.lookup(dirid, name) {
            Ok(attr) => Ok(attr.ino),
            Err(_) => Err(nfsstat3::NFS3ERR_NOENT),
        }
    }

    async fn getattr(&self, id: fileid3) -> Result<fattr3, nfsstat3> {
        debug!("nfs getattr id={}", id);
        match self.fs.getattr(id) {
            Ok(attr) => Ok(to_nfs_attr(&attr)),
            Err(_) => Err(nfsstat3::NFS3ERR_NOENT),
        }
    }

    async fn setattr(&self, id: fileid3, _setattr: sattr3) -> Result<fattr3, nfsstat3> {
        debug!("nfs setattr id={}", id);

        // No remote setattr implementation yet; return current attributes.
        match self.fs.getattr(id) {
            Ok(attr) => Ok(to_nfs_attr(&attr)),
            Err(_) => Err(nfsstat3::NFS3ERR_NOENT),
        }
    }

    async fn read(
        &self,
        id: fileid3,
        offset: u64,
        count: u32,
    ) -> Result<(Vec<u8>, bool), nfsstat3> {
        debug!("nfs read id={} offset={} count={}", id, offset, count);
        match self.fs.read(id, offset, count) {
            Ok(data) => {
                let eof = (data.len() as u32) < count;
                Ok((data, eof))
            }
            Err(_) => Err(nfsstat3::NFS3ERR_IO),
        }
    }

    async fn write(&self, id: fileid3, offset: u64, data: &[u8]) -> Result<fattr3, nfsstat3> {
        debug!("nfs write id={} offset={} len={}", id, offset, data.len());
        let _ = self
            .fs
            .write(id, offset, data)
            .map_err(|_| nfsstat3::NFS3ERR_IO)?;
        self.fs.flush(id).map_err(|_| nfsstat3::NFS3ERR_IO)?;
        let attr = self.fs.getattr(id).map_err(|_| nfsstat3::NFS3ERR_IO)?;
        Ok(to_nfs_attr(&attr))
    }

    async fn create(
        &self,
        dirid: fileid3,
        filename: &filename3,
        _setattr: sattr3,
    ) -> Result<(fileid3, fattr3), nfsstat3> {
        let name = filename_to_str(filename)?;
        debug!("nfs create dirid={} name={:?}", dirid, name);
        match self.fs.create(dirid, name, 0o644) {
            Ok(attr) => Ok((attr.ino, to_nfs_attr(&attr))),
            Err(_) => Err(nfsstat3::NFS3ERR_IO),
        }
    }

    async fn create_exclusive(
        &self,
        dirid: fileid3,
        filename: &filename3,
    ) -> Result<fileid3, nfsstat3> {
        let name = filename_to_str(filename)?;
        debug!("nfs create_exclusive dirid={} name={:?}", dirid, name);
        match self.fs.create(dirid, name, 0o644) {
            Ok(attr) => Ok(attr.ino),
            Err(_) => Err(nfsstat3::NFS3ERR_IO),
        }
    }

    async fn remove(&self, dirid: fileid3, filename: &filename3) -> Result<(), nfsstat3> {
        let name = filename_to_str(filename)?;
        debug!("nfs remove dirid={} name={:?}", dirid, name);
        self.fs
            .unlink(dirid, name)
            .map_err(|_| nfsstat3::NFS3ERR_IO)
    }

    async fn rename(
        &self,
        from_dirid: fileid3,
        from_filename: &filename3,
        to_dirid: fileid3,
        to_filename: &filename3,
    ) -> Result<(), nfsstat3> {
        let from_name = filename_to_str(from_filename)?;
        let to_name = filename_to_str(to_filename)?;
        debug!(
            "nfs rename from_dirid={} from={:?} to_dirid={} to={:?}",
            from_dirid, from_name, to_dirid, to_name
        );
        self.fs
            .rename(from_dirid, from_name, to_dirid, to_name)
            .map_err(|_| nfsstat3::NFS3ERR_IO)
    }

    async fn mkdir(
        &self,
        dirid: fileid3,
        dirname: &filename3,
    ) -> Result<(fileid3, fattr3), nfsstat3> {
        let name = filename_to_str(dirname)?;
        debug!("nfs mkdir dirid={} name={:?}", dirid, name);
        match self.fs.mkdir(dirid, name, 0o755) {
            Ok(attr) => Ok((attr.ino, to_nfs_attr(&attr))),
            Err(_) => Err(nfsstat3::NFS3ERR_IO),
        }
    }

    async fn readdir(
        &self,
        dirid: fileid3,
        start_after: fileid3,
        max_entries: usize,
    ) -> Result<ReadDirResult, nfsstat3> {
        debug!(
            "nfs readdir dirid={} start_after={} max_entries={}",
            dirid, start_after, max_entries
        );
        let entries = self.fs.readdir(dirid).map_err(|_| nfsstat3::NFS3ERR_IO)?;

        let nfs_entries: Vec<NfsDirEntry> = entries
            .iter()
            .filter(|e| e.name != "." && e.name != "..")
            .skip_while(|e| start_after > 0 && e.ino <= start_after)
            .take(max_entries)
            .map(|e| NfsDirEntry {
                fileid: e.ino,
                name: e.name.as_bytes().to_vec().into(),
                attr: self
                    .fs
                    .getattr(e.ino)
                    .ok()
                    .map(|a| to_nfs_attr(&a))
                    .unwrap_or_default(),
            })
            .collect();

        let eof = nfs_entries.len() < max_entries || entries.len() <= nfs_entries.len();

        Ok(ReadDirResult {
            entries: nfs_entries,
            end: eof,
        })
    }

    async fn readlink(&self, _id: fileid3) -> Result<nfspath3, nfsstat3> {
        // Symlink reading is not yet supported.
        Err(nfsstat3::NFS3ERR_NOTSUPP)
    }

    async fn symlink(
        &self,
        _dirid: fileid3,
        _linkname: &filename3,
        _symlink: &nfspath3,
        _attr: &sattr3,
    ) -> Result<(fileid3, fattr3), nfsstat3> {
        Err(nfsstat3::NFS3ERR_NOTSUPP)
    }
}

/// Starts a localhost NFSv3 server and returns the listener and the port.
///
/// The server binds to `127.0.0.1` on a randomly chosen free port. The
/// returned [`NFSTcpListener`] must be kept alive (e.g., held in a task)
/// to serve NFS requests.
///
/// # Errors
///
/// Returns an error if binding to a local port fails or the NFS server
/// cannot start.
pub(crate) async fn start_server(
    fs: Arc<RemoteFs>,
) -> io::Result<(NFSTcpListener<NfsHandler>, u16)> {
    let handler = NfsHandler::new(fs);
    let nfs_listener = NFSTcpListener::bind("127.0.0.1:0", handler)
        .await
        .map_err(|e| io::Error::other(format!("failed to start NFS server: {e}")))?;

    let port = nfs_listener.get_listen_port();

    Ok((nfs_listener, port))
}

/// Mounts the NFS server at the given mount point using OS-native mount
/// commands.
///
/// Each supported platform uses its own `mount_nfs` or `mount -t nfs`
/// invocation. The mount connects to `localhost` on the given port using
/// NFSv3 over TCP with file locking disabled.
///
/// # Errors
///
/// Returns an error if the mount point path is not valid UTF-8 or the
/// mount command fails.
pub(crate) fn os_mount(port: u16, mount_point: &Path) -> io::Result<()> {
    let mount_point_str = mount_point
        .to_str()
        .ok_or_else(|| io::Error::other("mount point is not valid UTF-8"))?;

    #[cfg(target_os = "openbsd")]
    let status = std::process::Command::new("mount_nfs")
        .args([
            "-o",
            &format!("port={port},mountport={port}"),
            "-3",
            "-T",
            "localhost:/",
            mount_point_str,
        ])
        .status()?;

    #[cfg(target_os = "netbsd")]
    let status = std::process::Command::new("mount_nfs")
        .args([
            "-o",
            &format!("port={port},mountport={port}"),
            "-3",
            "-T",
            "localhost:/",
            mount_point_str,
        ])
        .status()?;

    #[cfg(target_os = "linux")]
    let status = std::process::Command::new("mount")
        .args([
            "-t",
            "nfs",
            "-o",
            &format!("port={port},mountport={port},nfsvers=3,tcp,nolock"),
            "localhost:/",
            mount_point_str,
        ])
        .status()?;

    #[cfg(target_os = "macos")]
    let status = std::process::Command::new("mount_nfs")
        .args([
            "-o",
            &format!("port={port},mountport={port},nfsvers=3,tcp,nolocks"),
            "localhost:/",
            mount_point_str,
        ])
        .status()?;

    #[cfg(target_os = "freebsd")]
    let status = std::process::Command::new("mount_nfs")
        .args([
            "-o",
            &format!("port={port},mountport={port},nfsv3,tcp,nolockd"),
            "localhost:/",
            mount_point_str,
        ])
        .status()?;

    #[cfg(target_os = "windows")]
    let status = std::process::Command::new("mount")
        .args([
            "-o",
            &format!("port={port},mountport={port},nolock,mtype=hard"),
            "localhost:/",
            mount_point_str,
        ])
        .status()
        .map_err(|e| {
            io::Error::other(format!(
                "NFS mount failed: {e}. Ensure the 'Client for NFS' Windows \
                 feature is enabled (Settings > Apps > Optional Features > \
                 'Client for NFS', or: Enable-WindowsOptionalFeature \
                 -FeatureName ServicesForNFS-ClientOnly -Online)"
            ))
        })?;

    #[cfg(not(any(
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "linux",
        target_os = "macos",
        target_os = "freebsd",
        target_os = "windows",
    )))]
    return Err(io::Error::other(
        "NFS mount is not supported on this platform",
    ));

    if !status.success() {
        return Err(io::Error::other(format!(
            "mount command failed with status {status}"
        )));
    }

    Ok(())
}
