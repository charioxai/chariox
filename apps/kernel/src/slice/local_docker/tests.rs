use super::*;
use crate::slice::{CreateSliceInput, SliceOperationStatus, SliceStore};

fn test_record() -> SliceRecord {
    let store = SliceStore::default();
    store
        .create(
            "kernel-1",
            "machine-1",
            CreateSliceInput {
                name: "dev".to_string(),
                backend: SliceBackendKind::LocalDocker,
                os: "linux".to_string(),
                display_mode: SliceDisplayMode::Headed,
                workspace_id: None,
                worktree_id: None,
                workspace_mount: Some("/repo".to_string()),
                worker_kernel_ref: None,
                display_url: None,
                provider_auth: Vec::new(),
                from_saved_state: None,
                now_ms: 42,
            },
        )
        .expect("slice should create")
}

fn test_options() -> LocalDockerSliceOptions {
    LocalDockerSliceOptions {
        root: std::env::temp_dir(),
        docker_image: "chariox-slice-linux:test".to_string(),
        build_image: SliceImageBuildPolicy::Never,
        extension_dockerfile: None,
        allow_unconfined_seccomp: false,
        memory_mb: None,
        cpus: None,
        screen_width: 1280,
        screen_height: 800,
        saved_home_archive: None,
    }
}

fn test_root(label: &str) -> std::path::PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be available")
        .as_nanos();
    std::env::temp_dir().join(format!("chariox-{label}-{unique}"))
}

fn saved_state(manifest_path: String) -> SliceSavedStateRecord {
    SliceSavedStateRecord {
        id: "gmail-ready".to_string(),
        slice_name: "gmail-ready".to_string(),
        source_slice_id: "slice-1".to_string(),
        backend: SliceBackendKind::LocalDocker,
        os: "linux".to_string(),
        image_ref: "chariox-slice-state:gmail-ready".to_string(),
        home_archive_path: "/tmp/gmail-ready-home.tar.zst".to_string(),
        manifest_path,
        created_at_ms: 1000,
        updated_at_ms: 2000,
        size_bytes: Some(4096),
        last_operation: Some("state.save".to_string()),
        last_operation_status: Some(SliceOperationStatus::Completed),
        last_error: None,
    }
}

#[test]
fn linux_docker_slice_provisioner_validation_requires_an_existing_file() {
    let root = test_root("slice-provisioner");
    std::fs::create_dir_all(&root).expect("test root should be created");
    let script = root.join("provision.sh");
    std::fs::write(&script, "#!/usr/bin/env bash\n").expect("script should be written");

    assert_eq!(
        validate_linux_docker_slice_script(script.clone())
            .expect("existing provisioner should resolve"),
        script
    );
    assert!(validate_linux_docker_slice_script(root.join("missing.sh")).is_err());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn linux_docker_slice_support_refresh_includes_runtime_dependencies() {
    let script = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("slice-linux-docker/provision-linux-docker-slice.sh"),
    )
    .expect("slice provisioner should be readable");

    for support_file in [
        "start-runtime.sh",
        "start-providers.sh",
        "slice-screen.sh",
        "browser-cdp.mjs",
        "provider-port-bridge.mjs",
        "validate-screen.sh",
    ] {
        assert!(
            script.contains(&format!("docker/{support_file}")),
            "slice support refresh must copy {support_file}"
        );
    }
}

#[test]
fn linux_docker_headed_browser_trusts_the_local_terminal_origin() {
    let script = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("slice-linux-docker/docker/slice-screen.sh"),
    )
    .expect("slice screen script should be readable");

    assert!(script.contains(
        "CHARIOX_SLICE_CHROME_TRUSTED_INSECURE_ORIGINS:-http://host.docker.internal:4321"
    ));
    assert!(script.contains("--unsafely-treat-insecure-origin-as-secure="));
}

#[test]
fn linux_docker_slice_auto_build_refreshes_protocol_or_runtime_incompatible_workers() {
    let script = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("slice-linux-docker/provision-linux-docker-slice.sh"),
    )
    .expect("slice provisioner should be readable");
    let dockerfile = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("slice-linux-docker/docker/Dockerfile"),
    )
    .expect("slice Dockerfile should be readable");

    assert!(script.contains("io.chariox.relay-peer-protocol-version"));
    assert!(script.contains("io.chariox.runtime-source-revision"));
    assert!(script.contains("refresh_saved_state_runtime"));
    assert!(script.contains("preserving saved state image"));
    assert!(script.contains(
        "saved state image $SLICE_IMAGE is missing; restoring the saved home archive on $SLICE_BASE_IMAGE"
    ));
    assert!(script.contains("git rev-parse --is-inside-work-tree"));
    assert!(script.contains("runtime image $SLICE_IMAGE is stale and build policy is never"));
    assert!(script.contains("because its worker image is stale"));
    assert!(dockerfile.contains("io.chariox.relay-peer-protocol-version"));
    assert!(dockerfile.contains("io.chariox.runtime-source-revision"));
}

