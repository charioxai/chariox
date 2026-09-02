use super::*;
use crate::slice::{CreateSliceInput, SliceOperationStatus, SliceStore};

#[test]
fn selected_broker_credential_replaces_default_and_missing_selection_clears_it() {
    let mut inputs = vec![broker::ProvisionerInput {
        environment: "CHARIOX_SLICE_CODEX_AUTH",
        name: "codex-auth.json",
        contents: zeroize::Zeroizing::new(b"default".to_vec()),
    }];
    replace_broker_input(
        &mut inputs,
        "CHARIOX_SLICE_CODEX_AUTH",
        "codex-auth.json",
        Some(zeroize::Zeroizing::new(b"selected".to_vec())),
    )
    .expect("replace default credential");
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].contents.as_slice(), b"selected");

    replace_broker_input(
        &mut inputs,
        "CHARIOX_SLICE_CODEX_AUTH",
        "codex-auth.json",
        None,
    )
    .expect("clear missing selected credential");
    assert!(inputs.is_empty());
}

#[cfg(unix)]
#[test]
fn optional_provider_credential_path_ignores_missing_parents_but_rejects_symlinks() {
    use std::os::unix::fs::symlink;

    let root = test_root("missing-provider-credential-parent");
    std::fs::create_dir_all(&root).expect("fixture root should create");
    let root = std::fs::canonicalize(root).expect("fixture root should canonicalize");
    let missing = root.join(".local/share/opencode/auth.json");

    assert_eq!(
        read_provider_credential_no_symlinks(&missing)
            .expect("an absent optional credential should not fail the import"),
        None
    );

    let credential_root = root.join("managed-opencode");
    std::fs::create_dir_all(&credential_root).expect("credential root should create");
    std::fs::write(credential_root.join("auth.json"), b"secret")
        .expect("credential fixture should write");
    symlink(&credential_root, root.join("opencode-link"))
        .expect("credential symlink should create");
    assert!(
        read_provider_credential_no_symlinks(&root.join("opencode-link/auth.json")).is_err(),
        "a symlinked credential parent must remain fatal"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn github_token_probe_is_bounded_and_reaps_a_stalled_helper() {
    use std::os::unix::fs::PermissionsExt;

    let root = test_root("github-token-timeout");
    std::fs::create_dir_all(&root).expect("fixture root should create");
    let success = root.join("gh-success");
    std::fs::write(&success, "#!/bin/sh\nprintf 'github-token\\n'\n")
        .expect("success helper should write");
    std::fs::set_permissions(&success, std::fs::Permissions::from_mode(0o700))
        .expect("success helper should be executable");
    let token = bounded_github_token(&success, Duration::from_secs(1))
        .expect("bounded helper should return a token");
    assert_eq!(token.as_slice(), b"github-token\n");

    let stalled = root.join("gh-stalled");
    std::fs::write(&stalled, "#!/bin/sh\nsleep 30\n").expect("stalled helper should write");
    std::fs::set_permissions(&stalled, std::fs::Permissions::from_mode(0o700))
        .expect("stalled helper should be executable");
    let started = std::time::Instant::now();
    assert!(bounded_github_token(&stalled, Duration::from_millis(50)).is_none());
    assert!(started.elapsed() < Duration::from_secs(3));

    let _ = std::fs::remove_dir_all(root);
}

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
                display_backend: Default::default(),
                workspace_id: None,
                worktree_id: None,
                workspace_mount: Some("/repo".to_string()),
                development: None,
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
        home_public_key: DaemonConfig::for_tests().relay_public_key,
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
        "browser-controller-actions.mjs",
        "browser-controller-cdp.mjs",
        "browser-controller-events.mjs",
        "browser-controller-files.mjs",
        "browser-controller-permissions.mjs",
        "browser-controller-snapshot.mjs",
        "browser-controller.mjs",
        "managed-provider-isolation-probe.mjs",
        "managed-provider-isolation-probe-wrapper.sh",
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
fn linux_docker_browser_controller_is_private_and_kernel_owned() {
    let docker_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("slice-linux-docker/docker");
    let runtime = std::fs::read_to_string(docker_root.join("start-runtime.sh"))
        .expect("slice runtime script should be readable");
    let screen = std::fs::read_to_string(docker_root.join("slice-screen.sh"))
        .expect("slice screen script should be readable");
    let controller = std::fs::read_to_string(docker_root.join("browser-controller.mjs"))
        .expect("browser controller should be readable");

    assert!(runtime.contains("CHARIOX_BROWSER_CONTROLLER_SCRIPT=\"$ROOT/browser-controller.mjs\""));
    assert!(runtime.contains("CHARIOX_BROWSER_DOWNLOAD_DIR=\"$BROWSER_DOWNLOAD_DIR\""));
    assert!(runtime.contains("CHARIOX_BROWSER_UPLOAD_ROOTS=\"$BROWSER_UPLOAD_ROOTS\""));
    assert!(!screen.contains("browser-controller-start"));
    assert!(!screen.contains("browser-controller-status"));
    assert!(controller.contains("BrowserControllerStdioServer"));
    assert!(!controller.contains(".listen("));
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
fn linux_docker_pointer_click_preserves_desktop_focus_and_button() {
    let root = test_root("slice-pointer-click");
    let bin = root.join("bin");
    let home = root.join("home");
    std::fs::create_dir_all(&bin).expect("stub bin should be created");
    std::fs::create_dir_all(&home).expect("stub home should be created");
    let write_executable = |name: &str, contents: &str| {
        let path = bin.join(name);
        std::fs::write(&path, contents).expect("stub should be written");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
                .expect("stub should be executable");
        }
    };
    write_executable("xdpyinfo", "#!/bin/sh\nexit 0\n");
    write_executable("pgrep", "#!/bin/sh\nprintf '1 process\\n'\n");
    write_executable("timeout", "#!/bin/sh\nshift\nexec \"$@\"\n");
    write_executable(
        "xdotool",
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$CHARIOX_XDOTOOL_LOG\"\n",
    );
    let xdotool_log = root.join("xdotool.log");
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let output = Command::new("bash")
        .arg(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("slice-linux-docker/docker/slice-screen.sh"),
        )
        .args(["pointer-click", "320", "180", "right", "2"])
        .env("PATH", path)
        .env("HOME", &home)
        .env("CHARIOX_SLICE_ROOT", root.join("runtime"))
        .env("CHARIOX_XDOTOOL_LOG", &xdotool_log)
        .output()
        .expect("pointer helper should run");

    assert!(
        output.status.success(),
        "pointer helper failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&xdotool_log).expect("xdotool call should be logged"),
        "mousemove 320 180 click --repeat 2 --delay 80 3\n"
    );
    std::fs::remove_dir_all(root).expect("test root should be removed");
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
    assert!(script.contains("CHARIOX_SLICE_BUILD_CONTEXT_DIGEST"));
    assert!(script.contains("^sha256:[a-f0-9]{64}$"));
    assert!(script.contains("refresh_saved_state_runtime"));
    assert!(script.contains("preserving saved state image"));
    assert!(script.contains(
        "saved state image $SLICE_IMAGE is missing; restoring the saved home archive on $SLICE_BASE_IMAGE"
    ));
    assert!(script.contains("git rev-parse --is-inside-work-tree"));
    assert!(script.contains("Cargo.toml Cargo.lock"));
    assert!(script.contains("adapters/rust"));
    assert!(script.contains("apps/aegs-dummy apps/kernel apps/relay"));
    assert!(script.contains("packages/aegs-sdk packages/event-protocol"));
    assert!(!script.contains("grep -v '^apps/kernel/slice-linux-docker/'"));
    assert!(script.contains("packages/event-protocol"));
    assert!(dockerfile.contains("COPY packages/event-protocol packages/event-protocol"));
    assert!(dockerfile.contains("COPY Cargo.toml Cargo.lock ./"));
    assert!(dockerfile.contains("cargo build --locked --release"));
    assert!(dockerfile.contains("npm ci --omit=dev"));
    assert!(dockerfile.contains("snapshot.debian.org/archive/debian/20260701T000000Z"));
    assert!(!dockerfile.contains("npm install -g"));
    assert!(!dockerfile.contains("rustup.rs"));
    assert!(!dockerfile.contains("deb.nodesource.com"));
    for base in dockerfile.lines().filter(|line| line.starts_with("FROM ")) {
        assert!(
            base.contains("@sha256:"),
            "unpinned slice base image: {base}"
        );
    }
    assert!(script.contains("runtime image $SLICE_IMAGE is stale and build policy is never"));
    assert!(script.contains("because its worker image is stale"));
    assert!(dockerfile.contains("io.chariox.relay-peer-protocol-version"));
    assert!(dockerfile.contains("io.chariox.runtime-source-revision"));
}

#[cfg(unix)]
#[test]
fn managed_broker_stream_is_close_on_exec_for_provider_children() {
    use std::os::fd::AsRawFd;
    let (stream, _peer) = std::os::unix::net::UnixStream::pair().expect("broker stream pair");
    let flags = unsafe { libc::fcntl(stream.as_raw_fd(), libc::F_GETFD) };
    assert!(flags >= 0);
    assert_eq!(
        unsafe { libc::fcntl(stream.as_raw_fd(), libc::F_SETFD, flags & !libc::FD_CLOEXEC) },
        0
    );
    assert!(!super::broker::broker_stream_is_close_on_exec(&stream));
    super::broker::mark_broker_stream_close_on_exec(&stream).expect("mark broker lease CLOEXEC");
    assert!(super::broker::broker_stream_is_close_on_exec(&stream));
}

#[test]
fn managed_slice_rust_paths_do_not_bypass_the_broker() {
    let driver = include_str!("../local_docker.rs");
    let state = include_str!("state.rs");
    assert!(!driver.contains("Command::new(\"docker\")"));
    assert!(!state.contains("Command::new(\"docker\")"));
    assert!(driver.contains("broker::run_provisioner"));
    assert!(driver.contains("/usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh"));
    assert!(driver.contains("docker_command()"));
    assert!(state.contains("docker_command()"));
    let broker = include_str!("broker.rs");
    assert!(broker.contains("remove_var(BROKER_SOCKET_ENV)"));
    assert!(broker.contains("remove_var(BROKER_FD_ENV)"));
}

#[test]
fn local_docker_slice_runtime_uses_loopback_provider_bind_host() {
    let record = test_record();
    let options = test_options();
    let mut command = Command::new("slice-provisioner");

    configure_local_docker_slice_command(&mut command, &record, None, &options, true).unwrap();

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
fn local_docker_slice_compatibility_mode_probes_the_named_apparmor_boundary() {
    let _guard = crate::env_lock::lock();
    let previous_profile = std::env::var_os("CHARIOX_SLICE_APPARMOR_PROFILE");
    std::env::set_var("CHARIOX_SLICE_APPARMOR_PROFILE", "chariox-slice-provider");
    let record = test_record();
    let mut options = test_options();
    options.allow_unconfined_seccomp = true;
    let mut command = Command::new("slice-provisioner");

    configure_local_docker_slice_command(&mut command, &record, None, &options, true).unwrap();

    match previous_profile {
        Some(value) => std::env::set_var("CHARIOX_SLICE_APPARMOR_PROFILE", value),
        None => std::env::remove_var("CHARIOX_SLICE_APPARMOR_PROFILE"),
    }
    let envs: std::collections::BTreeMap<_, _> = command
        .get_envs()
        .filter_map(|(key, value)| Some((key.to_str()?, value?.to_str()?)))
        .collect();
    assert_eq!(
        envs.get("CHARIOX_SLICE_APPARMOR_PROFILE"),
        Some(&"chariox-slice-provider")
    );
    assert_eq!(
        envs.get("CHARIOX_MANAGED_PROVIDER_ISOLATION_PROBE"),
        Some(&"1")
    );
}

#[test]
fn local_docker_slice_mounts_only_development_repositories() {
    let store = SliceStore::default();
    let record = store
        .create(
            "kernel-1",
            "machine-1",
            CreateSliceInput {
                name: "project-dev".to_string(),
                backend: SliceBackendKind::SshDocker,
                os: "linux".to_string(),
                display_mode: SliceDisplayMode::Headless,
                display_backend: crate::slice::SliceDisplayBackend::default(),
                workspace_id: Some("/source/primary".to_string()),
                worktree_id: Some("/source/primary-worktree".to_string()),
                workspace_mount: Some("/source/primary-worktree".to_string()),
                development: None,
                worker_kernel_ref: None,
                display_url: None,
                provider_auth: Vec::new(),
                from_saved_state: None,
                now_ms: 42,
            },
        )
        .expect("slice should create");
    let record = store
        .set_development_publication(
            &record.id,
            crate::slice::SliceDevelopmentPublication {
                publication_id: "development".to_string(),
                destination_root: "/state/development/slice-1/development".to_string(),
                primary_repository_path: "/state/development/slice-1/development/primary"
                    .to_string(),
                repository_paths: vec![
                    "/state/development/slice-1/development/primary".to_string(),
                    "/state/development/slice-1/development/supporting".to_string(),
                ],
            },
            43,
        )
        .expect("publication should bind to slice");
    let mut command = Command::new("slice-provisioner");
    configure_local_docker_slice_command(&mut command, &record, None, &test_options(), true)
        .expect("slice command should configure");
    let envs: std::collections::BTreeMap<_, _> = command
        .get_envs()
        .filter_map(|(key, value)| Some((key.to_str()?, value?.to_str()?)))
        .collect();
    assert_eq!(
        envs.get("CHARIOX_SLICE_WORKSPACE"),
        Some(&"/state/development/slice-1/development/primary")
    );
    assert_eq!(
        envs.get("CHARIOX_SLICE_DEVELOPMENT_MOUNT_COUNT"),
        Some(&"2")
    );
    assert_eq!(
        envs.get("CHARIOX_SLICE_DEVELOPMENT_MOUNT_0"),
        Some(&"/state/development/slice-1/development/primary")
    );
    assert_eq!(
        envs.get("CHARIOX_SLICE_DEVELOPMENT_MOUNT_1"),
        Some(&"/state/development/slice-1/development/supporting")
    );
    assert!(!envs.contains_key("CHARIOX_SLICE_DEVELOPMENT_ROOT"));
    let script = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("slice-linux-docker/provision-linux-docker-slice.sh"),
    )
    .expect("slice provisioner should be readable");
    assert!(script.contains("mount_source_variable=\"${mount_variable}_SOURCE\""));
    assert!(script.contains(
        "-v \"$development_mount_source:$development_mount:$SLICE_WORKSPACE_MOUNT_MODE\""
    ));
    assert!(script
        .contains("-e \"CHARIOX_MANAGED_WORKSPACE_ROOT_COUNT=$SLICE_DEVELOPMENT_MOUNT_COUNT\""));
    assert!(
        script.contains("-e \"CHARIOX_MANAGED_WORKSPACE_ROOT_${mount_index}=$development_mount\"")
    );
    assert!(script.contains("local docker_create_args=("));
    assert!(script.contains("docker create \"${docker_create_args[@]}\" \"$SLICE_IMAGE\""));
    assert!(!script.contains("$SLICE_DEVELOPMENT_ROOT:$SLICE_DEVELOPMENT_ROOT"));
}

#[cfg(unix)]
#[test]
fn existing_slice_runtime_forwards_managed_workspace_roots() {
    use std::os::unix::fs::PermissionsExt;

    let root = test_root("existing-slice-workspace-roots");
    let bin = root.join("bin");
    let docker = bin.join("docker");
    let log = root.join("docker.log");
    std::fs::create_dir_all(&bin).expect("fake Docker directory should create");
    std::fs::write(
        &docker,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$DOCKER_LOG"
if [ "$1" = "container" ] && [ "$2" = "inspect" ]; then
  if [ "${3:-}" = "-f" ]; then
    printf 'sha256:fixture\n'
  fi
  exit 0
fi
if [ "$1" = "image" ] && [ "$2" = "inspect" ]; then
  printf 'sha256:fixture\n'
  exit 0
fi
if [ "$1" = "inspect" ] && [ "$2" = "-f" ]; then
  printf 'true\n'
  exit 0
fi
for argument in "$@"; do
  if [ "$argument" = "df" ]; then
    printf 'Filesystem 1024-blocks Used Available Capacity Mounted on\n'
    printf 'fixture 1000000 1 999999 1%% /\n'
    exit 0
  fi
done
exit 0
"#,
    )
    .expect("fake Docker should write");
    std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o700))
        .expect("fake Docker should become executable");

    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("slice-linux-docker/provision-linux-docker-slice.sh");
    let mut paths = vec![bin];
    if let Some(existing_path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing_path));
    }
    let path = std::env::join_paths(paths).expect("fake Docker PATH should join");
    let output = Command::new("bash")
        .arg(script)
        .arg("start-runtime")
        .env("PATH", path)
        .env("TMPDIR", &root)
        .env("DOCKER_LOG", &log)
        .env("CHARIOX_SLICE_NAME", "saved-slice")
        .env("CHARIOX_SLICE_DOCKER_IMAGE", "fixture")
        .env("CHARIOX_SLICE_BASE_IMAGE", "fixture")
        .env("CHARIOX_SLICE_DEVELOPMENT_MOUNT_COUNT", "2")
        .env("CHARIOX_SLICE_DEVELOPMENT_MOUNT_0", "/development/primary")
        .env(
            "CHARIOX_SLICE_DEVELOPMENT_MOUNT_1",
            "/development/supporting",
        )
        .output()
        .expect("slice runtime command should execute");
    assert!(
        output.status.success(),
        "slice runtime failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let calls = std::fs::read_to_string(&log).expect("fake Docker log should read");
    assert!(
        !calls.lines().any(|call| call.starts_with("create ")),
        "existing container must be reused: {calls}"
    );
    let runtime_call = calls
        .lines()
        .find(|call| call.ends_with(" saved-slice /opt/chariox-slice/start-runtime.sh"))
        .expect("runtime Docker exec should be recorded");
    for expected in [
        "-e CHARIOX_MANAGED_WORKSPACE_ROOT_COUNT=2",
        "-e CHARIOX_MANAGED_WORKSPACE_ROOT_0=/development/primary",
        "-e CHARIOX_MANAGED_WORKSPACE_ROOT_1=/development/supporting",
    ] {
        assert!(
            runtime_call.contains(expected),
            "runtime call is missing {expected}: {runtime_call}"
        );
    }

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn local_docker_slice_rejects_mounting_development_control_root() {
    let mut record = SliceStore::default()
        .create(
            "kernel-1",
            "machine-1",
            CreateSliceInput {
                name: "project-dev-invalid".to_string(),
                backend: SliceBackendKind::SshDocker,
                os: "linux".to_string(),
                display_mode: SliceDisplayMode::Headless,
                display_backend: crate::slice::SliceDisplayBackend::default(),
                workspace_id: Some("/source/primary".to_string()),
                worktree_id: Some("/source/primary-worktree".to_string()),
                workspace_mount: Some("/source/primary-worktree".to_string()),
                development: None,
                worker_kernel_ref: None,
                display_url: None,
                provider_auth: Vec::new(),
                from_saved_state: None,
                now_ms: 42,
            },
        )
        .expect("slice should create");
    record.development_publication = Some(crate::slice::SliceDevelopmentPublication {
        publication_id: "development".to_string(),
        destination_root: "/state/development/slice-1/development".to_string(),
        primary_repository_path: "/state/development/slice-1/development".to_string(),
        repository_paths: vec!["/state/development/slice-1/development".to_string()],
    });
    let mut command = Command::new("slice-provisioner");

    let error =
        configure_local_docker_slice_command(&mut command, &record, None, &test_options(), true)
            .expect_err("publication control root must never be mounted into the slice");

    assert!(error
        .to_string()
        .contains("repository mount escaped its publication"));
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
                display_backend: Default::default(),
                workspace_id: None,
                worktree_id: None,
                workspace_mount: Some("/repo".to_string()),
                development: None,
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

    configure_local_docker_slice_command(&mut command, &record, None, &options, true).unwrap();

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
        owner_public_key: Some("owner-public".to_string()),
        cloud_relay_config_json: None,
    };
    let mut command = Command::new("slice-provisioner");

    configure_local_docker_slice_command(&mut command, &record, Some(relay), &options, true)
        .unwrap();

    let envs: std::collections::BTreeMap<_, _> = command
        .get_envs()
        .filter_map(|(key, value)| Some((key.to_str()?, value?.to_str()?)))
        .collect();
    assert_eq!(
        envs.get("CHARIOX_SLICE_RELAY_URL"),
        Some(&"wss://relay.example.test")
    );
    assert_eq!(envs.get("CHARIOX_SLICE_RELAY_TOKEN"), Some(&"shared-token"));
    assert_eq!(
        envs.get("CHARIOX_SLICE_OWNER_PUBLIC_KEY"),
        Some(&"owner-public")
    );

    let provisioner = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("slice-linux-docker/provision-linux-docker-slice.sh"),
    )
    .expect("slice provisioner should be readable");
    let runtime = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("slice-linux-docker/docker/start-runtime.sh"),
    )
    .expect("slice runtime should be readable");
    assert!(provisioner.contains("-e CHARIOX_SLICE_OWNER_PUBLIC_KEY=\"$SLICE_OWNER_PUBLIC_KEY\""));
    assert!(runtime
        .contains("CHARIOX_MANAGED_SLICE_RELAY_OWNER_PUBLIC_KEY=\"$SLICE_OWNER_PUBLIC_KEY\""));
}

