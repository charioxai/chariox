//! Cloud authority is private to one kernel, including an explicit unlink tombstone.
use super::private_file::write_private_file;
use super::{DaemonConfig, PersistedCloudRelayProfile};
use crate::error::DaemonError;
use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
struct KernelCloudState {
    kernel_id: String,
    profile: Option<PersistedCloudRelayProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    former_cloud_kernel_id: Option<String>,
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
    pub(super) fn try_load_kernel_cloud_state(
        &mut self,
        relay_override: bool,
    ) -> Result<(), DaemonError> {
        self.read_kernel_cloud_state(relay_override)
            .map_err(|message| DaemonError::LocalTransport {
                operation: "load kernel-private Cloud state",
                message,
            })
    }
    fn read_kernel_cloud_state(&mut self, relay_override: bool) -> Result<(), String> {
        let path = self.kernel_cloud_state_path();
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => {
                return Err(format!(
                    "cannot read kernel-private Cloud state at {}",
                    path.display()
                ))
            }
        };
        let state: KernelCloudState = serde_json::from_slice(&bytes)
            .map_err(|_| format!("invalid kernel-private Cloud state at {}", path.display()))?;
        let foreign_kernel_id = std::iter::once(state.kernel_id.as_str())
            .chain(state.profile.as_ref().and_then(|p| p.kernel_id.as_deref()))
            .find(|id| *id != self.daemon_id);
        if let Some(id) = foreign_kernel_id {
            return Err(format!(
                "This home's Cloud link belongs to kernel {id:?}, and this address starts kernel {:?}. Start it at its original address, or use a new kernel home and link it to Cloud.",
                self.daemon_id,
            ));
        }
        self.persisted_cloud_kernel_id = state
            .profile
            .as_ref()
            .and_then(PersistedCloudRelayProfile::enrolled_kernel_id)
            .map(str::to_owned);
        self.former_cloud_kernel_id = state.former_cloud_kernel_id;
        if !relay_override {
            self.cloud_relay = state.profile;
            self.relay_url = state.relay_url;
            self.relay_token = state.relay_token;
        }
        Ok(())
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
            former_cloud_kernel_id: self.former_cloud_kernel_id.clone(),
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
    fn config_loading_refuses_a_cloud_link_at_a_different_address() {
        crate::test_support::isolated_env_test!();
        let home = crate::test_support::TestTempDir::new("foreign-cloud-link");
        std::env::set_var("CHARIOX_HOME", home.path());
        std::env::set_var("CHARIOX_KERNEL_HOST", "127.0.0.1");
        std::env::set_var("CHARIOX_KERNEL_PORT", "43118");
        std::fs::write(
            home.join("config.toml"),
            "[state]\npath = \"~/.chariox/shared/state.db\"\n",
        )
        .unwrap();
        let mut original = DaemonConfig::load_from_env().unwrap();
        original.cloud_relay = Some(PersistedCloudRelayProfile {
            kernel_id: Some(original.daemon_id.clone()),
            ..Default::default()
        });
        original.persist_kernel_cloud_state().unwrap();
        assert_eq!(
            DaemonConfig::load_from_env().unwrap().cloud_relay,
            original.cloud_relay
        );

        std::env::set_var("CHARIOX_KERNEL_PORT", "43119");
        let error = DaemonConfig::load_from_env().err().unwrap().to_string();
        assert!(error.contains(&original.daemon_id));
        assert!(error.contains("Start it at its original address"));
    }

    #[test]
    fn mp08_mp11_relay_override_persistence_preserves_kernel_enrollment() {
        let root = crate::test_support::TestWorktree::new("relay-override-enrollment");
        let mut config = DaemonConfig::for_tests();
        config.user_config.state.path =
            Some(root.path().join("kernel/state.db").display().to_string());
        let profile = PersistedCloudRelayProfile {
            kernel_id: Some(config.daemon_id.clone()),
            kernel_credential: Some("synthetic-kernel-grant".into()),
            ..Default::default()
        };
        config.cloud_relay = Some(profile.clone());
        config.persist_kernel_cloud_state().unwrap();
        config.cloud_relay = None; // The same state as an explicit environment relay override.
        config.relay_url = Some("ws://127.0.0.1:47000".into());
        config.relay_token = Some("synthetic-private-relay".into());
        config.persist_relay_config().unwrap();
        let mut restarted = config.clone();
        restarted.try_load_kernel_cloud_state(false).unwrap();
        assert_eq!(restarted.cloud_relay, Some(profile));
        assert_eq!(restarted.relay_url, config.relay_url);
    }

