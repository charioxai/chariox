use super::{Error, Result, DATA_BYTES, TMP_BYTES};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Component, Path};

/// Root-installed configuration. None of these fields are accepted over the
/// kernel connection; in particular a caller cannot choose another OS UID.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Enrollment {
    pub schema: String,
    pub owners: Vec<Owner>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Owner {
    pub uid: u32,
    pub gid: u32,
    pub cgroup_root: String,
}
impl Enrollment {
    pub fn validate(&self) -> Result<()> {
        if self.schema != "chariox.app-storage-enrollment.v1"
            || self.owners.is_empty()
            || self.owners.len() > 16
        {
            return Err(Error::Invalid);
        }
        let mut ids = std::collections::BTreeSet::new();
        for owner in &self.owners {
            if owner.uid == 0
                || owner.gid == 0
                || owner.uid == u32::MAX
                || owner.gid == u32::MAX
                || !ids.insert(owner.uid)
            {
                return Err(Error::Invalid);
            }
            let path = Path::new(&owner.cgroup_root);
            if !path.starts_with("/sys/fs/cgroup")
                || owner.cgroup_root.len() > 700
                || owner.cgroup_root.bytes().any(|b| b.is_ascii_control())
                || path
                    .components()
                    .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
                || owner.cgroup_root.ends_with('/')
                || owner.cgroup_root.contains("//")
                || owner.cgroup_root == "/sys/fs/cgroup"
            {
                return Err(Error::Invalid);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Request {
    Acquire {
        owner: String,
        installation: String,
        generation: u64,
        cgroup_leaf: String,
    },
    Release {
        lease: String,
    },
}
impl Request {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Acquire {
                owner,
                installation,
                generation,
                cgroup_leaf,
            } => {
                identifier(owner)?;
                identifier(installation)?;
                if *generation == 0 || *generation > i64::MAX as u64 {
                    return Err(Error::Invalid);
                }
                hex(cgroup_leaf.strip_prefix("app-").ok_or(Error::Invalid)?, 32)
            }
            Self::Release { lease } => hex(lease, 32),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Identity {
    pub device: u64,
    pub inode: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Image {
    pub role: Role,
    pub inode: Option<Identity>,
    pub uuid: String,
    pub capacity: u64,
    pub formatted: bool,
    pub retiring: bool,
    pub mount_id: Option<u64>,
    pub loop_minor: Option<u32>,
    pub mount_root: Identity,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(super) enum Role {
    Data,
    Tmp,
}
impl Role {
    pub fn directory(self) -> &'static str {
        match self {
            Self::Data => "data",
            Self::Tmp => "tmp",
        }
    }
    pub fn image(self) -> &'static str {
        match self {
            Self::Data => "data.ext4",
            Self::Tmp => "tmp.ext4",
        }
    }
    pub fn capacity(self) -> u64 {
        match self {
            Self::Data => DATA_BYTES,
            Self::Tmp => TMP_BYTES,
        }
    }
}

/// Fsynced before creating/formatting images and before making a lease usable.
/// Loop numbers and mount IDs are observations; image/cgroup inode and ext4 UUID
/// must be revalidated before any detach, mount, kill or delete after recovery.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Journal {
    pub schema: String,
    pub uid: u32,
    pub gid: u32,
    pub owner: String,
    pub installation: String,
    pub generation: u64,
    pub lease: String,
    pub cgroup_leaf: String,
    pub cgroup_identity: Identity,
    pub cgroup_root_identity: Identity,
    pub boot_id: String,
    pub pending_recovery: bool,
    pub images: [Image; 2],
}
impl Journal {
    pub fn validate(&self, uid: u32, name: &str) -> Result<()> {
        if self.schema != "chariox.app-storage-linux.v1"
            || self.uid != uid
            || self.gid == 0
            || self.gid == u32::MAX
            || self.generation == 0
            || self.generation > i64::MAX as u64
            || self.cgroup_identity.inode == 0
            || self.cgroup_root_identity.inode == 0
        {
            return Err(Error::Identity);
        }
        uuid(&self.boot_id)?;
        identifier(&self.owner)?;
        identifier(&self.installation)?;
        hex(&self.lease, 32)?;
        hex(
            self.cgroup_leaf
                .strip_prefix("app-")
                .ok_or(Error::Identity)?,
            32,
        )?;
        if name != installation_name(&self.owner, &self.installation)? {
            return Err(Error::Identity);
        }
        for (index, image) in self.images.iter().enumerate() {
            let role = if index == 0 { Role::Data } else { Role::Tmp };
            if image.role != role
                || image.capacity != role.capacity()
                || (image.formatted && image.inode.is_none())
                || image.inode.as_ref().is_some_and(|id| id.inode == 0)
                || image.mount_id == Some(0)
                || image.loop_minor.is_some_and(|minor| minor >= 4096)
                || (image.mount_id.is_some() && !image.formatted)
                || (image.retiring && image.role == Role::Data && image.formatted)
                || image.mount_root.inode == 0
            {
                return Err(Error::Identity);
            }
            uuid(&image.uuid)?;
        }
        Ok(())
    }
}

pub(super) fn installation_name(owner: &str, installation: &str) -> Result<String> {
    identifier(owner)?;
    identifier(installation)?;
    Ok(format!(
        "i-{:x}",
        Sha256::digest(format!("{owner}\0{installation}"))
    ))
}
fn identifier(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        Err(Error::Invalid)
    } else {
        Ok(())
    }
}
pub(super) fn hex(value: &str, length: usize) -> Result<()> {
    if value.len() != length
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        Err(Error::Invalid)
    } else {
        Ok(())
    }
}
pub(super) fn uuid(value: &str) -> Result<()> {
    if value.len() != 36
        || value.bytes().enumerate().any(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b != b'-'
            } else {
                !b.is_ascii_digit() && !(b'a'..=b'f').contains(&b)
            }
        })
    {
        Err(Error::Invalid)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requests_cannot_supply_authority_paths_or_change_closed_policy() {
        let request = r#"{"operation":"acquire","owner":"owner","installation":"app","generation":1,"cgroup_leaf":"app-11111111111111111111111111111111"}"#;
        serde_json::from_str::<Request>(request)
            .unwrap()
            .validate()
            .unwrap();
        for field in ["uid", "path", "device", "quota", "command"] {
            let injected = request.replacen('{', &format!("{{\"{field}\":1,"), 1);
            assert!(
                serde_json::from_str::<Request>(&injected).is_err(),
                "{field}"
            );
        }
        for leaf in [
            "../foreign",
            "app-0",
            "app-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        ] {
            assert!(Request::Acquire {
                owner: "owner".into(),
                installation: "app".into(),
                generation: 1,
                cgroup_leaf: leaf.into()
            }
            .validate()
            .is_err());
        }
        assert_ne!(
            installation_name("a", "bc").unwrap(),
            installation_name("ab", "c").unwrap()
        );
        assert_eq!(installation_name("../owner", "/app").unwrap().len(), 66);
    }
}
