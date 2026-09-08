use super::{
    commands::{self, Tool},
    identity::{self, FileIdentity},
    journal, private_mount, Error, MountedStorage, Result,
};
use crate::private_fs::{Dir, FsError};
use std::{
    ffi::OsStr,
    fs::File,
    os::{fd::AsRawFd, unix::fs::MetadataExt},
};

impl MountedStorage {
    fn require_root(&self) -> Result<()> {
        let current = Dir::open_private(&self.path)?;
        if FileIdentity::of(&current.0)? != FileIdentity::of(&self.root.0)? {
            return Err(Error::Identity);
        }
        Ok(())
    }

    fn open_image(&self, index: usize) -> Result<Option<File>> {
        self.require_root()?;
        let image = &self.journal.images[index];
        let file = match self.root.read_file(OsStr::new(&image.image), false) {
            Ok(file) => file,
            Err(FsError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                if image.volume_uuid.is_some() && image.role == "data" {
                    return Err(Error::Identity);
                }
                return Ok(None);
            }
            Err(e) => return Err(e.into()),
        };
        if let Some(expected) = &image.identity {
            expected.require(&self.root, OsStr::new(&image.image), &file)?;
        }
        let metadata = file.metadata()?;
        if metadata.len() > image.reserved() {
            return Err(Error::Identity);
        }
        if image.identity.is_some() && metadata.mode() & 0o7777 != 0o600 {
            return Err(Error::Identity);
        }
        Ok(Some(file))
    }