#[test]
fn local_docker_slice_runtime_uses_loopback_provider_bind_host() {
    let record = test_record();
    let options = test_options();
    let mut command = Command::new("slice-provisioner");

    configure_local_docker_slice_command(&mut command, &record, None, &options).unwrap();

    let provider_bind_host = command
        .get_envs()
        .find_map(|(key, value)| {
            (key == "CHARIOX_SLICE_PROVIDER_BIND_HOST")
                .then(|| value.and_then(|value| value.to_str()))
                .flatten()
        })
        .expect("provider bind host should be configured");
    assert_eq!(provider_bind_host, "127.0.0.1");
}

#[test]
fn local_docker_default_saved_state_round_trips_through_pointer_manifest() {
    let root = test_root("slice-default-state");
    let state_dir = root.join("states").join("gmail-ready");
    std::fs::create_dir_all(&state_dir).expect("state dir should be created");
    let manifest_path = state_dir.join("manifest.json");
    let state = saved_state(manifest_path.display().to_string());
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&state).expect("state should encode"),
    )
    .expect("state manifest should write");
    let options = LocalDockerSliceOptions {
        root: root.clone(),
        ..test_options()
    };

    set_local_docker_default_saved_state(&state, &options).expect("default pointer should write");
    let resolved =
        default_local_docker_saved_state(&options, SliceBackendKind::LocalDocker, "linux")
            .expect("default pointer should resolve")
            .expect("default state should exist");

    assert_eq!(resolved.id, "gmail-ready");
    assert_eq!(resolved.manifest_path, state.manifest_path);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn local_docker_slice_runtime_starts_desktop_for_headless_slices() {
    let store = SliceStore::default();
    let record = store
        .create(
            "kernel-1",
            "machine-1",
            CreateSliceInput {
                name: "dev".to_string(),
                backend: SliceBackendKind::LocalDocker,
                os: "linux".to_string(),
                display_mode: SliceDisplayMode::Headless,
                workspace_id: None,
                worktree_id: None,
                workspace_mount: Some("/repo".to_string()),
                worker_kernel_ref: None,
                display_url: None,
                provider_auth: Vec::new(),
                from_saved_state: None,
                now_ms: 42,
            },
        )
        .expect("headless slice should create");
    let options = test_options();
    let mut command = Command::new("slice-provisioner");

    configure_local_docker_slice_command(&mut command, &record, None, &options).unwrap();

    let envs: std::collections::BTreeMap<_, _> = command
        .get_envs()
        .filter_map(|(key, value)| Some((key.to_str()?, value?.to_str()?)))
        .collect();
    assert_eq!(envs.get("CHARIOX_SLICE_DISPLAY_MODE"), Some(&"headless"));
    assert_eq!(envs.get("CHARIOX_SLICE_START_DESKTOP"), Some(&"1"));
}

#[test]
fn local_docker_slice_runtime_projects_shared_relay_env() {
    let record = test_record();
    let options = test_options();
    let relay = LocalDockerSliceRelay {
        relay_url: "wss://relay.example.test".to_string(),
        container_relay_url: Some("wss://relay.example.test".to_string()),
        relay_token: "shared-token".to_string(),
        cloud_relay_config_json: None,
    };
    let mut command = Command::new("slice-provisioner");

    configure_local_docker_slice_command(&mut command, &record, Some(relay), &options).unwrap();

    let envs: std::collections::BTreeMap<_, _> = command
        .get_envs()
        .filter_map(|(key, value)| Some((key.to_str()?, value?.to_str()?)))
        .collect();
    assert_eq!(
        envs.get("CHARIOX_SLICE_RELAY_URL"),
        Some(&"wss://relay.example.test")
    );
    assert_eq!(envs.get("CHARIOX_SLICE_RELAY_TOKEN"), Some(&"shared-token"));
}

#[test]
fn local_docker_slice_runtime_keeps_private_relay_url_unset_for_container() {
    let record = test_record();
    let options = test_options();
    let relay = LocalDockerSliceRelay {
        relay_url: "ws://127.0.0.1:43130".to_string(),
        container_relay_url: None,
        relay_token: "slice-local-token".to_string(),
        cloud_relay_config_json: None,
    };
    let mut command = Command::new("slice-provisioner");

    configure_local_docker_slice_command(&mut command, &record, Some(relay), &options).unwrap();

    let envs: std::collections::BTreeMap<_, _> = command
        .get_envs()
        .filter_map(|(key, value)| Some((key.to_str()?, value?.to_str()?)))
        .collect();
    assert!(!envs.contains_key("CHARIOX_SLICE_RELAY_URL"));
    assert_eq!(
        envs.get("CHARIOX_SLICE_RELAY_TOKEN"),
        Some(&"slice-local-token")
    );
}
