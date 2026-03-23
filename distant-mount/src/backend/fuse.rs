//! FUSE backend for mounting remote filesystems via `fuser`.
//!
//! Implements the [`fuser::Filesystem`] trait by delegating all callbacks to
//! [`RemoteFs`], which bridges synchronous FUSE operations to the async distant
//! protocol.

use std::ffi::OsStr;
use std::io;
use std::os::raw::c_int;
use std::sync::Arc;
use std::time::Duration;

use fuser::{
    FileAttr as FuserFileAttr, FileType as FuserFileType, Filesystem, KernelConfig, ReplyAttr,
    ReplyCreate, ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, ReplyWrite, Request,
    TimeOrNow,
};

use crate::RemoteFs;
use crate::cache::FileAttr;
use distant_core::protocol::FileType;

/// TTL used for FUSE entry and attribute replies.
const TTL: Duration = Duration::from_secs(1);

/// FUSE filesystem handler that delegates all operations to [`RemoteFs`].
pub(crate) struct FuseHandler {
    fs: Arc<RemoteFs>,
}

/// Converts a crate-level [`FileAttr`] into a [`fuser::FileAttr`].
fn to_fuser_attr(attr: &FileAttr) -> FuserFileAttr {
    FuserFileAttr {
        ino: attr.ino,
        size: attr.size,
        blocks: attr.blocks,
        atime: attr.atime,
        mtime: attr.mtime,
        ctime: attr.ctime,
        crtime: attr.ctime,
        kind: to_fuser_file_type(attr.kind),
        perm: attr.perm,
        nlink: attr.nlink,
        uid: attr.uid,
        gid: attr.gid,
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

/// Maps an [`io::Error`] to a libc errno code for FUSE error replies.
fn io_error_to_errno(err: &io::Error) -> i32 {
    match err.kind() {
        io::ErrorKind::NotFound => libc::ENOENT,
        io::ErrorKind::PermissionDenied => libc::EACCES,
        io::ErrorKind::AlreadyExists => libc::EEXIST,
        io::ErrorKind::InvalidInput => libc::EINVAL,
        io::ErrorKind::Unsupported => libc::ENOSYS,
        _ => libc::EIO,
    }
}

impl Filesystem for FuseHandler {
    fn init(&mut self, _req: &Request<'_>, _config: &mut KernelConfig) -> Result<(), c_int> {
        Ok(())
    }

    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let name_str = name.to_str().unwrap_or("");
        match self.fs.lookup(parent, name_str) {
            Ok(attr) => reply.entry(&TTL, &to_fuser_attr(&attr), 0),
            Err(err) => reply.error(io_error_to_errno(&err)),
        }
    }

    fn getattr(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyAttr) {
        match self.fs.getattr(ino) {
            Ok(attr) => reply.attr(&TTL, &to_fuser_attr(&attr)),
            Err(err) => reply.error(io_error_to_errno(&err)),
        }
    }

    fn read(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock: Option<u64>,
        reply: ReplyData,
    ) {
        match self.fs.read(ino, offset as u64, size) {
            Ok(data) => reply.data(&data),
            Err(err) => reply.error(io_error_to_errno(&err)),
        }
    }

    fn write(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        match self.fs.write(ino, offset as u64, data) {
            Ok(written) => reply.written(written),
            Err(err) => reply.error(io_error_to_errno(&err)),
        }
    }

    fn readdir(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        match self.fs.readdir(ino) {
            Ok(entries) => {
                for (i, entry) in entries.iter().enumerate().skip(offset as usize) {
                    // reply.add returns true when the buffer is full.
                    let full = reply.add(
                        entry.ino,
                        (i + 1) as i64,
                        to_fuser_file_type(entry.file_type),
                        &entry.name,
                    );
                    if full {
                        break;
                    }
                }
                reply.ok();
            }
            Err(err) => reply.error(io_error_to_errno(&err)),
        }
    }

    fn create(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        _umask: u32,
        _flags: i32,
        reply: ReplyCreate,
    ) {
        let name_str = name.to_str().unwrap_or("");
        match self.fs.create(parent, name_str, mode) {
            Ok(attr) => reply.created(&TTL, &to_fuser_attr(&attr), 0, 0, 0),
            Err(err) => reply.error(io_error_to_errno(&err)),
        }
    }

    fn mkdir(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        let name_str = name.to_str().unwrap_or("");
        match self.fs.mkdir(parent, name_str, mode) {
            Ok(attr) => reply.entry(&TTL, &to_fuser_attr(&attr), 0),
            Err(err) => reply.error(io_error_to_errno(&err)),
        }
    }

    fn unlink(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        let name_str = name.to_str().unwrap_or("");
        match self.fs.unlink(parent, name_str) {
            Ok(()) => reply.ok(),
            Err(err) => reply.error(io_error_to_errno(&err)),
        }
    }

    fn rmdir(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        let name_str = name.to_str().unwrap_or("");
        match self.fs.rmdir(parent, name_str) {
            Ok(()) => reply.ok(),
            Err(err) => reply.error(io_error_to_errno(&err)),
        }
    }

    fn rename(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        newparent: u64,
        newname: &OsStr,
        _flags: u32,
        reply: ReplyEmpty,
    ) {
        let name_str = name.to_str().unwrap_or("");
        let newname_str = newname.to_str().unwrap_or("");
        match self.fs.rename(parent, name_str, newparent, newname_str) {
            Ok(()) => reply.ok(),
            Err(err) => reply.error(io_error_to_errno(&err)),
        }
    }

    fn flush(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        _lock_owner: u64,
        reply: ReplyEmpty,
    ) {
        match self.fs.flush(ino) {
            Ok(()) => reply.ok(),
            Err(err) => reply.error(io_error_to_errno(&err)),
        }
    }

    fn fsync(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        _datasync: bool,
        reply: ReplyEmpty,
    ) {
        match self.fs.fsync(ino) {
            Ok(()) => reply.ok(),
            Err(err) => reply.error(io_error_to_errno(&err)),
        }
    }

    fn release(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        match self.fs.release(ino) {
            Ok(()) => reply.ok(),
            Err(err) => reply.error(io_error_to_errno(&err)),
        }
    }

    fn forget(&mut self, _req: &Request<'_>, ino: u64, nlookup: u64) {
        self.fs.forget(ino, nlookup);
    }

    fn open(&mut self, _req: &Request<'_>, _ino: u64, _flags: i32, reply: ReplyOpen) {
        reply.opened(0, 0);
    }

    fn opendir(&mut self, _req: &Request<'_>, _ino: u64, _flags: i32, reply: ReplyOpen) {
        reply.opened(0, 0);
    }

    fn setattr(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        _size: Option<u64>,
        _atime: Option<TimeOrNow>,
        _mtime: Option<TimeOrNow>,
        _ctime: Option<std::time::SystemTime>,
        _fh: Option<u64>,
        _crtime: Option<std::time::SystemTime>,
        _chgtime: Option<std::time::SystemTime>,
        _bkuptime: Option<std::time::SystemTime>,
        _flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        // No remote setattr implementation yet; return current attributes.
        match self.fs.getattr(ino) {
            Ok(attr) => reply.attr(&TTL, &to_fuser_attr(&attr)),
            Err(err) => reply.error(io_error_to_errno(&err)),
        }
    }
}

/// Mounts the given [`RemoteFs`] at `mount_point` using FUSE.
///
/// Returns a [`fuser::BackgroundSession`] that keeps the mount alive until
/// dropped.
///
/// # Errors
///
/// Returns an error if the FUSE mount fails (e.g., missing permissions or
/// the mount point does not exist).
pub(crate) fn mount(
    fs: Arc<RemoteFs>,
    mount_point: &std::path::Path,
) -> io::Result<fuser::BackgroundSession> {
    let handler = FuseHandler { fs };
    let options = vec![
        fuser::MountOption::FSName("distant".to_string()),
        fuser::MountOption::AutoUnmount,
        fuser::MountOption::AllowOther,
    ];
    fuser::spawn_mount2(handler, mount_point, &options)
        .map_err(|e| io::Error::other(format!("FUSE mount failed: {e}")))
}
