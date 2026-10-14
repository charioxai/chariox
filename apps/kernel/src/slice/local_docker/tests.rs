use super::*;
use crate::slice::{CreateSliceInput, SliceOperationStatus, SliceStore};

#[test]
fn provider_auth_inspection_paths_follow_verified_layout_and_account() {
    let account = LocalDockerProviderAccount {
        owner_path_component: "owner-synthetic".to_string(),
        profile_id: "profile-synthetic".to_string(),
        environment: Default::default(),
    };
    assert_eq!(provider_auth_paths(Some(&account),true),
        ("/var/lib/chariox/slice-private/provider-accounts/owner-synthetic/codex/profile-synthetic/codex/auth.json".to_string(),
         "/var/lib/chariox/slice-private/provider-accounts/owner-synthetic/opencode/profile-synthetic/data/opencode/auth.json".to_string()));
    assert_eq!(
        provider_auth_paths(None, true).0,
        "/var/lib/chariox/slice-private/provider-accounts/local-user/codex/default/codex/auth.json"
    );
    assert_eq!(provider_auth_paths(Some(&account),false).0,"/home/slice/.chariox/daemon/provider-accounts/owner-synthetic/codex/profile-synthetic/codex/auth.json");
    assert_eq!(
        provider_auth_paths(None, false),
        (
            "/home/slice/.codex/auth.json".to_string(),
            "/home/slice/.local/share/opencode/auth.json".to_string()
        )
    );
}

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
    let managed_home = root.join("managed-home");
    let context = crate::managed_context::scm::GitCredentialCommandContext::for_tests(
        managed_home.clone(),
        std::ffi::OsString::from("/usr/bin:/bin"),
    );
    std::fs::write(
        &success,
        format!(
            "#!/bin/sh\n[ \"$HOME\" = '{}' ] || exit 41\n[ \"$GH_CONFIG_DIR\" = '{}/.config/gh' ] || exit 42\nprintf 'github-token\\n'\n",
            managed_home.display(),
            managed_home.display()
        ),
    )
    .expect("context-aware success helper should write");
    let token = bounded_github_token(&success, Duration::from_secs(1), &context)
        .expect("bounded helper should return a token");
    assert_eq!(token.as_slice(), b"github-token\n");

    let stalled = root.join("gh-stalled");
    std::fs::write(&stalled, "#!/bin/sh\nsleep 30\n").expect("stalled helper should write");
    std::fs::set_permissions(&stalled, std::fs::Permissions::from_mode(0o700))
        .expect("stalled helper should be executable");
    let started = std::time::Instant::now();
    assert!(bounded_github_token(&stalled, Duration::from_millis(50), &context).is_none());
    assert!(started.elapsed() < Duration::from_secs(3));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn github_auth_import_is_shared_by_the_agent_and_slice_user() {
    let provisioner = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("slice-linux-docker/provision-linux-docker-slice.sh"),
    )
    .expect("slice provisioner should be readable");
    let import = provisioner
        .split("import_github_auth() {")
        .nth(1)
        .and_then(|tail| tail.split("remove_github_auth() {").next())
        .expect("GitHub import function should exist");

    assert!(import.contains("export HOME='$SLICE_PROVIDER_HOME'"));
    assert!(import.contains(
        "HOME='/home/slice' git config --global --add include.path \\\"$SLICE_PROVIDER_HOME/.gitconfig\\\""
    ));
    assert!(
        import.contains("ln -s \\\"$SLICE_PROVIDER_HOME/.config/gh\\\" '/home/slice/.config/gh'")
    );
    assert!(import.contains("-z '$SLICE_PRIVATE_HOST_ROOT' && ! -e '/home/slice/.config/gh'"));
    assert!(provisioner.contains("GH_CONFIG_DIR=$SLICE_PROVIDER_HOME/.config/gh"));
}

pub(super) fn test_record() -> SliceRecord {
    let store = SliceStore::default();
    store
        .create(
            "kernel-1",
            "machine-1",
            CreateSliceInput {
                source_slice_ref: None,
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

#[test]
fn local_docker_hostname_is_stable_rfc1123_and_bounded() {
    let mut record = test_record();
    record.name = format!("Production.Room_{}", "A".repeat(96));

    let hostname = local_docker_hostname(&record);
    assert_eq!(hostname, local_docker_hostname(&record));
    assert!(hostname.len() <= 63);
    assert!(hostname
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'));
    assert!(hostname
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphanumeric));
    assert!(hostname
        .as_bytes()
        .last()
        .is_some_and(u8::is_ascii_alphanumeric));

    let mut normalized_collision = record.clone();
    normalized_collision.name = record.name.replace(['.', '_'], "-");
    assert_ne!(
        hostname,
        local_docker_hostname(&normalized_collision),
        "different legal slice names must not collapse to one hostname"
    );

    let mut command = Command::new("slice-provisioner");
    configure_local_docker_slice_command(&mut command, &record, None, &test_options(), true)
        .expect("slice command should configure");
    let configured_hostname = command
        .get_envs()
        .find_map(|(key, value)| {
            (key == "CHARIOX_SLICE_HOSTNAME")
                .then(|| value.and_then(|value| value.to_str()))
                .flatten()
        })
        .expect("slice hostname should be configured");
    assert_eq!(configured_hostname, hostname);
}

#[test]
fn local_docker_provisioning_preserves_an_existing_valid_hostname() {
    let record = test_record();
    let mut command = Command::new("slice-provisioner");

    configure_local_docker_slice_command(&mut command, &record, None, &test_options(), true)
        .expect("slice command should configure");

    let configured_hostname = command
        .get_envs()
        .find_map(|(key, value)| {
            (key == "CHARIOX_SLICE_HOSTNAME")
                .then(|| value.and_then(|value| value.to_str()))
                .flatten()
        })
        .expect("slice hostname should be configured");
    assert_eq!(configured_hostname, "chariox-slice-dev");
}

pub(super) fn test_options() -> LocalDockerSliceOptions {
    LocalDockerSliceOptions {
        root: std::env::temp_dir(),
        home_public_key: DaemonConfig::for_tests().relay_public_key,
        docker_image: "chariox-slice-linux:test".to_string(),
        build_image: SliceImageBuildPolicy::Never,
        extension_dockerfile: None,
        allow_unconfined_seccomp: false,
        allow_provider_sandbox_compatibility: false,
        memory_mb: None,
        cpus: None,
        disk_layer_mb: None,
        disk_home_mb: None,
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

#[cfg(unix)]
#[test]
fn managed_broker_slice_does_not_require_docker_in_the_kernel_namespace() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    let previous_path = std::env::var_os("PATH");
    let previous_required = std::env::var_os("CHARIOX_SLICE_DOCKER_BROKER_REQUIRED");
    struct Restore {
        path: Option<std::ffi::OsString>,
        required: Option<std::ffi::OsString>,
        root: std::path::PathBuf,
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            match self.path.take() {
                Some(path) => std::env::set_var("PATH", path),
                None => std::env::remove_var("PATH"),
            }
            match self.required.take() {
                Some(required) => {
                    std::env::set_var("CHARIOX_SLICE_DOCKER_BROKER_REQUIRED", required)
                }
                None => std::env::remove_var("CHARIOX_SLICE_DOCKER_BROKER_REQUIRED"),
            }
            let _ = std::fs::remove_dir(&self.root);
        }
    }
    let empty_path = test_root("managed-broker-no-docker");
    std::fs::create_dir_all(&empty_path).unwrap();
    let _restore = Restore {
        path: previous_path,
        required: previous_required,
        root: empty_path.clone(),
    };
    std::env::set_var("PATH", &empty_path);
    std::env::set_var("CHARIOX_SLICE_DOCKER_BROKER_REQUIRED", "1");

    assert!(broker::configured());
    let error = ensure_host_docker_ready().expect_err("missing broker must fail closed");
    assert!(
        error.to_string().contains("managed slice Docker broker is unavailable"),
        "the managed kernel must report the missing broker, not require its own Docker binary: {error}"
    );
    std::env::remove_var("CHARIOX_SLICE_DOCKER_BROKER_REQUIRED");
    let local_error = ensure_host_docker_ready().expect_err("local Docker is still required");
    assert!(local_error
        .to_string()
        .contains("docker is required for local Docker slices"));
}

#[cfg(unix)]
#[test]
fn disk_pressure_admission_fault_probe() {
    crate::test_support::isolated_env_test!();
    use std::os::unix::fs::PermissionsExt;

    let _environment = crate::env_lock::lock();
    let root = test_root("disk-pressure-admission");
    let bin = root.join("bin");
    let docker = bin.join("docker");
    let log = root.join("docker.log");
    let capacity = root.join("docker-capacity");
    let state_dir = root.join("states/dev");
    let manifest = state_dir.join("manifest.json");
    std::fs::create_dir_all(&bin).expect("fake Docker directory should create");
    std::fs::create_dir_all(&state_dir).expect("prior state directory should create");
    std::fs::write(
        &docker,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$DOCKER_LOG"
case "$*" in
  "ps --format {{.Names}}") printf 'chariox-slice-dev\n' ;;
  "inspect --format {{.State.Running}} {{.State.Status}} chariox-slice-dev") printf 'true paused\n' ;;
  "info --format {{.DockerRootDir}}") printf '/tmp\n' ;;
  "inspect --size --format {{.SizeRw}} chariox-slice-dev") printf '1048576\n' ;;
  *" du -sb /home-src") printf '1048576 /home-src\n' ;;
  *" find /home-src -printf . | wc -c") printf '1\n' ;;
  *" df -B1 --output=avail /tmp") cat "$DOCKER_CAPACITY" ;;
  *"tar --zstd -C /home-src -cf - .") printf 'known-good-home' ;;
