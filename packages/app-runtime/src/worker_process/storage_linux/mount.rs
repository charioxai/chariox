//! Fixed ext4 mounts. There are no generic mount flags, devices or target paths
//! in the client protocol; every action follows a verified backing descriptor.
use super::{
    files,
    loop_device::Device,
    model::{Image, Owner},
    Error, Result,
};
use crate::private_fs::Dir;
use std::{
    ffi::{CString, OsStr},
    fs::File,
    os::{
        fd::AsRawFd,
        unix::fs::{FileExt, MetadataExt},
    },
    path::Path,
};

pub(super) fn superblock(file: &File, expected: &Image) -> Result<()> {
    let mut bytes = [0u8; 1024];
    file.read_exact_at(&mut bytes, 1024)?;
    check_superblock(&bytes, &expected.uuid, expected.capacity)
}
fn check_superblock(bytes: &[u8; 1024], uuid: &str, capacity: u64) -> Result<()> {
    let word = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    let blocks = u64::from(word(4)) | (u64::from(word(0x150)) << 32);
    if bytes[0x38..0x3a] != [0x53, 0xef]
        || word(0x18) != 2
        || blocks.checked_mul(4096) != Some(capacity)
    {
        return Err(Error::Identity);
    }
    let raw = bytes[0x68..0x78]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let observed = format!(
        "{}-{}-{}-{}-{}",
        &raw[..8],
        &raw[8..12],
        &raw[12..16],
        &raw[16..20],
        &raw[20..]
    );
    if observed != uuid {
        return Err(Error::Identity);
    }
    Ok(())
}

pub(super) fn mount(
    parent: &Dir,
    image: &Image,
    backing: &File,
    device: &Device,
    owner: &Owner,
) -> Result<(Dir, u64)> {
    superblock(backing, image)?;
    device.verify()?;
    let underlying = parent.child(OsStr::new(image.role.directory()))?;
    files::require(
        parent,
        image.role.directory(),
        &underlying.0,
        &image.mount_root,
    )?;
    let target = CString::new(format!("/proc/self/fd/{}", underlying.0.as_raw_fd())).unwrap();
    let source = CString::new(device.descriptor_path()).unwrap();
    if unsafe {
        libc::mount(
            source.as_ptr(),
            target.as_ptr(),
            c"ext4".as_ptr(),
            libc::MS_NOEXEC | libc::MS_NODEV | libc::MS_NOSUID,
            c"errors=remount-ro".as_ptr().cast(),
        )
    } != 0
    {
        return Err(Error::Io);
    }
    let mounted = parent.child(OsStr::new(image.role.directory()))?;
    let id = observe(&mounted, image, device, owner, true)?;
    if unsafe { libc::fchmod(mounted.0.as_raw_fd(), 0o700) } != 0 {
        return Err(Error::Io);
    }
    mounted.sync()?;
    Ok((mounted, id))
}

pub(super) fn observe(
    dir: &Dir,
    image: &Image,
    device: &Device,
    owner: &Owner,
    writable: bool,
) -> Result<u64> {
    device.verify()?;
    let metadata = dir.0.metadata()?;
    if metadata.dev() != device.device_id()
        || metadata.uid() != owner.uid
        || metadata.gid() != owner.gid
    {
        return Err(Error::Identity);
    }
    let mut fs = std::mem::MaybeUninit::<libc::statfs>::zeroed();
    if unsafe { libc::fstatfs(dir.0.as_raw_fd(), fs.as_mut_ptr()) } != 0 {
        return Err(Error::Io);
    }
    let fs = unsafe { fs.assume_init() };
    let flags = (libc::ST_NOEXEC | libc::ST_NODEV | libc::ST_NOSUID) as libc::c_long;
    if fs.f_type != 0xef53
        || fs.f_flags & flags != flags
        || (writable && fs.f_flags & libc::ST_RDONLY as libc::c_long != 0)
        || fs.f_blocks.saturating_mul(fs.f_bsize as u64) > image.capacity
    {
        return Err(Error::Identity);
    }
    let mut stat = std::mem::MaybeUninit::<libc::statx>::zeroed();
    if unsafe {
        libc::statx(
            dir.0.as_raw_fd(),
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
    if stat.stx_mask & libc::STATX_MNT_ID == 0
        || stat.stx_mnt_id == 0
        || image
            .mount_id
            .is_some_and(|expected| expected != stat.stx_mnt_id)
    {
        return Err(Error::Identity);
    }
    Ok(stat.stx_mnt_id)
}

pub(super) fn unmount(
    parent: &Dir,
    path: &Path,
    image: &Image,
    device: &Device,
    owner: &Owner,
) -> Result<()> {
    let dir = parent.child(OsStr::new(image.role.directory()))?;
    if files::identity(&dir.0)? == image.mount_root {
        return Ok(());
    }
    observe(&dir, image, device, owner, false)?;
    let named_parent = files::root_directory(path)?;
    if files::identity(&named_parent.0)? != files::identity(&parent.0)? {
        return Err(Error::Identity);
    }
    drop(named_parent);
    if unsafe { libc::syncfs(dir.0.as_raw_fd()) } != 0 {
        return Err(Error::Io);
    }
    // Holding the target directory descriptor during umount would itself make
    // the filesystem busy. Its root-owned parent remains held and non-writable
    // by any client while the derived name is used for the syscall.
    drop(dir);
    let target = CString::new(
        path.join(image.role.directory())
            .as_os_str()
            .as_encoded_bytes(),
    )
    .map_err(|_| Error::Identity)?;
    if unsafe { libc::umount2(target.as_ptr(), 0) } != 0 {
        return Err(Error::RecoveryRequired);
    }
    let underlying = parent.child(OsStr::new(image.role.directory()))?;
    files::require(
        parent,
        image.role.directory(),
        &underlying.0,
        &image.mount_root,
    )?;
    parent.sync()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ext4_identity_is_fixed_to_exact_uuid_and_block_capacity() {
        let mut bytes = [0u8; 1024];
        bytes[0x38..0x3a].copy_from_slice(&[0x53, 0xef]);
        bytes[0x18..0x1c].copy_from_slice(&2u32.to_le_bytes());
        bytes[4..8].copy_from_slice(&16384u32.to_le_bytes());
        let uuid = "00000000-0000-0000-0000-000000000000";
        assert!(check_superblock(&bytes, uuid, 64 * 1024 * 1024).is_ok());
        assert!(check_superblock(&bytes, uuid, 512 * 1024 * 1024).is_err());
        bytes[0x68] = 1;
        assert!(check_superblock(&bytes, uuid, 64 * 1024 * 1024).is_err());
    }
}
