use super::{
    commands::{self, Tool},
    Error, Result,
};
use crate::private_fs::{entry_metadata, Dir};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    ffi::{CStr, OsStr},
    fs::File,
    mem::MaybeUninit,
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::Path,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileIdentity {
    pub device: u64,
    pub inode: u64,
}
impl FileIdentity {
    pub fn of(file: &File) -> Result<Self> {
        let metadata = file.metadata()?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
    pub fn require(&self, parent: &Dir, name: &OsStr, file: &File) -> Result<()> {
        let current = entry_metadata(parent, name)?;
        if current.st_dev as u64 != self.device
            || current.st_ino as u64 != self.inode
            || Self::of(file)? != *self
        {
            return Err(Error::Identity);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Attachment {
    pub whole: String,
    pub volume: String,
    pub uuid: String,
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty() && v.len() <= 2048)
        .ok_or(Error::Metadata)
}
pub(super) fn device(value: &str) -> Result<()> {
    let mut fields = value
        .strip_prefix("/dev/disk")
        .ok_or(Error::Identity)?
        .split('s');
    let number = |v: &str| {
        !v.is_empty()
            && v.len() <= 5
            && !v.starts_with('0')
            && v.bytes().all(|b| b.is_ascii_digit())
    };
    if !number(fields.next().ok_or(Error::Identity)?) {
        return Err(Error::Identity);
    }
    let rest: Vec<_> = fields.collect();
    if rest.len() > 2 || rest.iter().any(|v| !number(v)) {
        return Err(Error::Identity);
    }
    Ok(())
}
pub(super) fn uuid(value: &str) -> Result<()> {
    if value.len() != 36
        || value.bytes().enumerate().any(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b != b'-'
            } else {
                !b.is_ascii_hexdigit()
            }
        })
    {
        return Err(Error::Identity);
    }
    Ok(())
}

/// Disk identifiers are discovered from the exact owned image on every action.
/// A previously recorded /dev name is never sufficient authority to detach.
pub(super) fn image_entities(info: &Value, image: &Path) -> Result<Option<Vec<String>>> {
    let images = info
        .get("images")
        .and_then(Value::as_array)
        .ok_or(Error::Metadata)?;
    if images.len() > 256 {
        return Err(Error::Metadata);
    }
    let expected = image.to_str().ok_or(Error::Identity)?;
    let mut found = None;
    for item in images {
        if string(item, "image-path")? != expected {
            continue;
        }
        if found.is_some() {
            return Err(Error::Identity);
        }
        let entities = item
            .get("system-entities")
            .and_then(Value::as_array)
            .ok_or(Error::Metadata)?;
        if entities.is_empty() || entities.len() > 16 {
            return Err(Error::Metadata);
        }
        let mut names = Vec::new();
        for entity in entities {
            let name = string(entity, "dev-entry")?;
            device(name)?;
            if names.iter().any(|prior| prior == name) {
                return Err(Error::Identity);
            }
            names.push(name.to_owned());
        }
        found = Some(names);
    }
    Ok(found)
}

pub(super) fn volume_info(info: &Value, name: &str) -> Result<Option<(String, String)>> {
    if info.get("FilesystemType").and_then(Value::as_str) != Some("apfs") {
        return Ok(None);
    }
    let node = string(info, "DeviceNode")?;
    device(node)?;
    if string(info, "VolumeName")? != name {
        return Err(Error::Identity);
    }
    let id = string(info, "VolumeUUID")?;
    uuid(id)?;
    Ok(Some((node.into(), id.to_ascii_uppercase())))
}

pub(super) fn whole_info(info: &Value, expected: &str) -> Result<()> {
    if info.get("WholeDisk").and_then(Value::as_bool) != Some(true)
        || info.get("VirtualOrPhysical").and_then(Value::as_str) != Some("Virtual")
        || string(info, "DeviceNode")? != expected
    {
        return Err(Error::Identity);
    }
    Ok(())
}

pub(super) fn discover(
    image: &Path,
    name: &str,
    expected_uuid: Option<&str>,
    context: commands::Context<'_>,
) -> Result<Option<Attachment>> {
    let info = commands::plist(
        Tool::Hdiutil,
        &commands::strings(&["info", "-plist"]),
        context,
    )?;
    let Some(nodes) = image_entities(&info, image)? else {
        return Ok(None);
    };
    let whole: Vec<_> = nodes
        .iter()
        .filter(|n| !n.trim_start_matches("/dev/disk").contains('s'))
        .collect();
    // APFS exposes a physical image device and often a synthesized container.
    // hdiutil lists the physical device first, and it is confirmed with diskutil.
    let whole = whole.first().ok_or(Error::Identity)?.to_string();
    let whole_info = commands::plist(
        Tool::Diskutil,
        &commands::strings(&["info", "-plist", &whole]),
        context,
    )?;
    self::whole_info(&whole_info, &whole)?;
    let mut volume = None;
    for node in &nodes {
        let info = commands::plist(
            Tool::Diskutil,
            &commands::strings(&["info", "-plist", node]),
            context,
        )?;
        if let Some((actual, id)) = volume_info(&info, name)? {
            if actual != *node
                || expected_uuid.is_some_and(|expected| expected != id)
                || volume.is_some()
            {
                return Err(Error::Identity);
            }
            volume = Some((actual, id));
        }
    }
    let (volume, uuid) = volume.ok_or(Error::Identity)?;
    Ok(Some(Attachment {
        whole,
        volume,
        uuid,
    }))
}

pub(super) fn mounted(dir: &Dir, mount: &Path, attachment: &Attachment) -> Result<()> {
    let mut stat = MaybeUninit::<libc::statfs>::zeroed();
    if unsafe { libc::fstatfs(dir.0.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
        return Err(Error::Io);
    }
    let stat = unsafe { stat.assume_init() };
    let required = (libc::MNT_NOEXEC | libc::MNT_NODEV | libc::MNT_NOSUID) as u32;
    #[cfg(test)]
    eprintln!(
        "storage_mount_observation flags={} required={} ignore_ownership={} apfs={} device_matches={} mountpoint_matches={}",
        stat.f_flags,
        required,
        stat.f_flags & libc::MNT_IGNORE_OWNERSHIP as u32 != 0,
        unsafe { CStr::from_ptr(stat.f_fstypename.as_ptr()) }.to_bytes() == b"apfs",
        unsafe { CStr::from_ptr(stat.f_mntfromname.as_ptr()) }.to_bytes() == attachment.volume.as_bytes(),
        unsafe { CStr::from_ptr(stat.f_mntonname.as_ptr()) }.to_bytes() == mount.as_os_str().as_encoded_bytes(),
    );
    if stat.f_flags & required != required
        || stat.f_flags & libc::MNT_IGNORE_OWNERSHIP as u32 != 0
        || unsafe { CStr::from_ptr(stat.f_fstypename.as_ptr()) }.to_bytes() != b"apfs"
        || unsafe { CStr::from_ptr(stat.f_mntfromname.as_ptr()) }.to_bytes()
            != attachment.volume.as_bytes()
        || unsafe { CStr::from_ptr(stat.f_mntonname.as_ptr()) }.to_bytes()
            != mount.as_os_str().as_encoded_bytes()
    {
        return Err(Error::Identity);
    }
    Ok(())
}
