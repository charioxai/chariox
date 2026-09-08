use super::{Error, Result, DATA_BYTES, TMP_BYTES};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};

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
    #[serde(default)]
    pub kernel_database_paths: Vec<PathBuf>,
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
            if owner.kernel_database_paths.len() > 16 {
                return Err(Error::Invalid);
            }
            let mut databases = std::collections::BTreeSet::new();
            for database in &owner.kernel_database_paths {
                let path = database.to_str().ok_or(Error::Invalid)?;
                if !database.is_absolute()
                    || database.file_name().is_none()
                    || path.len() > 1024
                    || path.ends_with('/')
                    || path.contains("//")
                    || path.split('/').any(|part| matches!(part, "." | ".."))
                    || path.bytes().any(|byte| byte.is_ascii_control())
                    || database
                        .components()
                        .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
                    || !databases.insert(database)
                {
                    return Err(Error::Invalid);
                }
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
    AttachCode {
        lease: String,
        package_digest: String,
        runtime_digest: String,
        runtime_revision: u64,
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
            Self::AttachCode {
                lease,
                package_digest,
                runtime_digest,
                runtime_revision,
            } => {
                hex(lease, 32)?;
                super::code_model::validate_pins(package_digest, runtime_digest, *runtime_revision)
            }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<super::code_model::Record>,
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
        if let Some(code) = &self.code {
            code.validate()?;
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

#[cfg(test)]
mod enrollment_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn installed_database_sources_are_bounded_canonical_and_never_socket_arguments() {
        let base = json!({"schema":"chariox.app-storage-enrollment.v1","owners":[{"uid":1000,"gid":1000,"cgroup_root":"/sys/fs/cgroup/service/apps","kernel_database_paths":["/var/lib/chariox/home/state/kernel.db"]}]});
        let check = |value| {
            serde_json::from_value::<Enrollment>(value)
                .unwrap()
                .validate()
        };
        assert!(check(base.clone()).is_ok());
        for paths in [
            json!(["relative.db"]),
            json!(["/state/../kernel.db"]),
            json!(["/state/./kernel.db"]),
            json!(["/state//kernel.db"]),
            json!(["/state/kernel.db/"]),
            json!(["/state/kernel.db", "/state/kernel.db"]),
            json!(vec!["/state/kernel.db"; 17]),
        ] {
            let mut value = base.clone();
            value["owners"][0]["kernel_database_paths"] = paths;
            assert!(check(value).is_err());
        }
        let request = json!({"operation":"attach_code","lease":"a".repeat(32),"package_digest":format!("sha256:{}","b".repeat(64)),"runtime_digest":"c".repeat(64),"runtime_revision":1});
        serde_json::from_value::<Request>(request.clone())
            .unwrap()
            .validate()
            .unwrap();
        for field in [
            "path",
            "kernel_database_path",
            "runtime_root",
            "uid",
            "command",
        ] {
            let mut value = request.clone();
            value[field] = json!("/foreign");
            assert!(serde_json::from_value::<Request>(value).is_err());
        }
    }
}

#[cfg(test)]
mod capacity_tests {
    #[test]
    fn installed_fd_limit_covers_all_bounded_graph_leases_and_transient_work() {
        // Each live lease:40 signed graph files, runtime5 metadata/root/lease,
        // release2 roots, installation1 directory, and bound cgroup4 files.
        // The helper serializes operations. The128 transient/global descriptors
        // are reserved headroom for a <=24-level walk, new graph, formatter
        // and daemon root, not an assertion of a measured peak.
        let maximum = super::super::MAX_INSTALLATIONS
            * (crate::runtime_enrollment::MAX_INVENTORY_FILES + 12)
            + 64
            + 16
            + 128;
        let unit = include_str!("../../../../../deploy/managed-kernel/chariox-app-storage.service");
        let ceiling = unit
            .lines()
            .find_map(|line| line.strip_prefix("LimitNOFILE="))
            .unwrap()
            .parse::<usize>()
            .unwrap();
        assert_eq!(ceiling, 4096);
        assert!(maximum <= ceiling);
    }
}
