use std::fs::{self, OpenOptions};
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

use serde::Deserialize;

use crate::error::DaemonError;

const MAX_OBSERVATION_BYTES: u64 = 4096;
const MAX_MOUNTINFO_BYTES: u64 = 1024 * 1024;
const DATA_ROOT: &str = "/var/lib/chariox-docker/data";
const MIN_VOLUME_SIZE_GB: u32 = 10;
const MAX_VOLUME_SIZE_GB: u32 = 10_000;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AdmissionObservation {
    schema_version: u32,
    linux_boot_id: String,
    data_volume_serial: String,
    data_volume_size_gb: u32,
    filesystem_uuid: String,
    device_path: String,
    major_minor: String,
    mount_target: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AdmittedDataVolumeIdentity {
    pub(super) serial: String,
    pub(super) size_gb: u32,
}

pub(super) fn read_admitted_data_volume(
    observation_path: &Path,
    mountinfo_path: &Path,
    linux_boot_id: &str,
    expected_serial: &str,
    expected_size_gb: u32,
) -> Result<AdmittedDataVolumeIdentity, DaemonError> {
    let observation = read_observation(observation_path)?;
    validate_observation(
        &observation,
        linux_boot_id,
        expected_serial,
        expected_size_gb,
    )?;
    let mountinfo = read_bounded(mountinfo_path, MAX_MOUNTINFO_BYTES, "current mount table")?;
    validate_current_mount(&mountinfo, &observation)?;
    Ok(AdmittedDataVolumeIdentity {
        serial: observation.data_volume_serial,
        size_gb: observation.data_volume_size_gb,
    })
}

fn read_observation(path: &Path) -> Result<AdmissionObservation, DaemonError> {
    let parent = path
        .parent()
        .ok_or_else(|| observation_error("data-volume observation path is invalid"))?;
    let parent_metadata = fs::symlink_metadata(parent)
        .map_err(|_| observation_error("root-owned data-volume observation directory is missing"))?;
    if parent_metadata.file_type().is_symlink()
        || !parent_metadata.is_dir()
        || parent_metadata.uid() != 0
        || parent_metadata.gid() != 0
        || parent_metadata.permissions().mode() & 0o7777 != 0o755
    {
        return Err(observation_error(
            "data-volume observation directory is not root-owned and read-only to Chariox",
        ));
    }
    let runtime = parent
        .parent()
        .ok_or_else(|| observation_error("data-volume runtime path is invalid"))?;
    let runtime_metadata = fs::symlink_metadata(runtime)
        .map_err(|_| observation_error("root-owned data-volume runtime directory is missing"))?;
    if runtime_metadata.file_type().is_symlink()
        || !runtime_metadata.is_dir()
        || runtime_metadata.uid() != 0
        || runtime_metadata.permissions().mode() & 0o022 != 0
    {
        return Err(observation_error("data-volume runtime directory is unsafe"));
    }

    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| observation_error("root-owned data-volume observation is missing"))?;
    let metadata = file
        .metadata()
        .map_err(|_| observation_error("data-volume observation metadata is unavailable"))?;
    if !metadata.is_file()
        || metadata.uid() != 0
        || metadata.gid() != 0
        || metadata.permissions().mode() & 0o7777 != 0o644
        || metadata.len() == 0
        || metadata.len() > MAX_OBSERVATION_BYTES
    {
        return Err(observation_error(
            "data-volume observation is not a bounded root-owned read-only file",
        ));
    }
    let bytes = read_bounded_file(&mut file, MAX_OBSERVATION_BYTES, "data-volume observation")?;
    serde_json::from_slice(&bytes)
        .map_err(|_| observation_error("data-volume observation has an invalid shape"))
}

fn validate_observation(
    observation: &AdmissionObservation,
    linux_boot_id: &str,
    expected_serial: &str,
    expected_size_gb: u32,
) -> Result<(), DaemonError> {
    if observation.schema_version != 1
        || observation.linux_boot_id != linux_boot_id
        || !valid_volume_serial(&observation.data_volume_serial)
        || observation.data_volume_serial != expected_serial
        || !(MIN_VOLUME_SIZE_GB..=MAX_VOLUME_SIZE_GB).contains(&observation.data_volume_size_gb)
        || observation.data_volume_size_gb != expected_size_gb
        || !valid_filesystem_uuid(&observation.filesystem_uuid)
        || !observation.device_path.starts_with("/dev/")
        || !valid_major_minor(&observation.major_minor)
        || observation.mount_target != DATA_ROOT
    {
        return Err(observation_error(
            "data-volume admission identity does not match the protected bootstrap input",
        ));
    }
    Ok(())
}

