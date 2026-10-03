//! Fixed nonrecursive bind views. The root helper owns the namespace parents;
//! each source and underlying mountpoint is pinned before any mount syscall.
use super::{
    code_model::{Kind, View},
    files, Error, Result,
};
use crate::private_fs::Dir;
use std::{
    ffi::{CString, OsStr},
    fs::File,
    os::fd::{AsRawFd, FromRawFd},
    path::Path,
};

pub(super) fn mount(parent: &Dir, path: &Path, view: &mut View, source: &File) -> Result<()> {
    if files::identity(source)? != view.source {
        return Err(Error::Identity);
    }
    let underlying = parent.child(OsStr::new(view.kind.name()))?;
    if files::identity(&underlying.0)? != view.underlying {
        return Err(Error::Identity);
    }
    // Seal a detached nonrecursive clone before it is attached/propagated.
    // A later bind-remount changes only that mount object and is insufficient
    // to establish flags on a clone already propagated to the kernel namespace.
    let fd = unsafe {
        libc::syscall(
            libc::SYS_open_tree,
            source.as_raw_fd(),
            c"".as_ptr(),
            libc::OPEN_TREE_CLONE | libc::OPEN_TREE_CLOEXEC | libc::AT_EMPTY_PATH as u32,
        )
    };
    if fd < 0 {
        return Err(Error::Io);
    }
    let detached = unsafe { File::from_raw_fd(fd as i32) };
    let attributes = libc::mount_attr {
        attr_set: libc::MOUNT_ATTR_RDONLY
            | libc::MOUNT_ATTR_NODEV
            | libc::MOUNT_ATTR_NOSUID
            | if view.kind == Kind::Package {
                libc::MOUNT_ATTR_NOEXEC
            } else {
                0
            },
        attr_clr: if view.kind == Kind::Runtime {
            libc::MOUNT_ATTR_NOEXEC
        } else {
            0
        },
        propagation: libc::MS_PRIVATE as u64,
        userns_fd: 0,
    };
    if unsafe {
        libc::syscall(
            libc::SYS_mount_setattr,
            detached.as_raw_fd(),
            c"".as_ptr(),
            libc::AT_EMPTY_PATH as u32,
            &attributes,
            std::mem::size_of::<libc::mount_attr>(),
        )
    } != 0
    {
        return Err(Error::Io);
    }
    // The root-controlled pathname still names our selected mountpoint. The
    // syscall itself uses held descriptors for both detached source and target.
    owned_target(parent, path, view.kind)?;
    if unsafe {
        libc::syscall(
            libc::SYS_move_mount,
            detached.as_raw_fd(),
            c"".as_ptr(),
            underlying.0.as_raw_fd(),
            c"".as_ptr(),
            libc::MOVE_MOUNT_F_EMPTY_PATH | libc::MOVE_MOUNT_T_EMPTY_PATH,
        )
    } != 0
    {
        return Err(Error::Io);
    }
    let mounted = parent.child(OsStr::new(view.kind.name()))?;
    let id = observe(&mounted, view.kind, &view.source)?;
    view.mount_id = Some(id);
    view.sealed = true;
    Ok(())
}

pub(super) fn observe(dir: &Dir, kind: Kind, source: &super::model::Identity) -> Result<u64> {
    if files::identity(&dir.0)? != *source {
        return Err(Error::Identity);
    }
    let flags = files::mount_flags(&dir.0)?;
    let required = libc::ST_RDONLY | libc::ST_NODEV | libc::ST_NOSUID;
    if flags & required != required || (flags & libc::ST_NOEXEC != 0) != (kind == Kind::Package) {
        return Err(Error::Identity);
    }
    mount_id(&dir.0)
}

pub(super) fn unmount(parent: &Dir, path: &Path, view: &View) -> Result<()> {
    let mounted = parent.child(OsStr::new(view.kind.name()))?;
    let identity = files::identity(&mounted.0)?;
    if identity == view.underlying {
        return Ok(());
    }
    if identity != view.source
        || view
            .mount_id
            .is_some_and(|expected| mount_id(&mounted.0).ok() != Some(expected))
    {
        return Err(Error::Identity);
    }
    // A crash after attaching the sealed tree but before journaling its mount ID
    // is recoverable by the exact source inode. A recorded seal also requires
    // its recorded mount ID and flags before ordinary unmount.
    if view.sealed {
        observe(&mounted, view.kind, &view.source)?;
    }
    let target = owned_target(parent, path, view.kind)?;
    drop(mounted);
    if unsafe { libc::umount2(target.as_ptr(), 0) } != 0 {
        return Err(Error::RecoveryRequired);
    }
    let underlying = parent.child(OsStr::new(view.kind.name()))?;
    if files::identity(&underlying.0)? != view.underlying {
        return Err(Error::Identity);
    }
    parent.sync()?;
    Ok(())
}

fn owned_target(parent: &Dir, path: &Path, kind: Kind) -> Result<CString> {
    let named = files::root_directory(path)?;
    if files::identity(&named.0)? != files::identity(&parent.0)? {
        return Err(Error::Identity);
    }
    CString::new(path.join(kind.name()).as_os_str().as_encoded_bytes()).map_err(|_| Error::Identity)
}
fn mount_id(file: &File) -> Result<u64> {
    let mut stat = std::mem::MaybeUninit::<libc::statx>::zeroed();
    if unsafe {
        libc::statx(
            file.as_raw_fd(),
            c"".as_ptr(),
            libc::AT_EMPTY_PATH | libc::AT_SYMLINK_NOFOLLOW,
            libc::STATX_MNT_ID,
            stat.as_mut_ptr(),
        )
    } != 0
    {
        return Err(Error::Io);
    }
    let stat = unsafe { stat.assume_init() };
    if stat.stx_mask & libc::STATX_MNT_ID == 0 || stat.stx_mnt_id == 0 {
        return Err(Error::Identity);
    }
    Ok(stat.stx_mnt_id)
}