    #[test]
    fn mp08_mp11_relay_override_persistence_rejects_corrupt_or_foreign_state_without_panic() {
        for (name, contents) in [
            ("corrupt", b"{not json".to_vec()),
            ("foreign", br#"{"kernel_id":"other-kernel","profile":null,"relay_url":null,"relay_token":null}"#.to_vec()),
        ] {
            let root = crate::test_support::TestWorktree::new(&format!("relay-override-{name}"));
            let mut config = DaemonConfig::for_tests();
            config.user_config.state.path =
                Some(root.path().join("kernel/state.db").display().to_string());
            let path = config.kernel_cloud_state_path();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &contents).unwrap();
            config.relay_url = Some("ws://127.0.0.1:47000".into());
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                config.persist_relay_config()
            }));
            assert!(matches!(result, Ok(Err(_))), "{name} state must return DaemonError, not panic");
            assert_eq!(std::fs::read(&path).unwrap(), contents);
        }
    }

    #[test]
    fn kernel_map_loads_cloud_identity_with_an_explicit_relay_override() {
        crate::test_support::isolated_env_test!();
        let _env = crate::env_lock::lock();
        let root = crate::test_support::TestTempDir::new("kernel-map-override");
        std::env::set_var("CHARIOX_HOME", root.path());
        std::env::set_var("CHARIOX_DAEMON_ID", "fixture-kernel");
        std::env::remove_var("CHARIOX_RELAY_URL");
        std::env::remove_var("CHARIOX_RELAY_TOKEN");
        std::env::remove_var("CHARIOX_CLOUD_RELAY_CONFIG_JSON");

        let mut enrolled = DaemonConfig::load_from_env().unwrap();
        enrolled.relay_url = Some("wss://cloud.example.invalid".into());
        enrolled.relay_token = Some("synthetic-cloud-relay".into());
        enrolled
            .persist_cloud_relay_profile(Some(PersistedCloudRelayProfile {
                kernel_id: Some("fixture-kernel".into()),
                kernel_credential: Some("synthetic-kernel-grant".into()),
                ..Default::default()
            }))
            .unwrap();

        std::env::set_var("CHARIOX_RELAY_URL", "ws://127.0.0.1:2");
        std::env::set_var("CHARIOX_RELAY_TOKEN", "synthetic-private-relay");
        let restarted = DaemonConfig::load_from_env().unwrap();
        assert_eq!(restarted.enrolled_cloud_kernel_id(), Some("fixture-kernel"));
        assert!(restarted.cloud_relay.is_none());
        assert_eq!(restarted.relay_url.as_deref(), Some("ws://127.0.0.1:2"));
        assert_eq!(
            restarted.relay_token.as_deref(),
            Some("synthetic-private-relay")
        );

        enrolled.persist_cloud_relay_profile(None).unwrap();
        assert!(enrolled.enrolled_cloud_kernel_id().is_none());
        let unlinked = DaemonConfig::load_from_env().unwrap();
        assert!(unlinked.enrolled_cloud_kernel_id().is_none());
        assert_eq!(
            unlinked.former_cloud_kernel_id.as_deref(),
            Some("fixture-kernel")
        );
        assert!(unlinked.cloud_relay.is_none());
        assert_eq!(unlinked.relay_url.as_deref(), Some("ws://127.0.0.1:2"));
    }

    #[test]
    fn kernel_cloud_state_is_private_independent_and_unlink_is_durable() {
        crate::test_support::isolated_env_test!();
        let _env = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-kernel-cloud-store-{}",
            rand::random::<u64>()
        ));
        std::env::set_var("CHARIOX_HOME", &root);
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
        b.try_load_kernel_cloud_state(false).unwrap();
        assert!(b.cloud_relay.is_none());
        b.cloud_relay = Some(PersistedCloudRelayProfile {
            kernel_id: Some("b".into()),
            kernel_credential: Some("synthetic-grant-b".into()),
            ..Default::default()
        });
        b.persist_kernel_cloud_state().unwrap();
        let mut restarted = make("a");
        restarted.try_load_kernel_cloud_state(false).unwrap();
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
        a.persist_cloud_relay_profile(None).unwrap();
        a.relay_url = None;
        a.relay_token = None;
        a.persist_kernel_cloud_state().unwrap();
        let mut restarted = make("a");
        restarted.cloud_relay = b.cloud_relay.clone(); // legacy fallback must not resurrect authority
        restarted.try_load_kernel_cloud_state(false).unwrap();
        assert!(restarted.cloud_relay.is_none());
        assert_eq!(restarted.former_cloud_kernel_id.as_deref(), Some("a"));
        b.try_load_kernel_cloud_state(false).unwrap();
        assert_eq!(b.cloud_relay.unwrap().kernel_id.as_deref(), Some("b"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