fn validate_current_mount(
    mountinfo: &str,
    observation: &AdmissionObservation,
) -> Result<(), DaemonError> {
    let mut matches = mountinfo.lines().filter(|line| {
        let Some((mount_fields, filesystem_fields)) = line.split_once(" - ") else {
            return false;
        };
        let mount_fields: Vec<_> = mount_fields.split_ascii_whitespace().collect();
        let filesystem_fields: Vec<_> = filesystem_fields.split_ascii_whitespace().collect();
        if mount_fields.len() < 6
            || filesystem_fields.len() < 3
            || mount_fields[4] != observation.mount_target
        {
            return false;
        }
        let quota_enabled = mount_fields[5]
            .split(',')
            .chain(filesystem_fields[2].split(','))
            .any(|option| matches!(option, "pquota" | "prjquota"));
        mount_fields[2] == observation.major_minor
            && filesystem_fields[0] == "xfs"
            && quota_enabled
    });
    if matches.next().is_none() || matches.next().is_some() {
        return Err(observation_error(
            "current data-root mount does not match the admitted XFS volume",
        ));
    }
    Ok(())
}

fn read_bounded(path: &Path, maximum: u64, label: &str) -> Result<String, DaemonError> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| observation_error(&format!("{label} could not be read")))?;
    let bytes = read_bounded_file(&mut file, maximum, label)?;
    String::from_utf8(bytes).map_err(|_| observation_error(&format!("{label} is not UTF-8")))
}

fn read_bounded_file(file: &mut impl Read, maximum: u64, label: &str) -> Result<Vec<u8>, DaemonError> {
    let mut bytes = Vec::new();
    file.take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| observation_error(&format!("{label} could not be read")))?;
    if bytes.len() as u64 > maximum {
        return Err(observation_error(&format!("{label} exceeds its size limit")));
    }
    Ok(bytes)
}

fn valid_volume_serial(value: &str) -> bool {
    (1..=16).contains(&value.len())
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value.as_bytes()[0] != b'0'
        && value.parse::<u64>().is_ok_and(|number| number > 0 && number <= 9_007_199_254_740_991)
}

fn valid_filesystem_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

fn valid_major_minor(value: &str) -> bool {
    let Some((major, minor)) = value.split_once(':') else {
        return false;
    };
    !major.is_empty()
        && !minor.is_empty()
        && major.bytes().all(|byte| byte.is_ascii_digit())
        && minor.bytes().all(|byte| byte.is_ascii_digit())
}

fn observation_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "read admitted managed data-volume identity",
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{validate_current_mount, validate_observation, AdmissionObservation};

    fn observation() -> AdmissionObservation {
        AdmissionObservation {
            schema_version: 1,
            linux_boot_id: "01234567-89ab-cdef-0123-456789abcdef".to_string(),
            data_volume_serial: "12345".to_string(),
            data_volume_size_gb: 10,
            filesystem_uuid: "12345678-1234-1234-1234-123456789abc".to_string(),
            device_path: "/dev/sdb".to_string(),
            major_minor: "8:16".to_string(),
            mount_target: "/var/lib/chariox-docker/data".to_string(),
        }
    }

    #[test]
    fn admitted_identity_must_match_protected_volume_and_current_boot() {
        let value = observation();
        assert!(validate_observation(
            &value,
            "01234567-89ab-cdef-0123-456789abcdef",
            "12345",
            10,
        )
        .is_ok());
        assert!(validate_observation(
            &value,
            "fedcba98-7654-3210-fedc-ba9876543210",
            "12345",
            10,
        )
        .is_err());
        assert!(validate_observation(
            &value,
            "01234567-89ab-cdef-0123-456789abcdef",
            "54321",
            10,
        )
        .is_err());
    }

    #[test]
    fn current_mount_must_be_the_observed_xfs_volume_with_quotas() {
        let value = observation();
        let mountinfo = "36 25 8:16 / /var/lib/chariox-docker/data rw,relatime - xfs /dev/sdb rw,attr2,prjquota\n";
        assert!(validate_current_mount(mountinfo, &value).is_ok());
        assert!(validate_current_mount(
            "36 25 8:17 / /var/lib/chariox-docker/data rw,relatime - xfs /dev/sdb rw,attr2,prjquota\n",
            &value,
        )
        .is_err());
        assert!(validate_current_mount(
            "36 25 8:16 / /var/lib/chariox-docker/data rw,relatime - xfs /dev/sdb rw,attr2\n",
            &value,
        )
        .is_err());
    }
}
