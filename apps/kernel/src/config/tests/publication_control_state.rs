use super::*;

struct TestRoot(std::path::PathBuf);

impl TestRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "chariox-publication-control-{:032x}",
            rand::random::<u128>()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn config(&self) -> DaemonConfig {
        let mut config = DaemonConfig::new("publication-kernel", "publication-machine", "tester");
        config.user_config_path = self.0.join("private/config.toml");
        config.publication_control_state_root = Some(self.0.join("control"));
        config
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn publication_control_state_retains_runtime_history_without_account_or_transfer_state() {
    let root = TestRoot::new();
    let mut config = root.config();
    // Deployment control-state configuration takes precedence over ordinary
    // ephemeral history/store settings, without moving private account homes.
    config.user_config.state.path = Some(root.0.join("private/state.db").display().to_string());
    config.user_config.history.operational.path =
        Some(root.0.join("private/history.db").display().to_string());
    config.user_config.artifacts.operational.root =
        Some(root.0.join("private/artifacts").display().to_string());
    config.user_config.artifacts.operational.index_path =
        Some(root.0.join("private/index.db").display().to_string());
    config
        .validate()
        .expect("separate publication control root");
    assert_eq!(config.durable_state_path(), root.0.join("control/state.db"));
    assert_eq!(
        config.workflow_runtime_artifact_root(),
        root.0.join("control/workflow-runtime")
    );
    assert_eq!(
        config.workflow_code_artifact_root(),
        root.0.join("control/workflow-code")
    );
    assert_eq!(
        config.workflow_registry_root(),
        root.0.join("control/workflows")
    );
    assert_eq!(
        config.session_history_root(),
        root.0.join("control/sessions")
    );
    assert_eq!(
        config.operational_history_path(),
        root.0.join("control/history/operational.db")
    );
    assert_eq!(
        config.operational_artifact_root(),
        root.0.join("control/artifacts")
    );
    assert_eq!(
        config.operational_artifact_index_path(),
        root.0.join("control/artifacts/index.db")
    );
    assert_eq!(
        config.kernel_event_counter_path(),
        root.0
            .join("control/kernel-events/publication-kernel/event-counter.json")
    );
    assert_eq!(
        config.kernel_relay_event_counter_path(),
        root.0
            .join("control/kernel-events/publication-kernel/relay-event-counter.json")
    );
    assert_eq!(
        config.kernel_prompt_counter_path(),
        root.0
            .join("control/kernel-events/publication-kernel/prompt-counter.json")
    );
    assert_eq!(
        config.account_profile_registry_path(),
        root.0
            .join("private/kernels/publication-kernel/provider-accounts.json")
    );
    assert_eq!(
        config.private_runtime_state_root(),
        root.0.join("private/kernels/publication-kernel")
    );
}

#[test]
fn ordinary_kernel_state_paths_keep_the_existing_layout() {
    let root = TestRoot::new();
    let mut config = root.config();
    config.publication_control_state_root = None;
    config.user_config.state.path = Some(root.0.join("existing/state.db").display().to_string());
    config = config.with_session_history_root(root.0.join("existing/sessions"));
    assert_eq!(
        config.account_profile_registry_path(),
        root.0.join("existing/provider-accounts.json")
    );
    assert_eq!(config.private_runtime_state_root(), root.0.join("existing"));
    assert_eq!(
        config.session_history_root(),
        root.0.join("existing/sessions")
    );
}

#[test]
fn publication_control_state_rejects_relative_or_overlapping_roots() {
    let root = TestRoot::new();
    let mut config = root.config();
    for path in [
        std::path::PathBuf::new(),
        std::path::PathBuf::from("relative"),
        root.0.ancestors().last().unwrap().to_path_buf(),
        root.0.clone(),
        root.0.join("private"),
        root.0.join("private/retained"),
        root.0.join("control/../private"),
    ] {
        config.publication_control_state_root = Some(path.clone());
        assert!(
            matches!(
                config.validate(),
                Err(DaemonError::InvalidConfig {
                    field: "publication_control_state_root",
                    ..
                })
            ),
            "unsafe control root accepted: {}",
            path.display()
        );
    }
}

#[cfg(unix)]
#[test]
fn publication_control_state_rejects_symlinks_and_aliased_private_roots() {
    let root = TestRoot::new();
    let mut config = root.config();
    std::fs::create_dir(root.0.join("private")).unwrap();
    std::os::unix::fs::symlink(root.0.join("private"), root.0.join("alias")).unwrap();
    for path in [root.0.join("alias"), root.0.join("alias/nested")] {
        config.publication_control_state_root = Some(path);
        assert!(config.validate().is_err());
    }
    std::fs::write(root.0.join("file"), "not a directory").unwrap();
    config.publication_control_state_root = Some(root.0.join("file"));
    assert!(config.validate().is_err());
    std::os::unix::fs::symlink(root.0.join("missing"), root.0.join("dangling")).unwrap();
    config.publication_control_state_root = Some(root.0.join("dangling/nested"));
    assert!(config.validate().is_err());
}

#[test]
fn publication_control_state_requires_path_safe_kernel_identity() {
    let root = TestRoot::new();
    let mut config = root.config();
    for id in [
        "../control",
        "/absolute",
        ".",
        "..",
        "parent/child",
        "parent\\child",
    ] {
        config.daemon_id = id.to_string();
        assert!(config.validate().is_err(), "unsafe kernel identity: {id}");
    }
}