esac
exit 0
"#,
    )
    .expect("fake Docker should write");
    std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o700))
        .expect("fake Docker should become executable");

    let prior_state = saved_state(manifest.display().to_string());
    let prior_manifest =
        serde_json::to_vec_pretty(&prior_state).expect("prior state should encode");
    std::fs::write(&manifest, &prior_manifest).expect("prior manifest should write");
    std::fs::write(&capacity, b"1048576\n").expect("low capacity should write");

    let previous_path = std::env::var_os("PATH");
    let mut paths = vec![bin.clone()];
    if let Some(path) = &previous_path {
        paths.extend(std::env::split_paths(path));
    }
    std::env::set_var(
        "PATH",
        std::env::join_paths(paths).expect("fake Docker PATH should join"),
    );
    std::env::set_var("DOCKER_LOG", &log);
    std::env::set_var("DOCKER_CAPACITY", &capacity);

    let mut options = test_options();
    options.root = root.clone();
    let mut record = test_record();
    record.display_mode = SliceDisplayMode::Headless;
    snapshot_pause::begin(&record, &options, false)
        .expect("measurement obligation should persist without source pause");
    let rejection = disk_admission::with_slice_snapshot_disk_admission(|guard| {
        disk_admission::validate_slice_snapshot_disk_admission(&record, &options, guard)
    })
    .expect_err("low Docker capacity must reject the admission seam");
    snapshot_pause::recover(&record, &options)
        .expect("failed measurement obligation should retire");
    let pressured_calls = std::fs::read_to_string(&log).expect("Docker log should read");
    assert!(rejection
        .to_string()
        .contains("slice snapshot needs more disk headroom"));
    assert!(pressured_calls.contains("du -sb /home-src"));
    assert!(!pressured_calls
        .lines()
        .any(|call| call.starts_with("commit ")));
    assert_eq!(std::fs::read(&manifest).unwrap(), prior_manifest);
    std::fs::write(&capacity, b"107374182400\n").unwrap();
    snapshot_pause::begin(&record, &options, false)
        .expect("recovered measurement obligation should persist");
    disk_admission::with_slice_snapshot_disk_admission(|guard| {
        disk_admission::validate_slice_snapshot_disk_admission(&record, &options, guard)
    })
    .expect("recovered capacity should pass the independent admission seam");
    snapshot_pause::recover(&record, &options)
        .expect("successful measurement obligation should retire");
    assert!(!root
        .join("runtime")
        .join(&record.id)
        .join("snapshot-resume.json")
        .exists());
    let measurements = std::fs::read_to_string(&log).unwrap();
    assert!(!measurements.lines().any(|call| call.starts_with("pause ")
        || call.starts_with("stop ")
        || call.contains("slice-screen.sh")));
    std::fs::write(&log, "").unwrap();
    for result in [
        state::save_local_docker_slice_state_live(&record, &options).map(|_| ()),
        state::create_local_docker_slice_backup_live(&record, &options, Some("synthetic-refusal"))
            .map(|_| ()),
        state::save_local_docker_slice_state_retaining_replaced(&record, &options).map(|_| ()),
    ] {
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("unavailable because this storage layout"));
    }
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "");
    assert_eq!(std::fs::read(&manifest).unwrap(), prior_manifest);
    assert!(!root.join("backups").exists());

    match previous_path {
        Some(path) => std::env::set_var("PATH", path),
        None => std::env::remove_var("PATH"),
    }
    std::env::remove_var("DOCKER_LOG");
    std::env::remove_var("DOCKER_CAPACITY");

    println!(
        "CHARIOX_DISK_PRESSURE_PROBE:{}",
        serde_json::json!({
            "schema": "chariox.disk_pressure_admission_probe.v2",
            "independentAdmissionRejectsLowCapacity": true,
            "independentAdmissionAcceptsRecoveredCapacity": true,
            "unsupportedCaptureRefusesBeforeMutation": true,
            "lastKnownGoodPreserved": true,
            "reserveBytes": 2_u64 * 1024 * 1024 * 1024,
        })
    );
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn public_headless_slice_save_gracefully_quiesces_before_capture_and_fails_closed() {
    crate::test_support::isolated_env_test!();
    use std::os::unix::fs::PermissionsExt;

    let _environment = crate::env_lock::lock();
    let root = test_root("headless-save-quiescence");
    let bin = root.join("bin");
    let docker = bin.join("docker");
    let log = root.join("docker.log");
    let running = root.join("running");
    std::fs::create_dir_all(&bin).expect("fake Docker directory should create");
    std::fs::write(&running, b"running").expect("container should start running");
    std::fs::write(
        &docker,
        r##"#!/bin/sh
printf '%s\n' "$*" >> "$DOCKER_LOG"
case "$*" in
  "ps --format {{.Names}}")
    if [ -f "$DOCKER_RUNNING" ]; then printf 'chariox-slice-dev\n'; fi
    exit 0
    ;;
  "container inspect chariox-slice-dev") exit 0 ;;
  *"State.Running"*chariox-slice-dev)
    if [ -f "$DOCKER_RUNNING" ]; then printf 'true running\n'; else printf 'false exited\n'; fi
    exit 0
    ;;
  "info --format {{.DockerRootDir}}") printf '/tmp\n'; exit 0 ;;
  *"inspect --size --format {{.SizeRw}} chariox-slice-dev") printf '1048576\n'; exit 0 ;;
  *" du -sb /home-src") printf '1048576 /home-src\n'; exit 0 ;;
  *" find /home-src -printf . | wc -c") printf '1\n'; exit 0 ;;
  *" df -B1 --output=avail /tmp") printf '107374182400\n'; exit 0 ;;
  *"slice-screen.sh stop"*)
    if [ "${DOCKER_FAIL_SCREEN_STOP:-0}" = 1 ]; then
      printf 'fixture rejected graceful screen stop\n' >&2
      exit 17
    fi
    exit 0
    ;;
  "stop chariox-slice-dev") rm -f "$DOCKER_RUNNING"; exit 0 ;;
  *"tar --zstd -C /home-src -cf - .") printf 'fixture home archive'; exit 0 ;;
esac
exit 0
"##,
    )
    .expect("fake Docker should write");
    std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o700))
        .expect("fake Docker should become executable");

    struct RestoreEnvironment {
        path: Option<std::ffi::OsString>,
        docker_log: Option<std::ffi::OsString>,
        docker_running: Option<std::ffi::OsString>,
        fail_screen_stop: Option<std::ffi::OsString>,
        root: std::path::PathBuf,
    }
    impl Drop for RestoreEnvironment {
        fn drop(&mut self) {
            for (name, value) in [
                ("PATH", self.path.take()),
                ("DOCKER_LOG", self.docker_log.take()),
                ("DOCKER_RUNNING", self.docker_running.take()),
                ("DOCKER_FAIL_SCREEN_STOP", self.fail_screen_stop.take()),
            ] {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    let previous_path = std::env::var_os("PATH");
    let mut paths = vec![bin];
    if let Some(path) = &previous_path {
        paths.extend(std::env::split_paths(path));
    }
    let _restore = RestoreEnvironment {
        path: previous_path,
        docker_log: std::env::var_os("DOCKER_LOG"),
        docker_running: std::env::var_os("DOCKER_RUNNING"),
        fail_screen_stop: std::env::var_os("DOCKER_FAIL_SCREEN_STOP"),
        root: root.clone(),
    };
    std::env::set_var(
        "PATH",
        std::env::join_paths(paths).expect("fake Docker PATH should join"),
    );
    std::env::set_var("DOCKER_LOG", &log);
    std::env::set_var("DOCKER_RUNNING", &running);
    std::env::set_var("DOCKER_FAIL_SCREEN_STOP", "1");

    let mut record = test_record();
    record.display_mode = crate::slice::SliceDisplayMode::Headless;
    let mut options = test_options();
    options.root = root.clone();

    for fail_stop in ["1", "0"] {
        std::env::set_var("DOCKER_FAIL_SCREEN_STOP", fail_stop);
        let error = state::save_local_docker_slice_state(&record, &options)
            .expect_err("unsupported capture must refuse before stopping the running slice");
        assert!(error
            .to_string()
            .contains("unavailable because this storage layout"));
        assert!(running.exists());
        assert!(
            !log.exists(),
            "no Docker operation should occur before refusal"
        );
        assert!(!root.join("states").exists());
    }

    let screen = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("slice-linux-docker/docker/slice-screen.sh"),
    )
    .expect("slice screen script should be readable");
    assert!(screen.contains("slice_selkies stop >/dev/null"));
    assert!(screen.contains("python3 \"$ROOT/browser-lifecycle.py\" stop \"$CHROME_PROFILE\""));
    assert!(screen.contains("stop_process_pattern \"websockify.*$NOVNC_PORT\""));
    assert!(screen.contains("$HOME/.chariox/browser/chromium"));
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

fn backup_record(manifest_path: String) -> crate::slice::SliceBackupRecord {
    crate::slice::SliceBackupRecord {
        id: "gmail-ready-1".to_string(),
        name: "gmail-ready".to_string(),
        source_slice_id: "slice-1".to_string(),
        source_state_id: "gmail-ready".to_string(),
        image_ref: "chariox-slice-backup:gmail-ready-1".to_string(),
        home_archive_path: "/tmp/gmail-ready-home.tar.zst".to_string(),
        manifest_path,
        created_at_ms: 1000,
        size_bytes: Some(4096),
        home_archive_sha256: None,
        image_id: None,
    }
}

#[test]
fn backup_restore_rejects_legacy_artifacts_without_integrity_metadata() {
    let error = validate_local_docker_slice_backup(
        &test_record(),
        &backup_record("/tmp/missing-manifest.json".to_string()),
    )
    .expect_err("legacy backup without digests must be rejected");

    assert!(error.to_string().contains("integrity metadata"));
    assert!(error.to_string().contains("create a new backup"));
}

#[test]
fn backup_restore_rejects_cross_slice_records_before_reading_artifacts() {
    let mut backup = backup_record("/tmp/missing-manifest.json".to_string());
    backup.source_slice_id = "slice-other".to_string();
    backup.size_bytes = Some(7);
    backup.home_archive_sha256 = Some("a".repeat(64));
    backup.image_id = Some(format!("sha256:{}", "0".repeat(64)));

    let error = validate_local_docker_slice_backup(&test_record(), &backup)
        .expect_err("a backup from another slice must be rejected before artifact access");

    assert!(error.to_string().contains("belongs to another slice"));
    assert!(!error.to_string().contains("missing-manifest"));
}

#[cfg(unix)]
#[test]
fn backup_restore_quarantines_a_corrupt_archive_without_touching_known_good_state() {
    crate::test_support::isolated_env_test!();
    use sha2::Digest as _;
    use std::os::unix::fs::PermissionsExt;

    let _environment = crate::env_lock::lock();
    let root = test_root("backup-corrupt-archive");
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).expect("fake Docker directory should create");
    let docker = bin.join("docker");
    std::fs::write(
        &docker,
        format!(
            "#!/bin/sh\nif [ \"$1\" = image ] && [ \"$2\" = inspect ]; then printf 'sha256:{}\\n'; exit 0; fi\nexit 1\n",
            "a".repeat(64)
        ),
    )
    .expect("fake Docker should write");
    std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o700))
        .expect("fake Docker should become executable");
    let previous_path = std::env::var_os("PATH");
    let mut paths = vec![bin];
    if let Some(path) = &previous_path {
        paths.extend(std::env::split_paths(path));
    }
    std::env::set_var(
        "PATH",
        std::env::join_paths(paths).expect("fake Docker PATH should join"),
    );

    let manifest = root.join("manifest.json");
    let archive = root.join("home.tar.zst");
    std::fs::write(&archive, b"corrupt").expect("corrupt archive should write");
    let mut backup = backup_record(manifest.display().to_string());
    backup.home_archive_path = archive.display().to_string();
    backup.size_bytes = Some(7);
    backup.home_archive_sha256 = Some(format!("{:x}", sha2::Sha256::digest(b"correct")));
    backup.image_id = Some(format!("sha256:{}", "0".repeat(64)));
    std::fs::write(
        &manifest,
        serde_json::to_vec_pretty(&backup).expect("backup should encode"),
    )
    .expect("backup manifest should write");

    let error = validate_local_docker_slice_backup(&test_record(), &backup)
        .expect_err("a corrupt archive must be rejected before destructive restore");

    assert!(error.to_string().contains("archive integrity check failed"));
    assert!(error.to_string().contains("quarantined"));
    assert!(
        !archive.exists(),
        "the corrupt archive must leave the restore path"
    );
    let quarantined = std::fs::read_dir(&root)
        .expect("backup directory should remain readable")
        .map(|entry| entry.expect("backup entry should remain readable").path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("home.tar.zst.corrupt-"))
        })
        .expect("the corrupt archive should remain quarantined for inspection");
    assert_eq!(
        std::fs::read(quarantined).expect("quarantined archive should remain readable"),
        b"corrupt"
    );

    let known_good_dir = root.join("known-good");
    std::fs::create_dir_all(&known_good_dir).expect("known-good directory should create");
    let known_good_manifest = known_good_dir.join("manifest.json");
    let known_good_archive = known_good_dir.join("home.tar.zst");
    std::fs::write(&known_good_archive, b"known-good").expect("known-good archive should write");
    let mut known_good = backup_record(known_good_manifest.display().to_string());
    known_good.id = "known-good".to_string();
    known_good.name = "known-good".to_string();
    known_good.home_archive_path = known_good_archive.display().to_string();
    known_good.size_bytes = Some(10);
    known_good.home_archive_sha256 = Some(format!("{:x}", sha2::Sha256::digest(b"known-good")));
    known_good.image_id = Some(format!("sha256:{}", "a".repeat(64)));
    std::fs::write(
        &known_good_manifest,
        serde_json::to_vec_pretty(&known_good).expect("known-good backup should encode"),
    )
    .expect("known-good manifest should write");
    validate_local_docker_slice_backup(&test_record(), &known_good)
        .expect("the independent known-good backup should remain restorable");
    assert_eq!(
        std::fs::read(&known_good_archive).expect("known-good archive should remain readable"),
        b"known-good"
    );

    match previous_path {
        Some(path) => std::env::set_var("PATH", path),
        None => std::env::remove_var("PATH"),
    }
    std::fs::remove_dir_all(&root).expect("corrupt archive fixture should clean up");
    assert!(!root.exists());

    println!(
        "CHARIOX_SAVED_STATE_CORRUPTION_PROBE:{}",
        serde_json::json!({
            "schema": "chariox.saved_state_corruption_probe.v1",
            "corruptArchiveRejected": true,
            "corruptArchiveQuarantined": true,
            "restorePathCleared": true,
            "knownGoodBackupRestorable": true,
            "cleanupComplete": true,
        })
    );
}

