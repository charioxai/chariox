//! Private staging/retirement recovery under the one retained installer lock.
//! The generation lease is the last file removed and stays exclusively held
//! until the parent directory removal is durable.
use super::*;
use crate::private_fs;
use std::collections::BTreeSet;

pub(super) fn seal_children(
    root: &Dir,
    inventory: &super::super::manifest::Inventory,
    uid: u32,
) -> Result<()> {
    let mut directories = BTreeSet::new();
    for file in &inventory.files {
        let mut parent = Path::new(&file.path).parent();
        while let Some(path) = parent.filter(|path| !path.as_os_str().is_empty()) {
            directories.insert(path.to_path_buf());
            parent = path.parent();
        }
    }
    for path in directories.iter().rev() {
        let mut directory = Dir(root.0.try_clone()?);
        for component in path.components() {
            let std::path::Component::Normal(name) = component else {
                return Err(EnrollmentError::Contract);
            };
            directory = files::child(
                &directory,
                name.to_str().ok_or(EnrollmentError::Contract)?,
                uid,
            )?;
        }
        files::mode(&directory.0, 0o555)?;
        directory.sync().map_err(files::fs_error)?;
    }
    Ok(())
}
pub(super) fn recover(
    context: &Context,
    uid: u32,
    target: &str,
    current: Option<&Enrollment>,
    hook: Hook<'_>,
) -> Result<()> {
    files::remove_file(&context.enrollment, PENDING, uid)?;
    context.enrollment.sync().map_err(files::fs_error)?;
    for name in context
        .versions
        .entries(MAX_GENERATIONS + 2)
        .map_err(files::fs_error)?
    {
        let name = name.to_str().ok_or(EnrollmentError::Identity)?;
        if name == STAGING
            || name
                .strip_prefix(RETIRING)
                .is_some_and(|digest| manifest::hex(digest, 64))
        {
            if let Some(digest) = name.strip_prefix(RETIRING) {
                if current.is_some_and(|value| value.inventory_sha256 == digest) {
                    return Err(EnrollmentError::Identity);
                }
            }
            let generation = files::child(&context.versions, name, uid)?;
            let directory = filesystem::Directory::from_file(generation.0.try_clone()?, uid);
            let lease = match directory
                .file(LEASE, if name == STAGING { None } else { Some(0o444) })
            {
                Ok(lease) => match files::exclusive(&lease) {
                    Ok(()) => Some(lease),
                    Err(EnrollmentError::Busy) => continue,
                    Err(error) => return Err(error),
                },
                Err(EnrollmentError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                    if name != STAGING
                        && !generation.entries(1).map_err(files::fs_error)?.is_empty()
                    {
                        return Err(EnrollmentError::Identity);
                    }
                    None
                }
                Err(error) => return Err(error),
            };
            remove_generation(
                &context.versions,
                name,
                generation,
                lease,
                uid,
                target,
                hook,
            )?;
        } else if !manifest::hex(name, 64) {
            return Err(EnrollmentError::Identity);
        }
    }
    Ok(())
}
pub(super) fn remove_generation(
    parent: &Dir,
    name: &str,
    generation: Dir,
    lease: Option<File>,
    uid: u32,
    target: &str,
    hook: Hook<'_>,
) -> Result<()> {
    let allowed: BTreeSet<String> = manifest::expected_paths(target)?
        .into_iter()
        .chain([INVENTORY.into(), SIGNATURE.into(), LEASE.into()])
        .collect();
    let mut remaining = 48;
    remove_payload(&generation, "", &allowed, uid, &mut remaining, hook)?;
    if let Some(lease) = lease.as_ref() {
        let actual =
            private_fs::entry_metadata(&generation, OsStr::new(LEASE)).map_err(files::fs_error)?;
        let held = lease.metadata()?;
        if actual.st_dev as u64 != held.dev() || actual.st_ino as u64 != held.ino() {
            return Err(EnrollmentError::Identity);
        }
        files::remove_file(&generation, LEASE, uid)?;
    }
    generation.sync().map_err(files::fs_error)?;
    remove_directory(parent, name, &generation)?;
    parent.sync().map_err(files::fs_error)?;
    drop(lease);
    Ok(())
}
fn remove_payload(
    dir: &Dir,
    prefix: &str,
    allowed: &BTreeSet<String>,
    uid: u32,
    remaining: &mut usize,
    hook: Hook<'_>,
) -> Result<()> {
    files::mode(&dir.0, 0o700)?;
    for name in dir.entries(*remaining).map_err(files::fs_error)? {
        *remaining = remaining.checked_sub(1).ok_or(EnrollmentError::Limit)?;
        let name_str = name.to_str().ok_or(EnrollmentError::Identity)?;
        if prefix.is_empty() && name_str == LEASE {
            continue;
        }
        let path = format!("{prefix}{name_str}");
        let stat = private_fs::entry_metadata(dir, &name).map_err(files::fs_error)?;
        if stat.st_uid != uid || stat.st_mode & 0o7022 != 0 {
            return Err(EnrollmentError::Identity);
        }
        if stat.st_mode & libc::S_IFMT == libc::S_IFDIR {
            let next = format!("{path}/");
            if !allowed.iter().any(|entry| entry.starts_with(&next)) {
                return Err(EnrollmentError::Identity);
            }
            let child = files::child(dir, name_str, uid)?;
            remove_payload(&child, &next, allowed, uid, remaining, hook)?;
            remove_directory(dir, name_str, &child)?;
        } else {
            if stat.st_mode & libc::S_IFMT != libc::S_IFREG
                || stat.st_nlink != 1
                || !allowed.contains(&path)
            {
                return Err(EnrollmentError::Identity);
            }
            files::remove_file(dir, name_str, uid)?;
            dir.sync().map_err(files::fs_error)?;
            hook(Checkpoint::RetiringPayload)?;
        }
    }
    dir.sync().map_err(files::fs_error)?;
    Ok(())
}
fn remove_directory(parent: &Dir, name: &str, held: &Dir) -> Result<()> {
    if !private_fs::same_entry(parent, OsStr::new(name), held).map_err(files::fs_error)? {
        return Err(EnrollmentError::Identity);
    }
    let name = private_fs::cstring(OsStr::new(name)).map_err(files::fs_error)?;
    private_fs::check(unsafe {
        libc::unlinkat(parent.0.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR)
    })
    .map_err(files::fs_error)
}