#[test]
fn local_docker_slice_runtime_keeps_private_relay_url_unset_for_container() {
    let record = test_record();
    let options = test_options();
    let relay = LocalDockerSliceRelay {
        relay_url: "ws://127.0.0.1:43130".to_string(),
        container_relay_url: None,
        relay_token: "slice-local-token".to_string(),
        owner_public_key: None,
        cloud_relay_config_json: None,
    };
    let mut command = Command::new("slice-provisioner");

    configure_local_docker_slice_command(&mut command, &record, Some(relay), &options, true)
        .unwrap();

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

#[test]
fn hosted_relay_discovery_uses_owner_metadata_credential() {
    let relay = LocalDockerSliceRelay {
        relay_url: "wss://relay.example.test".to_string(),
        container_relay_url: Some("wss://relay.example.test".to_string()),
        relay_token: "worker-bootstrap-token".to_string(),
        owner_public_key: Some("owner-public".to_string()),
        cloud_relay_config_json: None,
    };
    let mut owner_config = DaemonConfig::for_tests();
    owner_config.relay_token = Some("owner-metadata-token".to_string());

    let discovery = relay.worker_discovery_config(owner_config);

    assert!(relay.uses_shared_relay());
    assert!(!relay.uses_private_relay());
    assert_eq!(
        discovery.relay_token.as_deref(),
        Some("owner-metadata-token")
    );
}

#[test]
fn private_relay_discovery_uses_private_relay_credential() {
    let relay = LocalDockerSliceRelay {
        relay_url: "ws://127.0.0.1:43130".to_string(),
        container_relay_url: None,
        relay_token: "slice-private-token".to_string(),
        owner_public_key: None,
        cloud_relay_config_json: None,
    };
    let mut owner_config = DaemonConfig::for_tests();
    owner_config.relay_token = Some("owner-token".to_string());

    let discovery = relay.worker_discovery_config(owner_config);

    assert!(relay.uses_private_relay());
    assert!(!relay.uses_shared_relay());
    assert_eq!(
        discovery.relay_token.as_deref(),
        Some("slice-private-token")
    );
    assert!(discovery.cloud_relay.is_none());
}