#[test]
fn backup_restore_never_quarantines_a_file_outside_the_owned_archive_shape() {
    use sha2::Digest as _;

    let root = test_root("backup-corrupt-unowned-file");
    std::fs::create_dir_all(&root).expect("backup directory should create");
    let manifest = root.join("manifest.json");
    let unowned = root.join("unowned.txt");
    std::fs::write(&unowned, b"must-remain").expect("unowned file should write");
    let mut backup = backup_record(manifest.display().to_string());
    backup.home_archive_path = unowned.display().to_string();
    backup.size_bytes = Some(11);
    backup.home_archive_sha256 = Some(format!("{:x}", sha2::Sha256::digest(b"different")));
    backup.image_id = Some(format!("sha256:{}", "0".repeat(64)));
    std::fs::write(
        &manifest,
        serde_json::to_vec_pretty(&backup).expect("backup should encode"),
    )
    .expect("backup manifest should write");

    let error = validate_local_docker_slice_backup(&test_record(), &backup)
        .expect_err("an invalid archive path must fail without renaming it");

    assert!(
        error.to_string().contains("cannot be quarantined safely"),
        "{error}"
    );
    assert_eq!(
        std::fs::read(&unowned).expect("unowned file must remain readable"),
        b"must-remain"
    );
    std::fs::remove_dir_all(&root).expect("unowned-file fixture should clean up");
    assert!(!root.exists());
}

#[test]
fn backup_restore_never_locally_quarantines_a_broker_managed_archive() {
    let root = test_root("backup-corrupt-broker-managed");
    std::fs::create_dir_all(&root).expect("broker backup directory should create");
    let manifest = root.join("manifest.json");
    let archive = root.join("home.tar.zst");
    std::fs::write(&manifest, b"broker manifest").expect("broker manifest should write");
    std::fs::write(&archive, b"broker-owned corrupt bytes")
        .expect("broker archive fixture should write");

    let error = state::reject_corrupt_home_archive(
        &manifest,
        &archive,
        "slice.backup.restore",
        "broker-backup",
        true,
    )
    .expect_err("broker-managed corruption must fail without local mutation");

    assert!(error
        .to_string()
        .contains("managed archive integrity check failed"));
    assert_eq!(
        std::fs::read(&archive).expect("broker-owned archive must remain in place"),
        b"broker-owned corrupt bytes"
    );
    assert_eq!(
        std::fs::read_dir(&root)
            .expect("broker backup directory should remain readable")
            .count(),
        2,
        "kernel must not create a local quarantine generation for broker storage"
    );

    std::fs::remove_dir_all(&root).expect("broker fixture should clean up");
    assert!(!root.exists());
}

#[test]
fn backup_restore_rolls_back_failures_and_retains_recovery_artifacts_if_rollback_fails() {
    use std::cell::Cell;

    let rollback = backup_record("/tmp/rollback-manifest.json".to_string());
    let rollback_calls = Cell::new(0);
    let persistence_calls = Cell::new(0);
    let cleanup_calls = Cell::new(0);
    let error = state::restore_local_docker_slice_backup_with_rollback::<()>(
        &rollback,
        || {
            Err(crate::error::DaemonError::LocalTransport {
                operation: "slice.backup.restore",
                message: "injected target restore failure".to_string(),
            })
        },
        || panic!("state capture must not run after target restore failure"),
        |_, _| {
            persistence_calls.set(persistence_calls.get() + 1);
            Ok(())
        },
        || {
            rollback_calls.set(rollback_calls.get() + 1);
            Ok(())
        },
        |_| {},
        || cleanup_calls.set(cleanup_calls.get() + 1),
    )
    .expect_err("the original restore failure must be returned after successful rollback");
    assert!(error
        .to_string()
        .contains("injected target restore failure"));
    assert_eq!(rollback_calls.get(), 1);
    assert_eq!(persistence_calls.get(), 1);
    assert_eq!(cleanup_calls.get(), 1);

    let cleanup_calls = Cell::new(0);
    let error = state::restore_local_docker_slice_backup_with_rollback::<()>(
        &rollback,
        || {
            Err(crate::error::DaemonError::LocalTransport {
                operation: "slice.backup.restore",
                message: "injected target restore failure".to_string(),
            })
        },
        || panic!("state capture must not run after target restore failure"),
        |_, _| panic!("state persistence must not run when rollback capture fails"),
        || {
            Err(crate::error::DaemonError::LocalTransport {
                operation: "slice.backup.restore",
                message: "injected rollback failure".to_string(),
            })
        },
        |_| {},
        || cleanup_calls.set(cleanup_calls.get() + 1),
    )
    .expect_err("a failed rollback must retain its recovery artifacts");
    assert!(error
        .to_string()
        .contains("injected target restore failure"));
    assert!(error.to_string().contains("injected rollback failure"));
    assert!(error.to_string().contains(&rollback.manifest_path));
    assert_eq!(cleanup_calls.get(), 0);

    let rollback_calls = Cell::new(0);
    let persistence_calls = Cell::new(0);
    let cleanup_calls = Cell::new(0);
    let error = state::restore_local_docker_slice_backup_with_rollback(
        &rollback,
        || Ok(()),
        || Ok("restored state"),
        |state, _| {
            persistence_calls.set(persistence_calls.get() + 1);
            if *state == "restored state" {
                Err(crate::error::DaemonError::LocalTransport {
                    operation: "slice.backup.restore",
                    message: "injected durable-state failure".to_string(),
                })
            } else {
                Ok(())
            }
        },
        || {
            rollback_calls.set(rollback_calls.get() + 1);
            Ok("rollback state")
        },
        |_| {},
        || cleanup_calls.set(cleanup_calls.get() + 1),
    )
    .expect_err("durable-state failure must roll back the restored machine");
    assert!(error.to_string().contains("injected durable-state failure"));
    assert_eq!(rollback_calls.get(), 1);
    assert_eq!(persistence_calls.get(), 2);
    assert_eq!(cleanup_calls.get(), 1);

    let persistence_calls = Cell::new(0);
    let cleanup_calls = Cell::new(0);
    let error = state::restore_local_docker_slice_backup_with_rollback(
        &rollback,
        || Ok(()),
        || Ok("restored state"),
        |_, _| {
            persistence_calls.set(persistence_calls.get() + 1);
            Err(crate::error::DaemonError::LocalTransport {
                operation: "slice.backup.restore",
                message: "injected persistent durable-state failure".to_string(),
            })
        },
        || Ok("rollback state"),
        |_| {},
        || cleanup_calls.set(cleanup_calls.get() + 1),
    )
    .expect_err("failed rollback-state publication must retain recovery artifacts");
    assert!(error
        .to_string()
        .contains("injected persistent durable-state failure"));
    assert!(error.to_string().contains("automatic rollback also failed"));
    assert!(error.to_string().contains(&rollback.manifest_path));
    assert_eq!(persistence_calls.get(), 2);
    assert_eq!(cleanup_calls.get(), 0);
}