    pub(super) fn prepare_volumes(&mut self) -> Result<()> {
        self.deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        // Temporary storage is discarded only after exact-image detachment.
        // Data and its durable identity are always retained across generations.
        if self.journal.images[1].identity.is_some() {
            if let Some(file) = self.open_image(1)? {
                self.journal.images[1].identity.as_ref().unwrap().require(
                    &self.root,
                    OsStr::new(&self.journal.images[1].image),
                    &file,
                )?;
                self.images[1] = None;
                self.root
                    .remove_file(OsStr::new(&self.journal.images[1].image))?;
                self.root.sync()?;
            }
            let prior = &self.journal.images[1];
            self.journal.images[1] =
                journal::image("tmp", prior.capacity, prior.mount_identity.clone());
            self.journal.save(&self.root)?;
        }
        for index in 0..2 {
            self.prepare_image(index)?;
            let image = &self.journal.images[index];
            let image_path = self.path.join(&image.image);
            let arguments = commands::strings(&[
                "attach",
                image_path.to_str().ok_or(Error::Identity)?,
                "-nomount",
                "-plist",
                "-owners",
                "on",
                "-nobrowse",
                "-noautoopen",
            ]);
            self.require_root()?;
            commands::plist(Tool::Hdiutil, &arguments, self.command_context())?;
            self.open_image(index)?.ok_or(Error::Identity)?;
            let attachment = identity::discover(
                &image_path,
                &image.volume_name,
                image.volume_uuid.as_deref(),
                self.command_context(),
            )?
            .ok_or(Error::Identity)?;
            self.journal.images[index].volume_uuid = Some(attachment.uuid.clone());
            self.journal.save(&self.root)?;
            let mount = self.path.join(&self.journal.images[index].role);
            let underlying = private_mount(&self.root, &self.journal.images[index].role)?;
            self.journal.images[index].mount_identity.require(
                &self.root,
                OsStr::new(&self.journal.images[index].role),
                &underlying.0,
            )?;
            // No flags, device names, volume names or mount paths come from an App.
            commands::run(
                Tool::Diskutil,
                &commands::strings(&[
                    "mount",
                    "nobrowse",
                    "-mountOptions",
                    "noexec,nodev,nosuid,owners",
                    "-mountPoint",
                    mount.to_str().ok_or(Error::Identity)?,
                    &attachment.volume,
                ]),
                &[],
                self.command_context(),
            )?;
            self.require_root()?;
            let dir = self
                .root
                .child(OsStr::new(&self.journal.images[index].role))?;
            identity::mounted(&dir, &mount, &attachment)?;
            if identity::discover(
                &image_path,
                &self.journal.images[index].volume_name,
                Some(&attachment.uuid),
                self.command_context(),
            )? != Some(attachment.clone())
            {
                return Err(Error::Identity);
            }
            #[cfg(test)]
            let metadata = dir.0.metadata()?;
            #[cfg(test)]
            eprintln!(
                "storage_root_observation role={} uid={} expected_uid={} mode={:o}",
                self.journal.images[index].role,
                metadata.uid(),
                unsafe { libc::geteuid() },
                metadata.mode() & 0o7777,
            );
            private_volume_root(&dir.0)?;
            let mut stat = std::mem::MaybeUninit::<libc::statfs>::zeroed();
            if unsafe { libc::fstatfs(dir.0.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
                return Err(Error::Io);
            }
            let stat = unsafe { stat.assume_init() };
            if stat.f_blocks.saturating_mul(stat.f_bsize as u64)
                > self.journal.images[index].capacity
            {
                return Err(Error::Capacity);
            }
            self.mounted[index] = Some(dir);
        }
        self.journal.save(&self.root)?;
        Ok(())
    }

    fn prepare_image(&mut self, index: usize) -> Result<()> {
        let image = &self.journal.images[index];
        if image.identity.is_none() {
            // A crash during first creation may leave an incomplete image. No
            // App can have run before a volume UUID was committed. The private,
            // random planned filename is owned by this durable creation intent.
            if let Some(file) = self.open_image(index)? {
                if identity::discover(
                    &self.path.join(&image.image),
                    &image.volume_name,
                    None,
                    self.command_context(),
                )?
                .is_some()
                {
                    return Err(Error::RecoveryRequired);
                }
                let held = FileIdentity::of(&file)?;
                held.require(&self.root, OsStr::new(&image.image), &file)?;
                self.root.remove_file(OsStr::new(&image.image))?;
                self.root.sync()?;
            }
            commands::run(
                Tool::Hdiutil,
                &create_arguments(
                    &self.path.join(&image.image),
                    &image.volume_name,
                    image.capacity,
                )?,
                &[],
                self.command_context(),
            )?;
        }
        let file = self.open_image(index)?.ok_or(Error::Identity)?;
        if file.metadata()?.len() < image.capacity {
            return Err(Error::Identity);
        }
        if unsafe { libc::fchmod(file.as_raw_fd(), 0o600) } != 0 {
            return Err(Error::Io);
        }
        file.sync_all()?;
        self.root.sync()?;
        let format = commands::run(
            Tool::Hdiutil,
            &commands::strings(&[
                "imageinfo",
                self.path
                    .join(&image.image)
                    .to_str()
                    .ok_or(Error::Identity)?,
                "-format",
            ]),
            &[],
            self.command_context(),
        )?;
        if format != b"UDRW\n" && format != b"UDRW" {
            return Err(Error::Identity);
        }
        self.journal.images[index].identity = Some(FileIdentity::of(&file)?);
        self.images[index] = Some(file);
        self.journal.save(&self.root)?;
        Ok(())
    }

    pub fn release_blocking(&mut self) -> Result<()> {
        if self.released {
            return Ok(());
        }
        // An explicit failed release leaves the journal for recovery. Drop must
        // not silently add another full cleanup deadline to that failed call.
        self.cleanup_attempted = true;
        self.deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        self.require_root()?;
        self.journal.pending_recovery = true;
        self.journal.save(&self.root)?;
        // No App may still own this filesystem: caller has reaped its worker.
        // Our own mounted-directory descriptors must also close before detach.
        self.mounted = [None, None];
        for index in (0..2).rev() {
            let image = &self.journal.images[index];
            if let Some(file) = self.open_image(index)? {
                let path = self.path.join(&image.image);
                if let Some(attached) = identity::discover(
                    &path,
                    &image.volume_name,
                    image.volume_uuid.as_deref(),
                    self.command_context(),
                )? {
                    // Recheck backing inode immediately before this device action.
                    FileIdentity::of(&file)?.require(
                        &self.root,
                        OsStr::new(&image.image),
                        &file,
                    )?;
                    commands::run(
                        Tool::Hdiutil,
                        &commands::strings(&["detach", &attached.whole]),
                        &[],
                        self.command_context(),
                    )?;
                    if identity::discover(
                        &path,
                        &image.volume_name,
                        image.volume_uuid.as_deref(),
                        self.command_context(),
                    )?
                    .is_some()
                    {
                        return Err(Error::RecoveryRequired);
                    }
                }
                file.sync_all()?;
            }
            let mount = private_mount(&self.root, &image.role)?;
            image
                .mount_identity
                .require(&self.root, OsStr::new(&image.role), &mount.0)?;
        }
        self.root.sync()?;
        self.journal.pending_recovery = false;
        self.journal.save(&self.root)?;
        self.released = true;
        Ok(())
    }
}

/// APFS creation currently returns an owner-matching 0755 volume root even
/// with hdiutil -mode0700. Set privacy on the verified mounted descriptor before
/// any worker can receive it; never chmod an unexpected owner's filesystem.
pub(super) fn private_volume_root(root: &File) -> Result<()> {
    let metadata = root.metadata()?;
    if !metadata.is_dir() || metadata.uid() != unsafe { libc::geteuid() } {
        return Err(Error::Identity);
    }
    if unsafe { libc::fchmod(root.as_raw_fd(), 0o700) } != 0 {
        return Err(Error::Io);
    }
    root.sync_all()?;
    if root.metadata()?.mode() & 0o7777 != 0o700 {
        return Err(Error::Identity);
    }
    Ok(())
}

pub(super) fn create_arguments(
    path: &std::path::Path,
    volume_name: &str,
    capacity: u64,
) -> Result<Vec<String>> {
    // Apple's -size suffix 'b' means 512-byte sectors, NOT bytes. Select the
    // explicit MiB option, and reject values outside our closed quota policy.
    if ![64 * 1024 * 1024, 512 * 1024 * 1024].contains(&capacity) {
        return Err(Error::Capacity);
    }
    Ok(commands::strings(&[
        "create",
        "-megabytes",
        &(capacity / (1024 * 1024)).to_string(),
        "-type",
        "UDIF",
        "-layout",
        "NONE",
        "-fs",
        "APFS",
        "-volname",
        volume_name,
        "-uid",
        &unsafe { libc::geteuid() }.to_string(),
        "-gid",
        &unsafe { libc::getegid() }.to_string(),
        "-mode",
        "0700",
        path.to_str().ok_or(Error::Identity)?,
    ]))
}
