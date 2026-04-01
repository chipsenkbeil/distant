//! FUSE backend for mounting remote filesystems via `fuser`.
//!
//! Implements the [`fuser::Filesystem`] trait by dispatching each synchronous
//! FUSE callback into the async [`Runtime`], which bridges to [`RemoteFs`].

use std::ffi::OsStr;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use log::warn;

use fuser::{
    BsdFileFlags, Errno, FileAttr as FuserFileAttr, FileHandle, FileType as FuserFileType,
    Filesystem, FopenFlags, Generation, INodeNo, KernelConfig, LockOwner, OpenFlags, RenameFlags,
    ReplyAttr, ReplyCreate, ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen,
    ReplyWrite, Request, SessionACL, TimeOrNow, WriteFlags,
};

use crate::core::{FileAttr, Runtime};
use distant_core::protocol::FileType;

/// TTL used for FUSE entry and attribute replies.
const TTL: Duration = Duration::from_secs(1);

/// FUSE filesystem handler that dispatches all operations to [`RemoteFs`]
/// via the async [`Runtime`].
pub(crate) struct FuseHandler {
    rt: Arc<Runtime>,
}

/// Converts a crate-level [`FileAttr`] into a [`fuser::FileAttr`].
///
/// Overrides `uid` and `gid` with the current process's values so the
/// mount point is accessible to the user who ran the mount command.
/// Without this, macFUSE shows root-owned files that are inaccessible
/// to the unprivileged user.
fn to_fuser_attr(attr: &FileAttr) -> FuserFileAttr {
    FuserFileAttr {
        ino: INodeNo(attr.ino),
        size: attr.size,
        blocks: attr.blocks,
        atime: attr.atime,
        mtime: attr.mtime,
        ctime: attr.ctime,
        crtime: attr.ctime,
        kind: to_fuser_file_type(attr.kind),
        perm: attr.perm,
        nlink: attr.nlink,
        uid: unsafe { libc::getuid() },
        gid: unsafe { libc::getgid() },
        rdev: 0,
        blksize: 512,
        flags: 0,
    }
}

/// Converts a distant [`FileType`] into a [`fuser::FileType`].
fn to_fuser_file_type(ft: FileType) -> FuserFileType {
    match ft {
        FileType::Dir => FuserFileType::Directory,
        FileType::File => FuserFileType::RegularFile,
        FileType::Symlink => FuserFileType::Symlink,
    }
}

/// Maps an [`io::Error`] to a fuser [`Errno`] for FUSE error replies.
///
/// Logs the original error at `warn` level before mapping so that
/// connection-level failures and unexpected error kinds are visible in logs
/// without requiring the caller to log separately.
fn io_error_to_errno(err: &io::Error) -> Errno {
    warn!("FUSE error: {err}");

    match err.kind() {
        io::ErrorKind::NotFound => Errno::ENOENT,
        io::ErrorKind::PermissionDenied => Errno::EACCES,
        io::ErrorKind::AlreadyExists => Errno::EEXIST,
        io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => Errno::EINVAL,
        io::ErrorKind::Unsupported => Errno::ENOSYS,
        io::ErrorKind::TimedOut => Errno::ETIMEDOUT,
        io::ErrorKind::ConnectionRefused => Errno::ECONNREFUSED,
        io::ErrorKind::ConnectionReset => Errno::ECONNRESET,
        io::ErrorKind::ConnectionAborted => Errno::ECONNABORTED,
        io::ErrorKind::BrokenPipe => Errno::EPIPE,
        io::ErrorKind::WouldBlock => Errno::EAGAIN,
        io::ErrorKind::DirectoryNotEmpty => Errno::ENOTEMPTY,
        io::ErrorKind::IsADirectory => Errno::EISDIR,
        _ => Errno::EIO,
    }
}

impl Filesystem for FuseHandler {
    fn init(&mut self, _req: &Request, _config: &mut KernelConfig) -> io::Result<()> {
        Ok(())
    }

    fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let name = name.to_str().unwrap_or("").to_string();
        self.rt.spawn(move |fs| async move {
            match fs.lookup(parent.0, &name).await {
                Ok(attr) => reply.entry(&TTL, &to_fuser_attr(&attr), Generation(0)),
                Err(e) => reply.error(io_error_to_errno(&e)),
            }
        });
    }

    fn getattr(&self, _req: &Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {
        self.rt.spawn(move |fs| async move {
            match fs.getattr(ino.0).await {
                Ok(attr) => reply.attr(&TTL, &to_fuser_attr(&attr)),
                Err(e) => reply.error(io_error_to_errno(&e)),
            }
        });
    }

    fn read(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyData,
    ) {
        self.rt.spawn(move |fs| async move {
            match fs.read(ino.0, offset, size).await {
                Ok(data) => reply.data(&data),
                Err(e) => reply.error(io_error_to_errno(&e)),
            }
        });
    }

    fn write(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        data: &[u8],
        _write_flags: WriteFlags,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyWrite,
    ) {
        let data = data.to_vec();
        self.rt.spawn(move |fs| async move {
            match fs.write(ino.0, offset, &data).await {
                Ok(written) => reply.written(written),
                Err(e) => reply.error(io_error_to_errno(&e)),
            }
        });
    }

    fn readdir(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        self.rt.spawn(move |fs| async move {
            match fs.readdir(ino.0).await {
                Ok(entries) => {
                    for (i, entry) in entries.iter().enumerate().skip(offset as usize) {
                        let full = reply.add(
                            INodeNo(entry.ino),
                            (i + 1) as u64,
                            to_fuser_file_type(entry.file_type),
                            &entry.name,
                        );
                        if full {
                            break;
                        }
                    }
                    reply.ok();
                }
                Err(e) => reply.error(io_error_to_errno(&e)),
            }
        });
    }

    fn create(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        _umask: u32,
        _flags: i32,
        reply: ReplyCreate,
    ) {
        let name = name.to_str().unwrap_or("").to_string();
        self.rt.spawn(move |fs| async move {
            match fs.create(parent.0, &name, mode).await {
                Ok(attr) => reply.created(
                    &TTL,
                    &to_fuser_attr(&attr),
                    Generation(0),
                    FileHandle(0),
                    FopenFlags::empty(),
                ),
                Err(e) => reply.error(io_error_to_errno(&e)),
            }
        });
    }

    fn mkdir(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        let name = name.to_str().unwrap_or("").to_string();
        self.rt.spawn(move |fs| async move {
            match fs.mkdir(parent.0, &name, mode).await {
                Ok(attr) => reply.entry(&TTL, &to_fuser_attr(&attr), Generation(0)),
                Err(e) => reply.error(io_error_to_errno(&e)),
            }
        });
    }

    fn unlink(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let name = name.to_str().unwrap_or("").to_string();
        self.rt.spawn(move |fs| async move {
            match fs.unlink(parent.0, &name).await {
                Ok(()) => reply.ok(),
                Err(e) => reply.error(io_error_to_errno(&e)),
            }
        });
    }

    fn rmdir(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let name = name.to_str().unwrap_or("").to_string();
        self.rt.spawn(move |fs| async move {
            match fs.rmdir(parent.0, &name).await {
                Ok(()) => reply.ok(),
                Err(e) => reply.error(io_error_to_errno(&e)),
            }
        });
    }

    fn rename(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        newparent: INodeNo,
        newname: &OsStr,
        _flags: RenameFlags,
        reply: ReplyEmpty,
    ) {
        let name = name.to_str().unwrap_or("").to_string();
        let newname = newname.to_str().unwrap_or("").to_string();
        self.rt.spawn(move |fs| async move {
            match fs.rename(parent.0, &name, newparent.0, &newname).await {
                Ok(()) => reply.ok(),
                Err(e) => reply.error(io_error_to_errno(&e)),
            }
        });
    }

    fn flush(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        _lock_owner: LockOwner,
        reply: ReplyEmpty,
    ) {
        self.rt.spawn(move |fs| async move {
            match fs.flush(ino.0).await {
                Ok(()) => reply.ok(),
                Err(e) => reply.error(io_error_to_errno(&e)),
            }
        });
    }

    fn fsync(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        _datasync: bool,
        reply: ReplyEmpty,
    ) {
        self.rt.spawn(move |fs| async move {
            match fs.fsync(ino.0).await {
                Ok(()) => reply.ok(),
                Err(e) => reply.error(io_error_to_errno(&e)),
            }
        });
    }

    fn release(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        self.rt.spawn(move |fs| async move {
            match fs.release(ino.0).await {
                Ok(()) => reply.ok(),
                Err(e) => reply.error(io_error_to_errno(&e)),
            }
        });
    }

    fn forget(&self, _req: &Request, ino: INodeNo, nlookup: u64) {
        self.rt.spawn(move |fs| async move {
            fs.forget(ino.0, nlookup).await;
        });
    }

    fn open(&self, _req: &Request, _ino: INodeNo, _flags: OpenFlags, reply: ReplyOpen) {
        reply.opened(FileHandle(0), FopenFlags::empty());
    }

    fn opendir(&self, _req: &Request, _ino: INodeNo, _flags: OpenFlags, reply: ReplyOpen) {
        reply.opened(FileHandle(0), FopenFlags::empty());
    }

    fn setattr(
        &self,
        _req: &Request,
        ino: INodeNo,
        _mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        _size: Option<u64>,
        _atime: Option<TimeOrNow>,
        _mtime: Option<TimeOrNow>,
        _ctime: Option<std::time::SystemTime>,
        _fh: Option<FileHandle>,
        _crtime: Option<std::time::SystemTime>,
        _chgtime: Option<std::time::SystemTime>,
        _bkuptime: Option<std::time::SystemTime>,
        _flags: Option<BsdFileFlags>,
        reply: ReplyAttr,
    ) {
        self.rt.spawn(move |fs| async move {
            match fs.getattr(ino.0).await {
                Ok(attr) => reply.attr(&TTL, &to_fuser_attr(&attr)),
                Err(e) => reply.error(io_error_to_errno(&e)),
            }
        });
    }
}

/// Mounts the given [`Runtime`] at `mount_point` using FUSE.
///
/// Returns a [`fuser::BackgroundSession`] that keeps the mount alive until
/// dropped.
pub(crate) fn mount(
    rt: Arc<Runtime>,
    mount_point: &std::path::Path,
    readonly: bool,
) -> io::Result<fuser::BackgroundSession> {
    let handler = FuseHandler { rt };
    let mut config = fuser::Config::default();
    config.mount_options = vec![
        fuser::MountOption::FSName("distant".to_string()),
        fuser::MountOption::AutoUnmount,
    ];
    if readonly {
        config.mount_options.push(fuser::MountOption::RO);
    }
    config.acl = SessionACL::All;
    fuser::spawn_mount2(handler, mount_point, &config)
        .map_err(|e| io::Error::other(format!("FUSE mount failed: {e}")))
}