#[test]
fn backup_restore_reclaims_replaced_generations_only_after_resolution_persists() {
    use std::cell::RefCell;

    let rollback = backup_record("/tmp/rollback-manifest.json".to_string());
    let calls = RefCell::new(Vec::new());
    let state = state::restore_local_docker_slice_backup_with_rollback(
        &rollback,
        || {
            calls.borrow_mut().push("restore");
            Ok(())
        },
        || {
            calls.borrow_mut().push("capture");
            Ok("restored state")
        },
        |_, resolution| {
            assert_eq!(resolution, state::SliceBackupRestoreResolution::Restored);
            calls.borrow_mut().push("persist");
            Ok(())
        },
        || panic!("rollback must not run after a successful durable resolution"),
        |_| calls.borrow_mut().push("cleanup replaced state"),
        || calls.borrow_mut().push("cleanup rollback"),
    )
    .expect("restore should succeed");

    assert_eq!(state, "restored state");
    assert_eq!(
        calls.into_inner(),
        vec![
            "restore",
            "capture",
            "persist",
            "cleanup replaced state",
            "cleanup rollback",
        ]
    );
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
        "tint2rc",
        "browser-app-restore.mjs",
        "browser-cdp.mjs",
        "browser-controller-actions.mjs",
        "browser-controller-apps.mjs",
        "browser-controller-bar.mjs",
        "browser-controller-cdp.mjs",
        "browser-controller-dialogs.mjs",
        "browser-controller-events.mjs",
        "browser-controller-files.mjs",
        "browser-controller-frames.mjs",
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
fn managed_provider_isolation_probe_preserves_ordinary_filesystem_outside_publication() {
    let docker_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("slice-linux-docker/docker");
    let runtime = std::fs::read_to_string(docker_root.join("start-runtime.sh"))
        .expect("slice runtime script should be readable");
    let provisioner = std::fs::read_to_string(
        docker_root
            .parent()
            .expect("docker support directory should have a parent")
            .join("provision-linux-docker-slice.sh"),
    )
    .expect("slice provisioner should be readable");
    let wrapper =
        std::fs::read_to_string(docker_root.join("managed-provider-isolation-probe-wrapper.sh"))
            .expect("managed provider probe wrapper should be readable");

    assert!(!runtime.contains("provider_probe_unselected"));
    assert!(!provisioner.contains("prepare_managed_provider_probe_unselected_path"));
    assert!(wrapper.contains("outside managed workspace"));
    assert!(wrapper.contains("git clone --quiet"));
    assert!(wrapper.contains("/var/lib/chariox"));
    assert!(wrapper.contains("managed_provider_isolation=failure"));
    assert!(
        !wrapper
            .lines()
            .any(|line| line.trim() == r#"/var/lib/chariox-slice-share \"#),
        "approved publication-root ancestors must exist so a selected workspace can be mounted"
    );
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
fn linux_docker_headed_browser_does_not_promote_insecure_origins() {
    let script = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("slice-linux-docker/docker/slice-screen.sh"),
    )
    .expect("slice screen script should be readable");

    assert!(!script.contains("CHARIOX_SLICE_CHROME_TRUSTED_INSECURE_ORIGINS"));
    assert!(!script.contains("--unsafely-treat-insecure-origin-as-secure"));
    assert!(script.contains("--remote-debugging-address=127.0.0.1"));
    assert!(script.contains("--remote-debugging-port=9222"));
}

#[test]
fn linux_docker_headed_browser_reopens_tabs_after_snapshot_quiescence() {
    let script = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("slice-linux-docker/docker/slice-screen.sh"),
    )
    .expect("slice screen script should be readable");

    assert!(script.contains("[[ -d \"$CHROME_PROFILE/Default\" ]]"));
    assert!(script.contains("chrome_startup_target_args+=(--restore-last-session)"));
    assert!(script.contains("chrome_startup_target_args=(-- \"$CHROME_URL\")"));
    assert!(script.contains("\"${chrome_startup_target_args[@]}\""));
}

#[test]
fn linux_docker_computer_input_preserves_desktop_focus_and_maps_commands() {
    let root = test_root("slice-computer-input");
    let bin = root.join("bin");
    let home = root.join("home");
    let temp = root.join("tmp");
    std::fs::create_dir_all(&bin).expect("stub bin should be created");
    std::fs::create_dir_all(&home).expect("stub home should be created");
    std::fs::create_dir_all(&temp).expect("test temp directory should be created");
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
    write_executable(
        "timeout",
        "#!/bin/sh\nwhile [ \"${1#--}\" != \"$1\" ]; do shift; done\nshift\nif [ \"$1\" = /opt/chariox-selkies/bin/python ]; then shift; exec \"$CHARIOX_KEYBOARD_STUB\" \"$@\"; fi\nexec \"$@\"\n",
    );
    write_executable(
        "xdotool",
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$CHARIOX_XDOTOOL_LOG\"\n",
    );
    write_executable(
        "keyboard",
        "#!/bin/sh\nprintf '%s\\n' \"${1##*/}${2:+ $2}\" >> \"$CHARIOX_KEYBOARD_LOG\"\n[ \"${2:-}\" = reset ] || cat >> \"$CHARIOX_KEYBOARD_STDIN_LOG\"\n",
    );
    write_executable(
        "xclip",
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$CHARIOX_XCLIP_ARGS_LOG\"\ncase \"$*\" in\n  *'-in'*)\n    cat > \"$CHARIOX_XCLIP_LOG\"\n    [ \"${CHARIOX_XCLIP_FAIL_WRITE:-0}\" != 1 ] || exit 17\n    ;;\n  *'-out'*) cat \"$CHARIOX_XCLIP_LOG\" 2>/dev/null || true ;;\nesac\n",
    );
    let xdotool_log = root.join("xdotool.log");
    let keyboard_log = root.join("keyboard.log");
    let keyboard_stdin_log = root.join("keyboard-stdin.log");
    let xclip_log = root.join("xclip.log");
    let xclip_args_log = root.join("xclip-args.log");
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("slice-linux-docker/docker/slice-screen.sh");
    let run = |args: &[&str], stdin: Option<&str>| {
        let mut command = Command::new("bash");
        command
            .arg(&script)
            .args(args)
            .env("PATH", &path)
            .env_remove("LC_ALL")
            .env("HOME", &home)
            .env("TMPDIR", &temp)
            .env("CHARIOX_SLICE_ROOT", root.join("runtime"))
            .env("CHARIOX_XDOTOOL_LOG", &xdotool_log)
            .env("CHARIOX_KEYBOARD_STUB", bin.join("keyboard"))
            .env("CHARIOX_KEYBOARD_LOG", &keyboard_log)
            .env("CHARIOX_KEYBOARD_STDIN_LOG", &keyboard_stdin_log)
            .env("CHARIOX_XCLIP_LOG", &xclip_log)
            .env("CHARIOX_XCLIP_ARGS_LOG", &xclip_args_log);
        let output = if let Some(input) = stdin {
            let mut child = command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("computer input helper should start");
            child
                .stdin
                .take()
                .expect("computer input stdin should be piped")
                .write_all(input.as_bytes())
                .expect("computer input should be written");
            child
                .wait_with_output()
                .expect("computer input helper should finish")
        } else {
            command.output().expect("computer input helper should run")
        };
        assert!(
            output.status.success(),
            "computer input helper {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };

    run(&["move", "20", "30"], None);
    run(&["pointer-click", "320", "180", "right", "2"], None);
    run(
        &["pointer-drag", "120", "160", "720", "560", "middle"],
        None,
    );
    run(&["pointer-scroll", "640", "400", "-3", "5"], None);
    run(&["computer-type-stdin"], Some("Grüße 世界"));
    run(&["computer-key-stdin", "3"], Some("ctrl+shift+p"));
    let clipboard_text = "Clipboard Grüße 世界\nsecond line\n";
    run(&["computer-clipboard-write-stdin"], Some(clipboard_text));
    for _ in 0..100 {
        if std::fs::read(&xclip_log).is_ok_and(|bytes| bytes == clipboard_text.as_bytes()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        std::fs::read_to_string(&xclip_log).expect("clipboard write should reach xclip stdin"),
        clipboard_text
    );
    let clipboard_read = run(&["computer-clipboard-read"], None);
    assert_eq!(clipboard_read.stdout, clipboard_text.as_bytes());
    let clipboard_read_again = run(&["computer-clipboard-read"], None);
    assert_eq!(
        clipboard_read_again.stdout,
        clipboard_text.as_bytes(),
        "ordinary clipboard content must remain available after one read"
    );
    assert_eq!(
        std::fs::read_to_string(&xclip_args_log).expect("xclip calls should be logged"),
        concat!(
            "-selection clipboard -in\n",
            "-selection clipboard -out\n",
            "-selection clipboard -out\n",
        )
    );
    let mut failed_clipboard_command = Command::new("bash");
    failed_clipboard_command
        .arg(&script)
        .arg("computer-clipboard-write-stdin")
        .env("PATH", &path)
        .env("HOME", &home)
        .env("TMPDIR", &temp)
        .env("CHARIOX_SLICE_ROOT", root.join("runtime"))
        .env("CHARIOX_XDOTOOL_LOG", &xdotool_log)
        .env("CHARIOX_XCLIP_LOG", &xclip_log)
        .env("CHARIOX_XCLIP_ARGS_LOG", &xclip_args_log)
        .env("CHARIOX_XCLIP_FAIL_WRITE", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut failed_clipboard = failed_clipboard_command
        .spawn()
        .expect("failing clipboard helper should start");
    failed_clipboard
        .stdin
        .take()
        .expect("failing clipboard stdin should be piped")
        .write_all(b"temporary clipboard content")
        .expect("failing clipboard input should be written");
    let failed_clipboard = failed_clipboard
        .wait_with_output()
        .expect("failing clipboard helper should finish");
    assert_eq!(failed_clipboard.status.code(), Some(17));
    assert_eq!(
        std::fs::read_dir(&temp)
            .expect("test temp directory")
            .count(),
        0,
        "clipboard helper must remove its temporary stdin file"
    );
    run(&["computer-input-reset"], None);

    assert_eq!(
        std::fs::read_to_string(&xdotool_log).expect("xdotool call should be logged"),
        concat!(
            "mousemove 20 30\n",
            "mousemove 320 180 click --repeat 2 --delay 80 3\n",
            "mousemove 120 160 mousedown 2 mousemove --sync 720 560 mouseup 2\n",
            "mousemove 640 400\n",
            "click --repeat 3 --delay 20 6\n",
            "click --repeat 5 --delay 20 5\n",
        )
    );
    // MP-08/MP-11: repeated chords use the same strict XTEST helper as text
    // (xdotool can report success for unknown keysyms without input).
    assert_eq!(
        std::fs::read_to_string(&keyboard_stdin_log)
            .expect("keyboard text should reach the Selkies helper stdin"),
        "Grüße 世界ctrl+shift+p"
    );
    assert_eq!(
        std::fs::read_to_string(&keyboard_log)
            .expect("text, repeat and reset should use the shared keyboard helper"),
        "slice-keyboard.py\nslice-keyboard.py key-repeat\nslice-keyboard.py reset\n"
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
    let roots = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("slice-linux-docker/runtime-source-roots.txt"),
    )
    .expect("slice runtime source roots should be readable");
    let roots: Vec<&str> = roots.lines().collect();

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
    assert!(script.contains("runtime-source-roots.txt"));
    for root in [
        "Cargo.toml",
        "Cargo.lock",
        "adapters/rust",
        "apps/aegs-dummy",
        "apps/kernel",
        "apps/relay",
        "packages/aegs-sdk",
        "packages/event-protocol",
    ] {
        assert!(roots.contains(&root), "missing runtime source root {root}");
    }
    assert!(!script.contains("grep -v '^apps/kernel/slice-linux-docker/'"));
    assert!(dockerfile.contains("COPY packages/event-protocol packages/event-protocol"));
    // The kernel links the App packages; the runtime image must build them.
    for package in ["app-package", "app-runtime", "app-sdk"] {
        let root = format!("packages/{package}");
        assert!(dockerfile.contains(&format!("COPY {root} {root}")));
        assert!(roots.contains(&root.as_str()));
    }
    let bundle_lock = "apps/app-worker/bundle.lock.json";
    assert!(dockerfile.contains(&format!("COPY {bundle_lock} {bundle_lock}")));
    assert!(roots.contains(&bundle_lock));
    assert!(!dockerfile.contains("chariox-app-storage.service"));
    assert!(dockerfile.contains("COPY Cargo.toml Cargo.lock ./"));
    assert!(dockerfile.contains("cargo build --locked --release"));
    assert!(dockerfile.contains("npm ci --omit=dev"));
    assert!(dockerfile.contains("snapshot.debian.org/archive/debian/20260701T000000Z"));
    assert!(!dockerfile.contains("npm install -g"));
    assert!(!dockerfile.contains("rustup.rs"));
    assert!(!dockerfile.contains("deb.nodesource.com"));
    let dockerfile_lines: Vec<_> = dockerfile.lines().map(str::trim).collect();
    let artifact_stage = "FROM scratch AS managed-release-artifacts";
    let artifact_stage_indices: Vec<_> = dockerfile_lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| (*line == artifact_stage).then_some(index))
        .collect();
    assert_eq!(
        artifact_stage_indices.len(),
        1,
        "the signed release artifact stage must have one exact scratch declaration"
    );

    for base in dockerfile_lines
        .iter()
        .filter(|line| line.starts_with("FROM "))
    {
        let mut fields = base.split_whitespace();
        assert_eq!(fields.next(), Some("FROM"));
        let first = fields.next().expect("FROM should name a base image");
        let image = if first.starts_with("--platform=") {
            fields
                .next()
                .expect("FROM platform should be followed by an image")
        } else {
            first
        };
        if image == "scratch" {
            assert_eq!(
                *base, artifact_stage,
                "scratch is allowed only for the exact release artifact stage"
            );
        } else {
            assert!(
                base.contains("@sha256:"),
                "unpinned slice base image: {base}"
            );
        }
    }

    let artifact_stage_index = artifact_stage_indices[0];
    let artifact_stage_end = dockerfile_lines[artifact_stage_index + 1..]
        .iter()
        .position(|line| line.starts_with("FROM "))
        .map(|offset| artifact_stage_index + 1 + offset)
        .unwrap_or(dockerfile_lines.len());
    let artifact_stage_instructions: Vec<_> = dockerfile_lines
        [artifact_stage_index + 1..artifact_stage_end]
        .iter()
        .copied()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    assert_eq!(
        artifact_stage_instructions,
        vec![
            "COPY --from=rust-builder /opt/chariox-source/target/release/chariox-kernel /chariox-kernel",
            "COPY --from=rust-builder /opt/chariox-source/target/release/chariox-managed-bootstrap /chariox-managed-bootstrap",
            "COPY --from=rust-builder /opt/chariox-source/target/release/chariox-relay /chariox-relay",
            "COPY --from=rust-builder /opt/chariox-source/target/release/chariox-app-package /chariox-app-package",
            "COPY --from=rust-builder /opt/chariox-source/target/release/chariox-app-storage /chariox-app-storage",
        ],
        "the release artifact stage must export only the five signed runtime binaries"
    );
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
fn local_docker_slice_uses_the_safe_default_memory_limit() {
    let record = test_record();
    let mut command = Command::new("slice-provisioner");

    configure_local_docker_slice_command(&mut command, &record, None, &test_options(), true)
        .expect("slice command should configure");

    let memory_limit = command
        .get_envs()
        .find_map(|(key, value)| {
            (key == "CHARIOX_SLICE_DOCKER_MEMORY")
                .then(|| value.and_then(|value| value.to_str()))
                .flatten()
        })
        .expect("slice memory limit should be configured");
    assert_eq!(memory_limit, "2048m");
}

#[test]
fn local_docker_provider_sandbox_compatibility_selects_named_apparmor_boundary() {
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let previous_profile = std::env::var_os("CHARIOX_SLICE_APPARMOR_PROFILE");
    std::env::set_var("CHARIOX_SLICE_APPARMOR_PROFILE", "chariox-slice-provider");
    let record = test_record();
    let mut options = test_options();
    options.allow_provider_sandbox_compatibility = true;
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
        envs.get("CHARIOX_SLICE_ALLOW_PROVIDER_SANDBOX_COMPATIBILITY"),
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
                source_slice_ref: None,
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
    assert!(script.contains("--hostname \"$SLICE_HOSTNAME\""));
    assert!(script.contains("-e \"CHARIOX_SLICE_NOVNC_PORT=$SLICE_NOVNC_PORT\""));
    assert!(script.contains(
        "-e \"CHARIOX_SLICE_SCREEN_GEOMETRY=${CHARIOX_SLICE_SCREEN_GEOMETRY:-1280x800x24}\""
    ));
    assert!(
        script.contains("-e \"CHARIOX_SLICE_DISPLAY_MODE=${CHARIOX_SLICE_DISPLAY_MODE:-unknown}\"")
    );
    assert!(script.contains("docker create \"${docker_create_args[@]}\" \"$SLICE_IMAGE\""));
    assert!(!script.contains("$SLICE_DEVELOPMENT_ROOT:$SLICE_DEVELOPMENT_ROOT"));
}

#[cfg(unix)]
fn browser_admission_docker_fixture(root: &Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(root).unwrap();
    let path = root.join("browser-fixture.py");
    std::fs::write(
        &path,
        r#"#!/usr/bin/env python3
import json,os,sys
args=sys.argv[1:]
if args == ['info','--format','{{.ID}}']:
    print('synthetic-docker-engine')
elif args and args[0] == 'inspect' and not any(flag in args for flag in ['-f','--format']):
    running=os.path.exists(os.environ['DOCKER_RUNNING']) if 'DOCKER_RUNNING' in os.environ else True
    print(json.dumps([{
        'Id':'a'*64,'Image':'sha256:'+'b'*64,'Created':'2026-09-30T00:00:00Z',
        'Config':{'Env':['HOME=/home/slice'],'Labels':{
            'io.chariox.slice.id':os.environ['CHARIOX_SLICE_ID'],
            'io.chariox.slice.owner-kernel-id':os.environ['CHARIOX_SLICE_OWNER_KERNEL_ID'],
            'io.chariox.slice.owner-machine-id':os.environ['CHARIOX_SLICE_OWNER_MACHINE_ID']}},
        'HostConfig':{'PidMode':'private'},
        'State':{'Running':running,'Paused':False,'Restarting':False,'Pid':321 if running else 0,
                 'Status':'running' if running else 'exited','StartedAt':'2026-09-30T00:01:00Z',
                 'FinishedAt':'2026-09-30T00:02:00Z'}}]))
elif args and args[0] == 'exec' and any('profileProcessCount' in arg for arg in args):
    print(json.dumps({'disposition':'clear','profileProcessCount':0}))
elif args == ['system','dial-stdio']:
    sys.stdin.buffer.read()
    sys.stdout.buffer.write(b'HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n')
else:
    sys.exit(1)
"#,
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path
}

#[cfg(unix)]
#[test]
fn existing_slice_runtime_forwards_managed_workspace_roots() {
    use std::os::unix::fs::PermissionsExt;

    let root = test_root("existing-slice-workspace-roots");
    let browser_fixture = browser_admission_docker_fixture(&root);
    let bin = root.join("bin");
    let docker = bin.join("docker");
    let log = root.join("docker.log");
    std::fs::create_dir_all(&bin).expect("fake Docker directory should create");
    std::fs::write(
        &docker,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$DOCKER_LOG"
if "$BROWSER_FIXTURE" "$@"; then exit 0; fi
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
if [ "$1" = "inspect" ] && [ "$2" = "--format" ]; then
  case "$3" in
    *HostConfig.Ulimits*) printf '8192:8192\n'; exit 0 ;;
  esac
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
        .env("BROWSER_FIXTURE", &browser_fixture)
        .env("CHARIOX_SLICE_ID", "synthetic-slice")
        .env("CHARIOX_SLICE_OWNER_KERNEL_ID", "synthetic-owner-kernel")
        .env("CHARIOX_SLICE_OWNER_MACHINE_ID", "synthetic-owner-machine")
        .env("CHARIOX_SLICE_NAME", "saved-slice")
        .env("CHARIOX_SLICE_DOCKER_IMAGE", "fixture")
        .env("CHARIOX_SLICE_BASE_IMAGE", "fixture")
        .env("CHARIOX_SLICE_DEVELOPMENT_MOUNT_COUNT", "2")
        .env("CHARIOX_SLICE_DOCKER_NOFILE_LIMIT", "8192")
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

#[cfg(unix)]
#[test]
fn failed_save_recovery_starts_only_the_existing_container() {
    use std::os::unix::fs::PermissionsExt;

    let root = test_root("failed-save-recovery");
    let browser_fixture = browser_admission_docker_fixture(&root);
    let bin = root.join("bin");
    let docker = bin.join("docker");
    let log = root.join("docker.log");
    let running = root.join("running");
    std::fs::create_dir_all(&bin).expect("fake Docker directory should create");
    std::fs::write(
        &docker,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$DOCKER_LOG"
if "$BROWSER_FIXTURE" "$@"; then exit 0; fi
if [ "$1" = "container" ] && [ "$2" = "inspect" ]; then
  exit 0
fi
if [ "$1" = "inspect" ] && [ "$2" = "--format" ]; then
  case "$3" in
    *HostConfig.Ulimits*) printf '8192:8192\n'; exit 0 ;;
  esac
fi
if [ "$1" = "inspect" ] && [ "$2" = "-f" ]; then
  if [ -f "$DOCKER_RUNNING" ]; then printf 'true\n'; else printf 'false\n'; fi
  exit 0
fi
if [ "$1" = "start" ] && [ "$2" = "saved-slice" ]; then
  : > "$DOCKER_RUNNING"
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
        .arg("recover")
        .env("PATH", path)
        .env("TMPDIR", &root)
        .env("DOCKER_LOG", &log)
        .env("BROWSER_FIXTURE", &browser_fixture)
        .env("CHARIOX_SLICE_ID", "synthetic-slice")
        .env("CHARIOX_SLICE_OWNER_KERNEL_ID", "synthetic-owner-kernel")
        .env("CHARIOX_SLICE_OWNER_MACHINE_ID", "synthetic-owner-machine")
        .env("DOCKER_RUNNING", &running)
        .env("CHARIOX_SLICE_NAME", "saved-slice")
        .env("CHARIOX_SLICE_DOCKER_IMAGE", "prior-saved-image")
        .env("CHARIOX_SLICE_BASE_IMAGE", "current-runtime-image")
        .env("CHARIOX_SLICE_DOCKER_NOFILE_LIMIT", "8192")
        .env("CHARIOX_SLICE_START_DESKTOP", "0")
        .env("CHARIOX_SLICE_START_PROVIDER_SERVERS", "0")
        .env("CHARIOX_SLICE_START_RUNTIME", "1")
        .output()
        .expect("slice recovery command should execute");
    assert!(
        output.status.success(),
        "slice recovery failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let calls = std::fs::read_to_string(&log).expect("fake Docker log should read");
    assert!(
        calls.lines().any(|call| call == "start saved-slice"),
        "recovery must restart the stopped current container: {calls}"
    );
    assert!(
        calls
            .lines()
            .any(|call| call.ends_with(" saved-slice /opt/chariox-slice/start-runtime.sh")),
        "recovery must restart the worker runtime: {calls}"
    );
    for forbidden in ["image inspect", "create ", "rm -f", "volume create"] {
        assert!(
            !calls.lines().any(|call| call.starts_with(forbidden)),
            "recovery must not replace the current container via `{forbidden}`: {calls}"
        );
    }

    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn backup_restore_replaces_the_slice_in_order_and_leaves_it_stopped() {
    use std::os::unix::fs::PermissionsExt;

    let root = test_root("backup-restore-order");
    let browser_fixture = browser_admission_docker_fixture(&root);
    let bin = root.join("bin");
    let docker = bin.join("docker");
    let log = root.join("docker.log");
    let container = root.join("container");
    let running = root.join("running");
    let volume = root.join("volume");
    let archive = root.join("backup-home.tar.zst");
    let restored_stdin = root.join("restore-stdin");
    std::fs::create_dir_all(&bin).expect("fake Docker directory should create");
    std::fs::write(&container, b"").expect("container state should write");
    std::fs::write(&running, b"").expect("running state should write");
    std::fs::write(&volume, b"").expect("volume state should write");
    std::fs::write(&archive, b"backup archive").expect("backup archive should write");
    std::fs::write(
        &docker,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$DOCKER_LOG"
if "$BROWSER_FIXTURE" "$@"; then exit 0; fi
case "$*" in
  *"chariox-home-restore -") cat > "$DOCKER_RESTORE_STDIN"; exit 0 ;;
esac
if [ "$1" = "info" ]; then
  exit 0
fi
if [ "$1" = "image" ] && [ "$2" = "inspect" ]; then
  case "$*" in
    *relay-peer-protocol-version*) printf '%s\n' "$EXPECTED_PROTOCOL" ;;
    *runtime-source-revision*) printf '%s\n' "$EXPECTED_REVISION" ;;
    *io.chariox.selkies-version*)
      case "$*" in *"$SELKIES_CAPABLE_IMAGE"*) printf '0.0.0.dev0\n' ;; esac
      ;;
    *io.chariox.selkies-source-revision*)
      case "$*" in *"$SELKIES_CAPABLE_IMAGE"*) printf '%s\n' "$EXPECTED_SELKIES_REVISION" ;; esac
      ;;
    *io.chariox.selkies-source*)
      case "$*" in *"$SELKIES_CAPABLE_IMAGE"*) printf 'https://github.com/selkies-project/selkies/commit/%s\n' "$EXPECTED_SELKIES_REVISION" ;; esac
      ;;
    *io.chariox.selkies-license*)
      case "$*" in *"$SELKIES_CAPABLE_IMAGE"*) printf '%s\n' "$EXPECTED_SELKIES_LICENSE" ;; esac
      ;;
    *'{{.Id}}'*) printf 'sha256:backup-image\n' ;;
  esac
  exit 0
