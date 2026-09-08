use super::{identity::FileIdentity, Error, Result};
use crate::private_fs::{Dir, FsError};
use serde::{Deserialize, Serialize};
use std::{ffi::OsStr, io::Read};

pub(super) const NAME: &str = "storage.json";
pub(super) const METADATA_ALLOWANCE: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Image {
    pub role: String,
    pub image: String,
    pub volume_name: String,
    pub capacity: u64,
    pub identity: Option<FileIdentity>,
    pub volume_uuid: Option<String>,
    pub mount_identity: FileIdentity,
}
impl Image {
    pub fn reserved(&self) -> u64 {
        self.capacity + METADATA_ALLOWANCE
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Journal {
    pub schema: String,
    pub owner: String,
    pub installation: String,
    pub generation: u64,
    /// True before any Apple tool can create/attach/mount an image, and only
    /// cleared after exact-identity detachment has been observed and synced.
    pub pending_recovery: bool,
    pub images: [Image; 2],
}
impl Journal {
    pub fn validate(&self) -> Result<()> {
        if self.schema != "chariox.app-storage.v1"
            || self.generation == 0
            || self.generation > i64::MAX as u64
        {
            return Err(Error::Metadata);
        }
        identifier(&self.owner)?;
        identifier(&self.installation)?;
        for (index, image) in self.images.iter().enumerate() {
            let role = ["data", "tmp"][index];
            let nonce = image
                .image
                .strip_prefix(&format!("{role}-"))
                .and_then(|v| v.strip_suffix(".dmg"))
                .ok_or(Error::Metadata)?;
            if nonce.len() != 32
                || !nonce
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                || image.role != role
                || image.volume_name != format!("cx-{role}-{nonce}")
                || ![64 * 1024 * 1024, 512 * 1024 * 1024].contains(&image.capacity)
                || (index == 1 && image.capacity != 64 * 1024 * 1024)
                || image.mount_identity.inode == 0
            {
                return Err(Error::Metadata);
            }
            if let Some(uuid) = &image.volume_uuid {
                super::identity::uuid(uuid)?;
            }
        }
        Ok(())
    }
    pub fn save(&self, dir: &Dir) -> Result<()> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|_| Error::Metadata)?;
        if bytes.len() > 16384 {
            return Err(Error::Metadata);
        }
        dir.atomic_replace(OsStr::new(NAME), &bytes)?;
        Ok(())
    }
}

pub(super) fn recover_temporaries(dir: &Dir) -> Result<()> {
    for name in dir.entries(16)? {
        if !crate::private_fs::is_atomic_temporary(&name) {
            continue;
        }
        let file = dir.open_private_file(&name)?;
        if !crate::private_fs::try_lock_file(&file)? {
            return Err(Error::Busy);
        }
        FileIdentity::of(&file)?.require(dir, &name, &file)?;
        dir.remove_file(&name)?;
    }
    dir.sync()?;
    Ok(())
}

pub(super) fn load(dir: &Dir) -> Result<Option<Journal>> {
    // Complete any interrupted metadata publication before observing its result.
    dir.sync()?;
    let file = match dir.open_private_file(OsStr::new(NAME)) {
        Ok(file) => file,
        Err(FsError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.take(16385).read_to_end(&mut bytes)?;
    if bytes.len() > 16384 {
        return Err(Error::Metadata);
    }
    let journal: Journal = serde_json::from_slice(&bytes).map_err(|_| Error::Metadata)?;
    journal.validate()?;
    Ok(Some(journal))
}

pub(super) fn identifier(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || value
            .bytes()
            .any(|b| !b.is_ascii_alphanumeric() && !b"-_.".contains(&b))
    {
        return Err(Error::Identity);
    }
    Ok(())
}

pub(super) fn image(role: &str, capacity: u64, mount_identity: FileIdentity) -> Image {
    let mut random = [0_u8; 16];
    unsafe {
        libc::arc4random_buf(random.as_mut_ptr().cast(), random.len());
    }
    let nonce = random
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    Image {
        role: role.into(),
        image: format!("{role}-{nonce}.dmg"),
        volume_name: format!("cx-{role}-{nonce}"),
        capacity,
        identity: None,
        volume_uuid: None,
        mount_identity,
    }
}
