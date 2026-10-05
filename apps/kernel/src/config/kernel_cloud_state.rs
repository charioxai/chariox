//! Cloud authority is private to one kernel, including an explicit unlink tombstone.
use super::private_file::write_private_file;
use super::{DaemonConfig, PersistedCloudRelayProfile};
use crate::error::DaemonError;
use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
struct KernelCloudState {
    kernel_id: String,
    profile: Option<PersistedCloudRelayProfile>,
    relay_url: Option<String>,
    relay_token: Option<String>,
}
impl DaemonConfig {
    fn kernel_cloud_state_path(&self) -> std::path::PathBuf {
        self.private_runtime_state_root().join("cloud-relay.json")
    }
    pub(super) fn kernel_cloud_state_exists(&self) -> bool {
        self.kernel_cloud_state_path().exists()
    }
    pub(super) fn load_kernel_cloud_state(&mut self) {
        let path = self.kernel_cloud_state_path();
        match std::fs::read(&path) {
            Ok(bytes) => {
                let state: KernelCloudState = serde_json::from_slice(&bytes).unwrap_or_else(|_| {
                    panic!("invalid kernel-private Cloud state at {}", path.display())
                });
                if state.kernel_id != self.daemon_id
                    || state
                        .profile
                        .as_ref()
                        .and_then(|p| p.kernel_id.as_deref())
                        .is_some_and(|id| id != self.daemon_id)
                {
                    panic!("Cloud credential belongs to another kernel; use a separate state root");
                }
                self.cloud_relay = state.profile;
                self.relay_url = state.relay_url;
                self.relay_token = state.relay_token;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => panic!(
                "cannot read kernel-private Cloud state at {}",
                path.display()
            ),
        }
    }
    pub(super) fn persist_kernel_cloud_state(&self) -> Result<(), DaemonError> {
        let operation = "persist kernel-private Cloud state";
        let path = self.kernel_cloud_state_path();
        if path.exists() {
            let existing: KernelCloudState =
                serde_json::from_slice(&std::fs::read(&path).map_err(|e| {
                    DaemonError::LocalTransport {
                        operation,
                        message: e.to_string(),
                    }
                })?)
                .map_err(|_| DaemonError::LocalTransport {
                    operation,
                    message: "invalid kernel-private Cloud state".into(),
                })?;
            if existing.kernel_id != self.daemon_id {
                return Err(DaemonError::LocalTransport {
                    operation,
                    message:
                        "Cloud credential belongs to another kernel; use a separate state root"
                            .into(),
                });
            }
        }
        let parent = path.parent().expect("kernel state parent");
        std::fs::create_dir_all(parent).map_err(|e| DaemonError::LocalTransport {
            operation,
            message: e.to_string(),
        })?;
        let bytes = serde_json::to_vec_pretty(&KernelCloudState {
            kernel_id: self.daemon_id.clone(),
            profile: self.cloud_relay.clone(),
            relay_url: self.relay_url.clone(),
            relay_token: self.relay_token.clone(),
        })
        .map_err(|e| DaemonError::LocalTransport {
            operation,
            message: e.to_string(),
        })?;
        write_private_file(&path, &bytes).map_err(|e| DaemonError::LocalTransport {
            operation,
            message: e.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn kernel_cloud_state_is_private_independent_and_unlink_is_durable() {
        let root = std::env::temp_dir().join(format!(
            "chariox-kernel-cloud-store-{}",
            rand::random::<u64>()
        ));
        let make = |id: &str| {
            let mut config = DaemonConfig::new(id, "machine-fixture", "fixture");
            config.user_config.state.path =
                Some(root.join(id).join("state.db").to_string_lossy().to_string());
            config
        };
        let mut a = make("a");
        a.cloud_relay = Some(PersistedCloudRelayProfile {
            kernel_id: Some("a".into()),
            kernel_credential: Some("synthetic-grant-a".into()),
            ..Default::default()
        });
        a.persist_kernel_cloud_state().unwrap();
        let mut b = make("b");
        b.load_kernel_cloud_state();
        assert!(b.cloud_relay.is_none());
        b.cloud_relay = Some(PersistedCloudRelayProfile {
            kernel_id: Some("b".into()),
            kernel_credential: Some("synthetic-grant-b".into()),
            ..Default::default()
        });
        b.persist_kernel_cloud_state().unwrap();
        let mut restarted = make("a");
        restarted.load_kernel_cloud_state();
        assert_eq!(
            restarted.cloud_relay.unwrap().kernel_id.as_deref(),
            Some("a")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(a.kernel_cloud_state_path())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        a.cloud_relay = None;
        a.relay_url = None;
        a.relay_token = None;
        a.persist_kernel_cloud_state().unwrap();
        let mut restarted = make("a");
        restarted.cloud_relay = b.cloud_relay.clone(); // legacy fallback must not resurrect authority
        restarted.load_kernel_cloud_state();
        assert!(restarted.cloud_relay.is_none());
        b.load_kernel_cloud_state();
        assert_eq!(b.cloud_relay.unwrap().kernel_id.as_deref(), Some("b"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