fi
if [ "$1" = "container" ] && [ "$2" = "inspect" ]; then
  [ -f "$DOCKER_CONTAINER" ] || exit 1
  if [ "${3:-}" = "-f" ]; then printf 'sha256:backup-image\n'; fi
  exit 0
fi
if [ "$1" = "inspect" ] && [ "$2" = "-f" ]; then
  if [ -f "$DOCKER_RUNNING" ]; then printf 'true\n'; else printf 'false\n'; fi
  exit 0
fi
if [ "$1" = "ps" ]; then
  if [ -f "$DOCKER_CONTAINER" ]; then
    if [ "${2:-}" = "-a" ] || [ -f "$DOCKER_RUNNING" ]; then printf 'saved-slice\n'; fi
  fi
  exit 0
fi
if [ "$1" = "stop" ] && [ "$2" = "saved-slice" ]; then
  rm -f "$DOCKER_RUNNING"
  exit 0
fi
if [ "$1" = "rm" ] && [ "${2:-}" = "saved-slice" ]; then
  rm -f "$DOCKER_CONTAINER" "$DOCKER_RUNNING"
  exit 0
fi
if [ "$1" = "volume" ] && [ "$2" = "inspect" ]; then
  if [ ! -f "$DOCKER_VOLUME" ]; then
    printf 'Error response from daemon: get saved-slice-home: no such volume\n' >&2
    exit 1
  fi
  case "$*" in
    *io.chariox.saved-home.archive-sha256*) cat "$DOCKER_VOLUME.archive-sha256"; exit $? ;;
    *io.chariox.saved-home.initialization-token*) cat "$DOCKER_VOLUME.initialization-token"; exit $? ;;
  esac
  exit 0
fi
if [ "$1" = "volume" ] && [ "$2" = "rm" ]; then
  rm -f "$DOCKER_VOLUME" "$DOCKER_VOLUME.archive-sha256" "$DOCKER_VOLUME.initialization-token"
  exit 0
fi
if [ "$1" = "volume" ] && [ "$2" = "create" ]; then
  : > "$DOCKER_VOLUME"
  for argument in "$@"; do
    case "$argument" in
      io.chariox.saved-home.archive-sha256=*)
        printf '%s' "${argument#*=}" > "$DOCKER_VOLUME.archive-sha256"
        ;;
      io.chariox.saved-home.initialization-token=*)
        printf '%s' "${argument#*=}" > "$DOCKER_VOLUME.initialization-token"
        ;;
    esac
  done
  exit 0
fi
if [ "$1" = "create" ]; then
  previous=""
  for argument in "$@"; do
    if [ "$previous" = "--name" ] && [ "$argument" = "saved-slice" ]; then
      : > "$DOCKER_CONTAINER"
      break
    fi
    previous="$argument"
  done
  exit 0
fi
if [ "$1" = "start" ] && [ "$2" = "saved-slice" ]; then
  : > "$DOCKER_RUNNING"
  exit 0
fi
case "$*" in
  *'cat /opt/chariox-slice/runtime-source-revision'*) printf '%s\n' "$EXPECTED_REVISION" ;;
