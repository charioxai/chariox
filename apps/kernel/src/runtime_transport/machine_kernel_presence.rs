use std::fs;
use std::io::{self, Read as _};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::DaemonConfig;
use crate::local::{KernelRuntimeProcessIdentity, RelayStatus};
use crate::session::{unix_epoch_ms, RuntimeSession};

const FRESHNESS_MS: u64 = 30_000;
const RECORD_RETENTION: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
const MAX_RECORD_BYTES: u64 = 16 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct KernelIdentity {
    pub(crate) kernel_id: String,
    label: String,
    process_id: u32,
    cloud: CloudLink,
    running: bool,
    relay_connected: bool,
    workload: Option<KernelWorkload>,
    observed_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum CloudLink {
    Linked {
        kernel_id: String,
    },
    NotLinked {
        #[serde(skip_serializing_if = "Option::is_none")]
        former_kernel_id: Option<String>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct KernelWorkload {
    workflows: usize,
    endpoints: usize,
    last_room_activity_at_ms: Option<u64>,
}

impl KernelIdentity {
    pub(crate) fn new(
        config: &DaemonConfig,
        relay: &RelayStatus,
        sessions: Option<&[std::sync::Arc<RuntimeSession>]>,
    ) -> Self {
        let cloud = match config
            .cloud_relay
            .as_ref()
            .map(crate::runtime::cloud_api_client::cloud_profile_from_persisted)
            .filter(|profile| profile.kernel_enrolled)
            .and_then(|profile| profile.kernel_id)
        {
            Some(kernel_id) => CloudLink::Linked { kernel_id },
            None => CloudLink::NotLinked {
                former_kernel_id: config.former_cloud_kernel_id.clone(),
            },
        };
        let workload = sessions.map(|sessions| {
            let mut workload = KernelWorkload {
                workflows: 0,
                endpoints: 0,
                last_room_activity_at_ms: None,
            };
            for session in sessions {
                workload.workflows += session.workflows().len();
                workload.endpoints += session
                    .workflows()
                    .iter()
                    .map(|workflow| workflow.endpoints().len())
                    .sum::<usize>();
                if !session.is_hidden() {
                    workload.last_room_activity_at_ms = workload
                        .last_room_activity_at_ms
                        .max(Some(session.last_activity_at_ms()));
                }
            }
            workload
        });
        Self {
            kernel_id: relay.daemon_id.clone(),
            label: relay
                .machine_alias
                .clone()
                .unwrap_or_else(|| relay.daemon_id.clone()),
            process_id: std::process::id(),
            cloud,
            running: true,
            relay_connected: relay.connected,
            workload,
            observed_at_ms: unix_epoch_ms(),
        }
    }

    fn process_key(&self) -> (String, u32) {
        (self.kernel_id.clone(), self.process_id)
    }

    pub(crate) fn response(&self) -> serde_json::Value {
        let cloud_link = match &self.cloud {
            CloudLink::Linked { kernel_id } => format!("linked as {kernel_id}"),
            CloudLink::NotLinked {
                former_kernel_id: Some(kernel_id),
            } => format!("not linked; was {kernel_id}"),
            CloudLink::NotLinked {
                former_kernel_id: None,
            } => "not linked".to_owned(),
        };
        serde_json::json!({"kernel": self, "cloud_link": cloud_link})
    }
}

#[derive(Deserialize, Serialize)]
struct PresenceRecord {
    kernel: KernelIdentity,
    process_identity: Option<KernelRuntimeProcessIdentity>,
}

pub(crate) struct MachineKernelPresenceLease {
    path: PathBuf,
    process_identity: Option<KernelRuntimeProcessIdentity>,
}

impl MachineKernelPresenceLease {
    pub(crate) fn start(kernel: KernelIdentity, relay: &RelayStatus) -> io::Result<Self> {
        let directory = presence_directory();
        let uid = euid();
        private_directory(
            directory.parent().expect("machine presence parent"),
            uid,
            true,
        )?;
        private_directory(&directory, uid, true)?;
        prune_old_records(&directory);
        let lease = Self {
            path: directory.join(format!(
                "{}-{:032x}.json",
                std::process::id(),
                rand::random::<u128>()
            )),
            process_identity: relay.runtime_process_identity.clone(),
        };
        lease.update(kernel)?;
        Ok(lease)
    }

    pub(crate) fn update(&self, kernel: KernelIdentity) -> io::Result<()> {
        use std::io::Write as _;

        let contents = serde_json::to_vec(&PresenceRecord {
            kernel,
            process_identity: self.process_identity.clone(),
        })?;
        let temp = self.path.with_extension("tmp");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        let result = file
            .write_all(&contents)
            .and_then(|()| fs::rename(&temp, &self.path));
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    }
}

impl Drop for MachineKernelPresenceLease {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub(crate) fn list_response(current: KernelIdentity) -> io::Result<serde_json::Value> {
    let directory = presence_directory();
    let parent = directory.parent().expect("machine presence parent");
    match private_directory(parent, euid(), false) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    list_from(&directory, current, unix_epoch_ms())
}

fn list_from(
    directory: &Path,
    current: KernelIdentity,
    now_ms: u64,
) -> io::Result<serde_json::Value> {
    let mut unreadable = 0;
    let mut unverified = 0;
    let current_key = current.process_key();
    let mut kernels = std::collections::BTreeMap::from([(current_key.clone(), current)]);
    let available = match private_directory(directory, euid(), false) {
        Ok(()) => {
            for entry in fs::read_dir(directory)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().is_none_or(|suffix| suffix != "json") {
                    continue;
                }
                let record = read_record(&path);
                let Ok(record) = record else {
                    unreadable += 1;
                    continue;
                };
                if !record.is_current(now_ms) {
                    unverified += 1;
                    continue;
                }
                let identity = record.kernel;
                let key = identity.process_key();
                if key != current_key
                    && kernels
                        .get(&key)
                        .is_none_or(|reported| identity.observed_at_ms > reported.observed_at_ms)
                {
                    kernels.insert(key, identity);
                }
            }
            true
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error),
    };
    Ok(serde_json::json!({
        "scope": "reporting kernels on this machine for this OS user",
        "registry_available": available,
        "unreadable_records": unreadable,
        "unverified_records": unverified,
        "kernels": kernels.values().map(KernelIdentity::response).collect::<Vec<_>>()
    }))
}

impl PresenceRecord {
    fn is_current(&self, now_ms: u64) -> bool {
        if now_ms.abs_diff(self.kernel.observed_at_ms) > FRESHNESS_MS {
            return false;
        }
        match &self.process_identity {
            Some(identity) => {
                self.kernel.process_id == identity.pid
                    && crate::runtime::kernel_process_identity::for_pid(identity.pid).as_ref()
                        == Some(identity)
            }
            None => !cfg!(target_os = "linux"),
        }
    }
}

fn read_record(path: &Path) -> io::Result<PresenceRecord> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.uid() != euid() || metadata.mode() & 0o077 != 0 {
        return Err(io::Error::other(
            "machine presence record must be an owner-private file",
        ));
    }
    if metadata.len() > MAX_RECORD_BYTES {
        return Err(io::Error::other("machine presence record is too large"));
    }
    Ok(serde_json::from_reader(file.take(MAX_RECORD_BYTES))?)
}

fn private_directory(directory: &Path, uid: u32, create: bool) -> io::Result<()> {
    if create {
        match fs::DirBuilder::new().mode(0o700).create(directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    let metadata = fs::symlink_metadata(directory)?;
    if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
        return Err(io::Error::other(
            "machine presence directory must be owner-private",
        ));
    }
    Ok(())
}

fn presence_directory() -> PathBuf {
    PathBuf::from(format!("/tmp/chariox-{}/kernel-presence", euid()))
}

fn euid() -> u32 {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() }
}

fn prune_old_records(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_file())
            || !entry
                .path()
                .extension()
                .is_some_and(|suffix| suffix == "json" || suffix == "tmp")
        {
            continue;
        }
        if entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|time| time.elapsed().is_ok_and(|age| age > RECORD_RETENTION))
        {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(id: &str, observed_at_ms: u64) -> KernelIdentity {
        KernelIdentity {
            kernel_id: id.into(),
            label: id.into(),
            process_id: std::process::id(),
            cloud: CloudLink::Linked {
                kernel_id: id.into(),
            },
            running: true,
            relay_connected: true,
            workload: None,
            observed_at_ms,
        }
    }

    #[test]
    fn kernel_map_lists_fresh_processes_and_reports_incomplete_coverage() {
        let root = crate::test_temp_dir::TestTempDir::new("machine-kernel-map");
        let directory = root.path().join("presence");
        private_directory(&directory, euid(), true).unwrap();
        let now = FRESHNESS_MS * 2;
        let process_identity = crate::runtime::kernel_process_identity::current();
        let mut leases = Vec::new();
        for (id, observed) in [
            ("kernel-a", now),
            ("kernel-b", now - 10),
            ("kernel-c", now),
            ("expired", now - FRESHNESS_MS - 1),
        ] {
            let lease = MachineKernelPresenceLease {
                path: directory.join(format!("{id}.json")),
                process_identity: process_identity.clone(),
            };
            lease.update(identity(id, observed)).unwrap();
            leases.push(lease);
        }
        let found = list_from(&directory, identity("kernel-here", now), now).unwrap();
        let entries = found["kernels"].as_array().unwrap();
        let ids: Vec<_> = entries
            .iter()
            .map(|entry| entry["kernel"]["cloud"]["kernel_id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["kernel-a", "kernel-b", "kernel-c", "kernel-here"]);
        assert_eq!(found["unverified_records"], 1);
        assert_eq!(found["registry_available"], true);
        let no_registry =
            list_from(&directory.join("absent"), identity("kernel-here", now), now).unwrap();
        assert_eq!(no_registry["registry_available"], false);
        assert_eq!(no_registry["kernels"].as_array().unwrap().len(), 1);
    }
}
