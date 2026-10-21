//! The kernels this kernel's gateway paired with through a terminal pairing
//! link (self-hosted mode): `gateway-kernels.json`, readable by this user only.
use std::path::PathBuf;
use std::sync::Mutex;

use chariox_relay::protocol::RelayKernelPresence;
use serde::{Deserialize, Serialize};

use super::error::GatewayError;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct PairedKernel {
    pub(super) presence: RelayKernelPresence,
    /// The operator's CLIENT transport token of a scoped relay; never returned by any request.
    #[serde(default)]
    pub(super) relay_token: Option<String>,
    pub(super) paired_at_ms: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct Stored {
    kernels: Vec<PairedKernel>,
}

pub(super) struct PairedKernels {
    path: PathBuf,
    lock: Mutex<()>,
}

fn unavailable(detail: String) -> GatewayError {
    GatewayError::Unreachable {
        name: "paired kernels".into(),
        detail,
    }
}

impl PairedKernels {
    pub(super) fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }

    fn read(&self) -> Result<Stored, GatewayError> {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| {
                unavailable(format!("gateway-kernels.json is unreadable: {error}"))
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Stored::default()),
            Err(error) => Err(unavailable(format!(
                "cannot read gateway-kernels.json: {error}"
            ))),
        }
    }

    pub(super) fn all(&self) -> Result<Vec<PairedKernel>, GatewayError> {
        let _guard = self.lock.lock().expect("paired kernels poisoned");
        Ok(self.read()?.kernels)
    }

    pub(super) fn find(&self, kernel_id: &str) -> Option<PairedKernel> {
        self.all()
            .ok()?
            .into_iter()
            .find(|target| target.presence.kernel_id == kernel_id)
    }

    /// Adds or replaces the kernel with the same kernel id.
    pub(super) fn save(&self, target: PairedKernel) -> Result<(), GatewayError> {
        let _guard = self.lock.lock().expect("paired kernels poisoned");
        let mut stored = self.read()?;
        stored
            .kernels
            .retain(|existing| existing.presence.kernel_id != target.presence.kernel_id);
        stored.kernels.push(target);
        self.write(&stored)
    }

    pub(super) fn remove(&self, kernel_id: &str) -> Result<bool, GatewayError> {
        let _guard = self.lock.lock().expect("paired kernels poisoned");
        let mut stored = self.read()?;
        let before = stored.kernels.len();
        stored
            .kernels
            .retain(|target| target.presence.kernel_id != kernel_id);
        let removed = stored.kernels.len() != before;
        if removed {
            self.write(&stored)?;
        }
        Ok(removed)
    }

    fn write(&self, stored: &Stored) -> Result<(), GatewayError> {
        let bytes = serde_json::to_vec_pretty(stored).expect("paired kernels serialize");
        crate::config::write_private_file(&self.path, &bytes)
            .map_err(|error| unavailable(format!("cannot store gateway-kernels.json: {error}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(id: &str, token: Option<&str>) -> PairedKernel {
        PairedKernel {
            presence: RelayKernelPresence {
                kernel_id: id.into(),
                machine_id: format!("machine-{id}"),
                machine_alias: None,
                relay_alias: None,
                kernel_alias: Some(id.into()),
                available_providers: Vec::new(),
                provider_accounts: Vec::new(),
                capabilities: Vec::new(),
                accepting_remote_leases: false,
                leased_agent_count: 0,
                local_session_count: 0,
                public_key: format!("key-{id}"),
            },
            relay_token: token.map(str::to_string),
            paired_at_ms: 5,
        }
    }

    // EA2-08: a pairing survives a restart, is replaced by a re-pairing, can be
    // forgotten, and the file is readable by this user only.
    #[test]
    fn pairings_are_stored_privately_replaced_and_forgotten() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("paired-{:08x}", rand::random::<u32>()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("gateway-kernels.json");
        let store = PairedKernels::new(path.clone());
        assert!(store.all().unwrap().is_empty());
        store.save(target("b3", Some("client-token"))).unwrap();
        store.save(target("other", None)).unwrap();
        store.save(target("b3", None)).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let reopened = PairedKernels::new(path);
        let all = reopened.all().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(reopened.find("b3").unwrap().relay_token, None);
        assert!(reopened.remove("b3").unwrap());
        assert!(!reopened.remove("b3").unwrap());
        assert_eq!(reopened.all().unwrap().len(), 1);
    }
}