esac
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
    let revision = format!("sha256:{}", "a".repeat(64));
    let selkies_lock: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("slice-linux-docker/selkies.lock.json"),
        )
        .expect("Selkies lock should be readable"),
    )
    .expect("Selkies lock should be valid JSON");
    let selkies_revision = selkies_lock["selkies"]["revision"]
        .as_str()
        .expect("Selkies lock should include its pinned revision");
    let selkies_license = selkies_lock["selkies"]["license"]
        .as_str()
        .expect("Selkies lock should include its license");
    let run_restore = |selkies_capable_image: &str| {
        Command::new("bash")
            .arg(&script)
            .arg("restore-state")
            .env("PATH", &path)
            .env("TMPDIR", &root)
            .env("DOCKER_LOG", &log)
            .env("BROWSER_FIXTURE", &browser_fixture)
            .env("CHARIOX_SLICE_ID", "synthetic-slice")
            .env("CHARIOX_SLICE_OWNER_KERNEL_ID", "synthetic-owner-kernel")
            .env("CHARIOX_SLICE_OWNER_MACHINE_ID", "synthetic-owner-machine")
            .env("DOCKER_CONTAINER", &container)
            .env("DOCKER_RUNNING", &running)
            .env("DOCKER_VOLUME", &volume)
            .env("DOCKER_RESTORE_STDIN", &restored_stdin)
            .env(
                "EXPECTED_PROTOCOL",
                crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION.to_string(),
            )
            .env("EXPECTED_REVISION", &revision)
            .env("EXPECTED_SELKIES_REVISION", selkies_revision)
            .env("EXPECTED_SELKIES_LICENSE", selkies_license)
            .env("SELKIES_CAPABLE_IMAGE", selkies_capable_image)
            .env("CHARIOX_SLICE_BUILD_CONTEXT_DIGEST", &revision)
            .env("CHARIOX_SLICE_NAME", "saved-slice")
            .env("CHARIOX_SLICE_HOME_VOLUME", "saved-slice-home")
            .env("CHARIOX_SLICE_DOCKER_IMAGE", "backup-image")
            .env("CHARIOX_SLICE_BASE_IMAGE", "runtime-image")
            .env("CHARIOX_SLICE_BUILD_IMAGE", "never")
            .env("CHARIOX_SLICE_SAVED_HOME_ARCHIVE", &archive)
            .env("CHARIOX_SLICE_START_DESKTOP", "1")
            .env("CHARIOX_SLICE_START_PROVIDER_SERVERS", "1")
            .env("CHARIOX_SLICE_START_RUNTIME", "1")
            .output()
            .expect("slice restore command should execute")
    };
    let output = run_restore("runtime-image");
    assert!(
        output.status.success(),
        "slice restore failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let calls = std::fs::read_to_string(&log).expect("fake Docker log should read");
    let calls = calls.lines().collect::<Vec<_>>();
    let position = |needle: &str| {
        calls
            .iter()
            .position(|call| call.starts_with(needle))
            .unwrap_or_else(|| panic!("missing Docker call `{needle}`: {calls:?}"))
    };
    let remove_container = position("rm saved-slice");
    let remove_volume = position("volume rm saved-slice-home");
    let create_volume = calls
        .iter()
        .position(|call| call.starts_with("volume create ") && call.ends_with(" saved-slice-home"))
        .unwrap_or_else(|| panic!("missing labeled home volume creation: {calls:?}"));
    let create_container = position("create --init --name saved-slice ");
    for label in [
        "io.chariox.selkies-version",
        "io.chariox.selkies-source-revision",
        "io.chariox.selkies-source",
        "io.chariox.selkies-license",
    ] {
        assert!(
            calls
                .iter()
                .any(|call| call.contains(label) && call.ends_with(" backup-image")),
            "restore must inspect the saved image's {label} capability label: {calls:?}"
        );
    }
    assert!(
        calls[create_container].ends_with(" runtime-image"),
        "restore must reject the saved image's missing Selkies labels and use the authoritative runtime image: {calls:?}"
    );
    let start_container = calls
        .iter()
        .position(|call| *call == "start saved-slice")
        .expect("replacement container should be started for configuration");
    let stop_container = calls
        .iter()
        .rposition(|call| *call == "stop saved-slice")
        .expect("restored container should be stopped");
    assert!(
        remove_container < remove_volume
            && remove_volume < create_volume
            && create_volume < create_container
            && create_container < start_container
            && start_container < stop_container,
        "restore lifecycle must be ordered: {calls:?}"
    );
    assert_eq!(
        std::fs::read(&restored_stdin).expect("restore should consume file stdin"),
        std::fs::read(&archive).expect("selected archive should remain readable"),
        "restore must stream the exact selected archive into the replacement volume"
    );
    assert!(calls.iter().any(|call| {
        call.starts_with("exec -i -u root saved-slice-home-restore-")
            && call.ends_with("chariox-home-restore -")
    }));
    assert!(
        !calls.iter().any(|call| {
            call.starts_with("cp -L ") && call.contains(archive.to_string_lossy().as_ref())
        }),
        "private restore must not stage the selected archive in a helper layer"
    );
    assert!(
        !calls
            .iter()
            .any(|call| call.contains("bash -lc /opt/chariox-slice/slice-screen.sh start")),
        "restore must not start the desktop: {calls:?}"
    );
    for forbidden_suffix in [
        " saved-slice /opt/chariox-slice/start-runtime.sh",
        " saved-slice /opt/chariox-slice/start-providers.sh",
    ] {
        assert!(
            !calls.iter().any(|call| call.ends_with(forbidden_suffix)),
            "restore must leave services stopped and must not run `{forbidden_suffix}`: {calls:?}"
        );
    }
    assert!(
        container.exists(),
        "replacement container should remain available"
    );
    assert!(
        !running.exists(),
        "replacement container must remain stopped"
    );
    assert!(
        volume.exists(),
        "replacement home volume should remain available"
    );

    std::fs::write(&log, b"").expect("compatible-image Docker log should reset");
    let compatible_output = run_restore("backup-image");
    assert!(
        compatible_output.status.success(),
        "compatible saved-image restore failed: {}",
        String::from_utf8_lossy(&compatible_output.stderr)
    );
    let compatible_calls_text =
        std::fs::read_to_string(&log).expect("compatible-image Docker log should read");
    let compatible_calls = compatible_calls_text.lines().collect::<Vec<_>>();
    let compatible_create = compatible_calls
        .iter()
        .find(|call| call.starts_with("create --init --name saved-slice "))
        .expect("compatible saved image should create the replacement container");
    assert!(
        compatible_create.ends_with(" backup-image"),
        "a runtime-compatible saved image should be retained: {compatible_calls:?}"
    );
    assert!(
        !running.exists(),
        "compatible saved-image restore must also leave the replacement stopped"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn local_docker_slice_rejects_mounting_development_control_root() {
    let mut record = SliceStore::default()
        .create(
            "kernel-1",
            "machine-1",
            CreateSliceInput {
                source_slice_ref: None,
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
fn saved_state_archives_use_distinct_generation_paths() {
    let root = test_root("slice-state-generations");
    let first = state::active_state_home_archive_path(&root);
    let second = state::active_state_home_archive_path(&root);

    assert_ne!(first, second);
    assert_eq!(first.parent(), Some(root.as_path()));
    assert_eq!(second.parent(), Some(root.as_path()));
    assert_eq!(
        first.extension().and_then(|value| value.to_str()),
        Some("zst")
    );
    assert_eq!(
        second.extension().and_then(|value| value.to_str()),
        Some("zst")
    );
}

#[test]
fn failed_saved_state_publication_after_archive_capture_preserves_restorable_prior_generation() {
    let root = test_root("slice-state-publication-failure");
    std::fs::create_dir_all(&root).expect("state directory should create");
    let manifest = root.join("manifest.json");
    let prior_archive = root.join("home-prior.tar.zst");
    let next_archive = root.join("home-next.tar.zst");
    std::fs::write(&prior_archive, b"prior generation").expect("prior archive should write");
    std::fs::write(&next_archive, b"next generation").expect("next archive should write");

    let mut prior = saved_state(manifest.display().to_string());
    prior.home_archive_path = prior_archive.display().to_string();
    prior.image_ref = "chariox-slice-state:gmail-ready-prior".to_string();
    let prior_manifest = serde_json::to_vec_pretty(&prior).expect("prior state should encode");
    std::fs::write(&manifest, &prior_manifest).expect("prior manifest should write");

    let mut next = prior.clone();
    next.home_archive_path = next_archive.display().to_string();
    next.image_ref = "chariox-slice-state:gmail-ready-next".to_string();
    next.updated_at_ms += 1;

    let error = state::publish_saved_state_generation_with(
        &manifest,
        &next,
        Some(&prior),
        |_path, _state| {
            Err(crate::error::DaemonError::LocalTransport {
                operation: "slice.state.manifest",
                message: "injected publication failure".to_string(),
            })
        },
    )
    .expect_err("injected manifest publication must fail");

    assert!(error.to_string().contains("injected publication failure"));
    assert_eq!(
        std::fs::read(&manifest).expect("prior manifest should remain readable"),
        prior_manifest
    );
    assert_eq!(
        std::fs::read(&prior_archive).expect("prior archive should remain readable"),
        b"prior generation"
    );
    assert!(
        !next_archive.exists(),
        "unpublished archive must be removed"
    );

    let restored: SliceSavedStateRecord = serde_json::from_slice(
        &std::fs::read(&manifest).expect("prior manifest should remain readable for restore"),
    )
    .expect("prior manifest should remain valid");
    let restore_options = test_options().with_saved_state(&restored);
    assert_eq!(
        restore_options.saved_home_archive.as_deref(),
        Some(prior_archive.as_path())
    );
    assert_eq!(restore_options.docker_image, prior.image_ref);

    std::fs::remove_dir_all(&root).expect("publication failure fixture should clean up");
    assert!(!root.exists());
}

#[test]
fn uncertain_saved_state_publication_after_manifest_rename_retains_both_generations() {
    let root = test_root("slice-state-publication-uncertain");
    std::fs::create_dir_all(&root).expect("state directory should create");
    let manifest = root.join("manifest.json");
    let prior_archive = root.join("home-prior.tar.zst");
    let next_archive = root.join("home-next.tar.zst");
    std::fs::write(&prior_archive, b"prior generation").expect("prior archive should write");
    std::fs::write(&next_archive, b"next generation").expect("next archive should write");

    let mut prior = saved_state(manifest.display().to_string());
    prior.home_archive_path = prior_archive.display().to_string();
    prior.image_ref = "chariox-slice-state:gmail-ready-prior".to_string();
    std::fs::write(
        &manifest,
        serde_json::to_vec_pretty(&prior).expect("prior state should encode"),
    )
    .expect("prior manifest should write");

    let mut next = prior.clone();
    next.home_archive_path = next_archive.display().to_string();
    next.image_ref = "chariox-slice-state:gmail-ready-next".to_string();
    next.updated_at_ms += 1;

    state::publish_saved_state_generation_with(&manifest, &next, Some(&prior), |path, state| {
        state::write_state_manifest_with(path, state, |_parent| {
            Err(std::io::Error::other(
                "injected directory sync failure after rename",
            ))
        })
    })
    .expect("a published manifest with uncertain durability must retain both generations");

    let published: SliceSavedStateRecord = serde_json::from_slice(
        &std::fs::read(&manifest).expect("renamed manifest should remain readable"),
    )
    .expect("renamed manifest should contain the next state");
    assert_eq!(published.home_archive_path, next.home_archive_path);
    assert_eq!(
        std::fs::read(&prior_archive).expect("prior archive must be retained"),
        b"prior generation"
    );
    assert_eq!(
        std::fs::read(&next_archive).expect("published archive must be retained"),
        b"next generation"
    );

    let prior_restore = test_options().with_saved_state(&prior);
    let next_restore = test_options().with_saved_state(&published);
    assert_eq!(
        prior_restore.saved_home_archive.as_deref(),
        Some(prior_archive.as_path())
    );
    assert_eq!(
        next_restore.saved_home_archive.as_deref(),
        Some(next_archive.as_path())
    );

    std::fs::remove_dir_all(&root).expect("uncertain publication fixture should clean up");
    assert!(!root.exists());
}

#[test]
fn saved_state_publication_interruption_preserves_restorable_generations() {
    failed_saved_state_publication_after_archive_capture_preserves_restorable_prior_generation();
    uncertain_saved_state_publication_after_manifest_rename_retains_both_generations();

    println!(
        "CHARIOX_SLICE_SAVE_INTERRUPTION_PROBE:{}",
        serde_json::json!({
            "schema": "chariox.slice_save_interruption_probe.v1",
            "preCommitFailurePreservedPrior": true,
            "unpublishedGenerationRemoved": true,
            "uncertainCommitRetainedPrior": true,
            "uncertainCommitRetainedNext": true,
            "bothGenerationsRestorable": true,
            "cleanupComplete": true,
        })
    );
}

#[test]
fn local_docker_slice_runtime_starts_desktop_for_headless_slices() {
    let store = SliceStore::default();
    let record = store
        .create(
            "kernel-1",
            "machine-1",
            CreateSliceInput {
                source_slice_ref: None,
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
fn slice_worker_identity_launch_separates_display_alias_and_hosted_machine() {
    let record = test_record();
    for hosted_machine in [None, Some(record.owner_machine_id.clone())] {
        let relay = LocalDockerSliceRelay {
            relay_url: "wss://relay.example.test".into(),
            container_relay_url: Some("wss://relay.example.test".into()),
            relay_token: "synthetic-bootstrap-token".into(),
            owner_public_key: Some("public-fixture".into()),
            cloud_relay_config_json: None,
            worker_machine_id: hosted_machine.clone(),
        };
        let mut command = Command::new("synthetic-provisioner");
        configure_local_docker_slice_command(
            &mut command,
            &record,
            Some(relay),
            &test_options(),
            true,
        )
        .unwrap();
        let envs: std::collections::BTreeMap<_, _> = command
            .get_envs()
            .filter_map(|(key, value)| Some((key.to_str()?, value?.to_str()?)))
            .collect();
        assert_eq!(
            envs.get("CHARIOX_SLICE_DAEMON_ID"),
            Some(&record.worker_kernel_ref.as_str())
        );
        assert_eq!(
            envs.get("CHARIOX_SLICE_DAEMON_ALIAS"),
            Some(&format!("slice:{}", record.name).as_str())
        );
        let expected_machine = hosted_machine.unwrap_or_else(|| format!("slice:{}", record.id));
        assert_eq!(
            envs.get("CHARIOX_SLICE_MACHINE_ID"),
            Some(&expected_machine.as_str())
        );
    }
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
        worker_machine_id: None,
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
        worker_machine_id: None,
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
        worker_machine_id: None,
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
        worker_machine_id: None,
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

#[test]
fn persisted_daemon_relay_url_recovers_public_slice_relay_endpoint() {
    let record = test_record();

    let endpoint =
        relay_endpoint_from_persisted_daemon_relay_url(&record, b"wss://relay.example.test\n")
            .expect("public relay endpoint should recover");

    assert_eq!(
        endpoint,
        SliceRelayEndpoint {
            url: "wss://relay.example.test".to_string(),
            private: false,
        }
    );
}

#[test]
fn persisted_daemon_relay_url_maps_container_loopback_to_private_host_endpoint() {
    let record = test_record();

    let endpoint =
        relay_endpoint_from_persisted_daemon_relay_url(&record, b"ws://127.0.0.1:43118\n")
            .expect("private relay endpoint should recover");

    assert_eq!(endpoint, local_docker_private_relay_endpoint(&record));
}

#[test]
fn pending_restore_reuses_rollback_and_clears_quarantine_only_after_durable_resolution() {
    let root = test_root("pending-restore-no-capture");
    std::fs::create_dir_all(&root).unwrap();
    let mut options = test_options();
    options.root = root.clone();
    let record = test_record();
    let prior = saved_state(root.join("prior-manifest.json").display().to_string());
    let rollback = backup_record(root.join("rollback-manifest.json").display().to_string());
    std::fs::write(
        &rollback.manifest_path,
        b"synthetic retained rollback manifest",
    )
    .unwrap();
    let transaction = crate::slice::SliceBackupRestoreTransactionRecord {
        id: "pending-synthetic".to_string(),
        source_slice_id: record.id.clone(),
        target_backup: backup_record(root.join("target-manifest.json").display().to_string()),
        rollback_backup: rollback.clone(),
        previous_saved_state: Some(prior.clone()),
        started_at_ms: 1,
    };
    let store = SliceStore::default();
    store.restore_records(vec![record.clone()]);
    store.restore_pending_backup_restore_records(vec![transaction.clone()]);
    let generation = state::recovered_rollback_generation(&record, &options, &transaction).unwrap();
    assert_eq!(generation.state.image_ref, rollback.image_ref);
    assert_eq!(
        generation.state.home_archive_path,
        rollback.home_archive_path
    );
    assert_eq!(generation.state.created_at_ms, rollback.created_at_ms);
    assert!(store
        .try_begin_operation(&record.id, "slice.start")
        .is_err());
    let resolve = |fail| {
        store.resolve_backup_restore_transactionally(
            &transaction.id,
            &record.id,
            generation.state.clone(),
            2,
            crate::slice::SliceBackupRestoreResolution::RolledBack,
            Some("rolled back".to_string()),
            |_, _, _| {
                if fail {
                    Err(crate::error::DaemonError::LocalTransport {
                        operation: "synthetic.persist",
                        message: "interrupted".to_string(),
                    })
                } else {
                    Ok(())
                }
            },
        )
    };
    assert!(resolve(true).is_err());
    assert_eq!(
        store.list_pending_backup_restores(),
        vec![transaction.clone()]
    );
    assert!(std::path::Path::new(&rollback.manifest_path).exists());
    let resolved = resolve(false).unwrap();
    assert_eq!(resolved.status, crate::slice::SliceStatus::Stopped);
    assert!(store.list_pending_backup_restores().is_empty());
    assert!(store.try_begin_operation(&record.id, "slice.start").is_ok());
    assert!(std::path::Path::new(&rollback.manifest_path).exists());
    assert_eq!(transaction.previous_saved_state, Some(prior));
    // Neither generation publication nor durable resolution invokes Docker.
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn restore_acknowledgement_retries_broker_failure_and_retains_rollback_until_acknowledged() {
    use std::cell::{Cell, RefCell};

    let record = test_record();
    let backup = |id: &str| crate::slice::SliceBackupRecord {
        id: id.to_string(),
        image_ref: format!("chariox-slice-backup:{id}"),
        home_archive_path: format!("/tmp/{id}-home.tar.zst"),
        ..backup_record(format!("/tmp/{id}-manifest.json"))
    };
    let transaction = |id: &str| crate::slice::SliceBackupRestoreTransactionRecord {
        id: id.to_string(),
        source_slice_id: record.id.clone(),
        target_backup: backup(&format!("{id}-target")),
        rollback_backup: backup(&format!("{id}-rollback")),
        previous_saved_state: None,
        started_at_ms: 1,
    };
    let store = SliceStore::default();
    store.restore_records(vec![record.clone()]);
    let first = transaction("restore-first");
    store
        .begin_backup_restore_transactionally(first.clone(), |_| Ok(()))
        .unwrap();
    // A failed restore rolls back; its published home is the rollback archive.
    store
        .resolve_backup_restore_transactionally(
            &first.id,
            &record.id,
            saved_state("/tmp/rolled-back-manifest.json".to_string()),
            2,
            crate::slice::SliceBackupRestoreResolution::RolledBack,
            None,
            |_, _, acknowledgement| {
                assert_eq!(
                    acknowledgement.home_archive_path,
                    first.rollback_backup.home_archive_path
                );
                Ok(())
            },
        )
        .unwrap();

    let attempts = Cell::new(0);
    let broker_available = Cell::new(false);
    let released = RefCell::new(Vec::new());
    let reconcile = || {
        state::reconcile_local_docker_restore_acknowledgements(
            &store,
            Some(&record.id),
            |slice, acknowledgement| {
                attempts.set(attempts.get() + 1);
                assert_eq!(slice.id, record.id);
                assert_eq!(acknowledgement.transaction_id, first.id);
                if broker_available.get() {
                    Ok(())
                } else {
                    Err(crate::error::DaemonError::LocalTransport {
                        operation: "slice.backup.restore",
                        message: "managed slice Docker broker is unavailable".to_string(),
                    })
                }
            },
            |acknowledgement| {
                store.acknowledge_backup_restore_transactionally(
                    &acknowledgement.transaction_id,
                    |_| Ok(()),
                )
            },
            |rollback| released.borrow_mut().push(rollback.id.clone()),
        )
        .unwrap()
    };

    // Broker failure: the committed resolution stays, the acknowledgement and
    // the rollback archive it names are retained, and no restore can start.
    let pending = reconcile();
    assert_eq!(pending.len(), 1);
    assert_eq!(store.list_pending_restore_acknowledgements(), pending);
    assert!(released.borrow().is_empty());
    assert_eq!(
        store
            .active_saved_state_for_slice(&record.id)
            .unwrap()
            .map(|state| state.id),
        Some("gmail-ready".to_string())
    );
    let second = transaction("restore-second");
    let error = store
        .begin_backup_restore_transactionally(second.clone(), |_| {
            panic!("an unacknowledged publication must refuse the next restore before journaling")
        })
        .expect_err("the broker would roll back the committed restore");
    assert!(error.to_string().contains(&first.id));
    assert!(store.try_begin_operation(&record.id, "slice.start").is_ok());

    // A failed durable acknowledgement commit also keeps the record.
    broker_available.set(true);
    assert!(store
        .acknowledge_backup_restore_transactionally(&first.id, |_| {
            Err(crate::error::DaemonError::LocalTransport {
                operation: "synthetic.persist",
                message: "interrupted".to_string(),
            })
        })
        .is_err());
    assert_eq!(store.list_pending_restore_acknowledgements().len(), 1);

    // Retry: acknowledged once, then the rollback is released exactly once.
    assert!(reconcile().is_empty());
    assert_eq!(attempts.get(), 2);
    assert_eq!(*released.borrow(), vec![first.rollback_backup.id.clone()]);
    assert!(reconcile().is_empty());
    assert_eq!(attempts.get(), 2, "an acknowledged restore is not retried");
    assert_eq!(released.borrow().len(), 1);
    store
        .begin_backup_restore_transactionally(second, |_| Ok(()))
        .expect("the next restore starts once acknowledged");
}

#[test]
fn restore_acknowledgement_keeps_rollback_published_as_active_saved_state() {
    let record = test_record();
    let rollback = crate::slice::SliceBackupRecord {
        home_archive_path: "/tmp/recovered-rollback-home.tar.zst".to_string(),
        ..backup_record("/tmp/recovered-rollback-manifest.json".to_string())
    };
    let transaction = crate::slice::SliceBackupRestoreTransactionRecord {
        id: "restore-recovered".to_string(),
        source_slice_id: record.id.clone(),
        target_backup: backup_record("/tmp/recovered-target-manifest.json".to_string()),
        rollback_backup: rollback.clone(),
        previous_saved_state: None,
        started_at_ms: 1,
    };
    let store = SliceStore::default();
    store.restore_records(vec![record.clone()]);
    store.restore_pending_backup_restore_records(vec![transaction.clone()]);
    // Startup recovery publishes the rollback artifacts as the active state.
    let recovered = SliceSavedStateRecord {
        image_ref: rollback.image_ref.clone(),
        home_archive_path: rollback.home_archive_path.clone(),
        ..saved_state("/tmp/recovered-manifest.json".to_string())
    };
    store
        .resolve_backup_restore_transactionally(
            &transaction.id,
            &record.id,
            recovered,
            2,
            crate::slice::SliceBackupRestoreResolution::RolledBack,
            None,
            |_, _, _| Ok(()),
        )
        .unwrap();
    let pending = state::reconcile_local_docker_restore_acknowledgements(
        &store,
        None,
        |_, _| Ok(()),
        |acknowledgement| {
            store
                .acknowledge_backup_restore_transactionally(&acknowledgement.transaction_id, |_| {
                    Ok(())
                })
        },
        |_| panic!("the active saved state still references the rollback"),
    )
    .unwrap();
    assert!(pending.is_empty());
    assert!(store.list_pending_restore_acknowledgements().is_empty());
}

#[cfg(unix)]
#[test]
fn release_kernel_finds_its_installed_slice_build_context_without_a_source_tree() {
    use std::os::unix::fs::symlink;

    let root = test_root("release-slice-build-context");
    let prefix = root.join("chariox-0.2.0-linux-x64");
    let kernel = prefix.join("bin/chariox-kernel");
    let link = root.join("local-bin/chariox-kernel");
    let system = root.join("usr/lib/chariox/slice-build-context");
    std::fs::create_dir_all(prefix.join("bin")).expect("bundle bin should create");
    std::fs::create_dir_all(root.join("local-bin")).expect("link directory should create");
    std::fs::write(&kernel, "").expect("kernel should write");
    symlink(&kernel, &link).expect("kernel link should create");
    let provisioner = "apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh";
    let bundled = std::fs::canonicalize(&prefix)
        .expect("prefix should canonicalize")
        .join("share/chariox/slice-build-context")
        .join(provisioner);
    let system_wide = system.join(provisioner);
    let install = |script: &Path| {
        std::fs::create_dir_all(script.parent().expect("provisioner has a parent"))
            .expect("context should create");
        std::fs::write(script, "").expect("provisioner should write");
    };

    // The system-wide context is the one the managed broker path already uses.
    assert_eq!(
        Path::new(SYSTEM_SLICE_BUILD_CONTEXT).join(SLICE_DOCKER_PROVISIONER),
        Path::new(MANAGED_SLICE_DOCKER_PROVISIONER)
    );

    // With no installed context, the error names both places and the override,
    // and never falls back to the source tree the kernel was built from.
    let error = installed_slice_script(Some(&kernel), &system)
        .expect_err("a release kernel without a context should refuse")
        .to_string();
    assert!(error.contains(&bundled.display().to_string()), "{error}");
    assert!(
        error.contains(&system_wide.display().to_string()),
        "{error}"
    );
    assert!(
        error.contains("CHARIOX_SLICE_DOCKER_PROVISIONER"),
        "{error}"
    );
    assert!(!error.contains(env!("CARGO_MANIFEST_DIR")), "{error}");

    // The system-wide context serves a kernel installed without its own.
    install(&system_wide);
    assert_eq!(
        installed_slice_script(Some(&kernel), &system).expect("system context"),
        system_wide
    );
    assert_eq!(
        installed_slice_script(None, &system).expect("system context"),
        system_wide
    );

    // The bundle's own context comes first, also through a link to the kernel.
    install(&bundled);
    assert_eq!(
        installed_slice_script(Some(&kernel), &system).expect("bundled context"),
        bundled
    );
    assert_eq!(
        installed_slice_script(Some(&link), &system).expect("bundled context"),
        bundled
    );
    std::fs::remove_dir_all(root).expect("fixture root should remove");
}

#[cfg(unix)]
#[test]
fn mp08_mp11_slice_sandbox_option_is_independent_of_broker_placement() {
    let _guard = crate::env_lock::lock();
    let name = "CHARIOX_SLICE_DOCKER_BROKER_REQUIRED";
    let previous = std::env::var_os(name);
    let mut config = DaemonConfig::for_tests();
    for selected in [false, true] {
        config
            .user_config
            .slices
            .linux
            .allow_provider_sandbox_compatibility = Some(selected);
        std::env::remove_var(name);
        let ordinary = LocalDockerSliceOptions::from_config(&config);
        std::env::set_var(name, "1");
        let managed = LocalDockerSliceOptions::from_config(&config);
        // Restore before assertions so a RED test cannot contaminate the suite.
        match &previous {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
        assert_eq!(ordinary.allow_provider_sandbox_compatibility, selected);
        assert_eq!(managed.allow_provider_sandbox_compatibility, selected);
        let record = test_record();
        let mut a = Command::new("unused");
        let mut b = Command::new("unused");
        configure_local_docker_slice_command(&mut a, &record, None, &ordinary, true).unwrap();
        configure_local_docker_slice_command(&mut b, &record, None, &managed, true).unwrap();
        assert_eq!(
            a.get_envs().collect::<Vec<_>>(),
            b.get_envs().collect::<Vec<_>>()
        );
    }
}

#[test]
fn mp08_mp11_inherited_tuning_is_explicit_in_both_adapters() {
    let _guard = crate::env_lock::lock();
    let tuning = [
        ("CHARIOX_SLICE_DOCKER_PIDS_LIMIT", "2048"),
        ("CHARIOX_SLICE_DOCKER_NOFILE_LIMIT", "32768"),
        ("CHARIOX_SLICE_MIN_FREE_MB", "768"),
    ];
    let previous = tuning.map(|(name, _)| (name, std::env::var_os(name)));
    for (name, value) in tuning {
        std::env::set_var(name, value);
    }
    let mut command = Command::new("unused-provisioner");
    let result = configure_local_docker_slice_command(
        &mut command,
        &test_record(),
        None,
        &test_options(),
        true,
    );
    for (name, value) in previous {
        if let Some(value) = value {
            std::env::set_var(name, value);
        } else {
            std::env::remove_var(name);
        }
    }
    result.unwrap();
    let projected = broker::provisioner_environment(&command);
    for (name, value) in tuning {
        assert_eq!(
            projected.get(name).map(String::as_str),
            Some(value),
            "missing tuning {name}"
        );
        assert!(command
            .get_envs()
            .any(|(key, actual)| key == name && actual == Some(std::ffi::OsStr::new(value))));
    }
}

#[cfg(unix)]
#[test]
fn deep_saved_images_flatten_and_admit_their_whole_root_filesystem() {
    crate::test_support::isolated_env_test!();
    use std::os::unix::fs::PermissionsExt;

    let _environment = crate::env_lock::lock();
    let root = test_root("capture-depth");
    let bin = root.join("bin");
    let log = root.join("calls.log");
    let depth = root.join("parent-depth");
    std::fs::create_dir_all(&bin).expect("fake tool directory should create");
    for (name, script) in [
        (
            "docker",
            r#"#!/bin/sh
printf 'docker %s\n' "$*" >> "$CALL_LOG"
case "$*" in
  "inspect --format {{.Image}} chariox-slice-dev") printf 'sha256:%064d\n' 7 ;;
  "image inspect --format {{len .RootFS.Layers}} sha256:"*) cat "$PARENT_DEPTH" ;;
  "ps --format {{.Names}}") printf 'chariox-slice-dev\n' ;;
  "inspect --format {{.State.Running}} {{.State.Status}} chariox-slice-dev") printf 'false exited\n' ;;
  "info --format {{.DockerRootDir}}") printf '/tmp\n' ;;
  "inspect --size --format {{.SizeRw}} chariox-slice-dev") printf '1048576\n' ;;
  "inspect --size --format {{.SizeRootFs}} chariox-slice-dev") printf '8589934592\n' ;;
  *" du -sb /home-src") printf '1048576 /home-src\n' ;;
  *" find /home-src -printf . | wc -c") printf '1\n' ;;
  *" df -B1 --output=avail /tmp") printf '5368709120\n' ;;
esac
exit 0
"#,
        ),
        (
            "node",
            "#!/bin/sh\nprintf 'node %s\\n' \"$*\" >> \"$CALL_LOG\"\nexit 0\n",
        ),
    ] {
        let path = bin.join(name);
        std::fs::write(&path, script).expect("fake tool should write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .expect("fake tool should become executable");
    }
    let previous_path = std::env::var_os("PATH");
    let mut paths = vec![bin.clone()];
    if let Some(path) = &previous_path {
        paths.extend(std::env::split_paths(path));
    }
    std::env::set_var(
        "PATH",
        std::env::join_paths(paths).expect("PATH should join"),
    );
    std::env::set_var("CALL_LOG", &log);
    std::env::set_var("PARENT_DEPTH", &depth);
    assert!(!broker::configured());

    let mut options = test_options();
    options.root = root.clone();
    let mut record = test_record();
    record.display_mode = SliceDisplayMode::Headless;
    let admit = || {
        snapshot_pause::begin(&record, &options, false)
            .expect("measurement obligation should persist");
        let result = disk_admission::with_slice_snapshot_disk_admission(|guard| {
            disk_admission::validate_slice_snapshot_disk_admission(&record, &options, guard)
        });
        snapshot_pause::recover(&record, &options).expect("measurement obligation should retire");
        result
    };
    let image = "chariox-slice-state:dev-depth";
    let calls = |depth_value: &str| {
        std::fs::write(&depth, depth_value).expect("parent depth should write");
        std::fs::write(&log, "").expect("call log should reset");
        capture_depth::commit_container_bounded("chariox-slice-dev", image, "slice.state.save")
            .expect("bounded capture should succeed");
        std::fs::read_to_string(&log).expect("call log should read")
    };

    // Below the bound: the ordinary one-layer commit, measured by its writable layer.
    let shallow = calls("99\n");
    assert!(
        shallow.contains(&format!("docker commit chariox-slice-dev {image}")),
        "{shallow}"
    );
    assert!(!shallow.contains("node "), "{shallow}");
    admit().expect("a 1 MiB writable layer fits the 5 GiB fixture capacity");

    // At the bound: flatten instead of a 101st layer, and admit the whole root filesystem.
    let deep = calls("100\n");
    assert!(
        deep.lines().any(|call| call.starts_with("node ")
            && call.ends_with(&format!(
                "captured-image-depth.mjs flatten chariox-slice-dev {image}"
            ))),
        "{deep}"
    );
    assert!(
        !deep.lines().any(|call| call.starts_with("docker commit")),
        "{deep}"
    );
    std::fs::write(&depth, "100\n").unwrap();
    let rejected = admit().expect_err("an 8 GiB flatten must not pass a 5 GiB admission");
    assert!(
        rejected
            .to_string()
            .contains("slice snapshot needs more disk headroom"),
        "{rejected}"
    );

    // Unknown depth keeps the ordinary commit.
    let unknown = calls("");
    assert!(
        unknown.contains(&format!("docker commit chariox-slice-dev {image}")),
        "{unknown}"
    );

    match previous_path {
        Some(path) => std::env::set_var("PATH", path),
        None => std::env::remove_var("PATH"),
    }
    std::env::remove_var("CALL_LOG");
    std::env::remove_var("PARENT_DEPTH");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
#[cfg(unix)]
fn mp11_relay_config_has_no_host_file_even_under_foreign_paths() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    for layout in ["absent", "loose-file", "leaf-symlink", "ancestor-symlink"] {
        let root = std::env::temp_dir().join(format!(
            "mp11-relay-input-{layout}-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut options = test_options();
        options.root = root.clone();
        let record = test_record();
        let foreign = root.join("foreign");
        std::fs::create_dir(&foreign).unwrap();
        let runtime = root.join("runtime");
        let destination = runtime.join(&record.id).join("cloud-relay-config.json");
        if layout == "ancestor-symlink" {
            symlink(&foreign, &runtime).unwrap();
        } else if layout != "absent" {
            std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
            if layout == "leaf-symlink" {
                std::fs::write(foreign.join("user-file"), b"user data").unwrap();
                symlink(foreign.join("user-file"), &destination).unwrap();
            } else {
                std::fs::write(&destination, b"user data").unwrap();
                std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o644))
                    .unwrap();
            }
        }
        let relay = LocalDockerSliceRelay {
            relay_url: "wss://relay.example.test".into(),
            container_relay_url: None,
            relay_token: "synthetic".into(),
            owner_public_key: None,
            cloud_relay_config_json: Some("synthetic non-secret config".into()),
            worker_machine_id: None,
        };
        let mut command = Command::new("slice-provisioner");
        configure_local_docker_slice_command(&mut command, &record, Some(relay), &options, true)
            .unwrap();
        assert!(!command
            .get_envs()
            .any(|(key, _)| key == "CHARIOX_SLICE_CLOUD_RELAY_CONFIG_HOST_PATH"));
        if layout == "absent" {
            assert!(!runtime.exists());
        }
        if layout == "ancestor-symlink" {
            assert_eq!(std::fs::read_dir(&foreign).unwrap().count(), 0);
        }
        if layout == "loose-file" || layout == "leaf-symlink" {
            assert_eq!(std::fs::read(&destination).unwrap(), b"user data");
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn mp11_log_tail_is_bounded_and_survives_split_utf8() {
    let root = test_root("mp11-log");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("run.log");
    // A 4000-byte tail starts inside this multibyte scalar on the old code.
    std::fs::write(&path, format!("{}x", "€".repeat(2000))).unwrap();
    let result = std::panic::catch_unwind(|| command_log_preview(&path));
    std::fs::write(&path, "a".repeat(2 * 1024 * 1024)).unwrap();
    let projected = read_slice_log_file_entry("fixture", &path, 1);
    std::fs::remove_dir_all(root).unwrap();
    assert!(result.is_ok(), "log slicing panicked inside a UTF-8 scalar");
    assert!(
        projected.text.len() <= 65536,
        "single-line log projection is unbounded"
    );
    assert!(projected.truncated);
}
