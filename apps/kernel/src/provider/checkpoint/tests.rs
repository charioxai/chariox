use super::*;
use crate::provider::{LaunchProviderRequest, ProviderResumeState};

const SESSION: &str = "00000000-0000-0000-0000-000000000008";
const ARTIFACT: &str =
    "sessions/2026/10/25/rollout-2026-10-25T00-00-00-00000000-0000-0000-0000-000000000008.jsonl";

fn checkpoint() -> ProviderNativeCheckpoint {
    let contents = b"private native session bytes".to_vec();
    ProviderNativeCheckpoint {
        format: "codex-jsonl-v1".to_string(),
        relative_path: ARTIFACT.to_string(),
        binding: NativeCheckpointBinding {
            provider: "codex".to_string(),
            session_id: SESSION.to_string(),
            account_sha256: "a".repeat(64),
            model: "gpt-6-luna".to_string(),
            variant: Some("low".to_string()),
            cli_version: "codex-cli 0.159.3".to_string(),
            source_os: "linux".to_string(),
            artifact_sha256: digest(&contents),
        },
        contents,
    }
}

#[test]
fn native_checkpoint_refuses_corruption_and_unsupported_formats() {
    let original = checkpoint();
    #[cfg(target_os = "linux")]
    assert_eq!(original.validate(), Ok(()));
    #[cfg(not(target_os = "linux"))]
    assert_eq!(
        original.validate(),
        Err(NativeCheckpointFailure::UnsupportedPlatform)
    );
    let mut corrupt = original.clone();
    corrupt.contents.push(0);
    assert_eq!(
        corrupt.validate(),
        Err(NativeCheckpointFailure::InvalidArtifact)
    );
    let mut version = original.clone();
    version.binding.cli_version = "codex-cli 0.0.0".to_string();
    assert_eq!(
        version.validate(),
        Err(NativeCheckpointFailure::UnsupportedVersion)
    );
    let mut platform = original;
    platform.binding.source_os = "macos".to_string();
    assert_eq!(
        platform.validate(),
        Err(NativeCheckpointFailure::UnsupportedPlatform)
    );
}

#[test]
fn native_checkpoint_paths_cannot_name_credentials_or_escape_sessions() {
    for path in [
        "auth.json",
        "../auth.json",
        "sessions/../../auth.json",
        "/sessions/2026/10/25/rollout.jsonl",
        "sessions/2026/10/25/../auth.json",
        "sessions/2026/10/25/rollout-foreign.jsonl",
    ] {
        assert!(
            !codex::valid_relative_path(path, SESSION),
            "accepted {path}"
        );
    }
    assert!(codex::valid_relative_path(ARTIFACT, SESSION));
    assert!(!codex::valid_relative_path(ARTIFACT, "../auth"));
}

#[test]
fn native_checkpoint_debug_and_generic_launch_serialization_hide_payloads() {
    let checkpoint = checkpoint();
    assert!(!format!("{checkpoint:?}").contains("private native session bytes"));
    let request = LaunchProviderRequest::new("session", "codex", "codex", "profile", "gpt-6-luna")
        .with_native_checkpoint(checkpoint);
    let serialized = serde_json::to_string(&request).unwrap();
    assert!(!serialized.contains("private native session bytes"));
    assert!(!serialized.contains("native_checkpoint"));
}

#[test]
fn required_native_session_survives_storage_and_blocks_all_resume_recovery() {
    let state = ProviderResumeState::from_codex_thread_id(SESSION).requiring_native_session(
        "codex",
        SESSION,
        Some(checkpoint().binding),
    );
    let state: ProviderResumeState =
        serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    assert!(state.required_native_session().is_some());
    assert_eq!(
        state.replacement_after_provider_resume_failure("codex", "thread/resume"),
        None
    );
    assert_eq!(
        state.replacement_after_provider_resume_failure("codex", "codex_thread_resume"),
        None
    );
    assert_eq!(state.codex_thread_id(), Some(SESSION));
    let changed =
        state.after_profile_change("codex", "source-account", "codex", "different-account");
    assert!(changed.required_native_session().is_none());
    assert!(changed.codex_thread_id().is_none());
}

#[cfg(target_os = "linux")]
#[test]
fn required_native_session_rejects_wrong_session_and_unverified_interfaces() {
    let checkpoint = checkpoint();
    let state = ProviderResumeState::from_codex_thread_id(SESSION).requiring_native_session(
        "codex",
        SESSION,
        Some(checkpoint.binding),
    );
    let required = state.required_native_session().unwrap();
    let request = LaunchProviderRequest::new("session", "codex", "codex", "profile", "gpt-6-luna")
        .with_variant(Some("low".to_string()))
        .with_resume_state(state.clone());
    assert_eq!(required.validate_request(&request), Ok(()));
    assert_eq!(
        required.confirm_session("new-thread"),
        Err(NativeCheckpointFailure::SessionChanged)
    );
    let native = request
        .clone()
        .with_client_interface(ProviderClientInterface::NativeTui);
    assert_eq!(
        required.validate_request(&native),
        Err(NativeCheckpointFailure::UnsupportedInterface)
    );
    let changed = request.with_variant(Some("high".to_string()));
    assert_eq!(
        required.validate_request(&changed),
        Err(NativeCheckpointFailure::ConfigurationMismatch)
    );
}

#[cfg(unix)]
struct Scratch(std::path::PathBuf);
#[cfg(unix)]
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "chariox-native-checkpoint-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
#[cfg(unix)]
impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn native_checkpoint_publication_is_atomic_private_and_never_overwrites() {
    use std::os::unix::fs::PermissionsExt;
    let root = Scratch::new();
    let directory = files::Directory::open(&root.0).unwrap();
    directory.publish(ARTIFACT, b"native-state").unwrap();
    assert_eq!(directory.read(ARTIFACT).unwrap(), b"native-state");
    assert_eq!(
        std::fs::metadata(root.0.join(ARTIFACT))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    directory.publish(ARTIFACT, b"native-state").unwrap();
    assert_eq!(
        directory.publish(ARTIFACT, b"different-state"),
        Err(NativeCheckpointFailure::TargetConflict)
    );
    assert_eq!(directory.read(ARTIFACT).unwrap(), b"native-state");
    let siblings = std::fs::read_dir(root.0.join("sessions/2026/10/25"))
        .unwrap()
        .count();
    assert_eq!(
        siblings, 1,
        "staging files must be cleaned on success and conflict"
    );
}

#[cfg(unix)]
#[test]
fn native_checkpoint_refuses_symlinked_parents_and_leaf_files() {
    use std::os::unix::fs::symlink;
    let root = Scratch::new();
    let outside = Scratch::new();
    let directory = files::Directory::open(&root.0).unwrap();
    symlink(&outside.0, root.0.join("sessions")).unwrap();
    assert!(directory.publish(ARTIFACT, b"native-state").is_err());
    assert_eq!(std::fs::read_dir(&outside.0).unwrap().count(), 0);
    std::fs::remove_file(root.0.join("sessions")).unwrap();
    std::fs::create_dir_all(root.0.join("sessions/2026/10/25")).unwrap();
    std::fs::write(outside.0.join("protected"), b"keep").unwrap();
    symlink(outside.0.join("protected"), root.0.join(ARTIFACT)).unwrap();
    assert!(directory.publish(ARTIFACT, b"native-state").is_err());
    assert_eq!(std::fs::read(outside.0.join("protected")).unwrap(), b"keep");
}

#[cfg(unix)]
#[test]
fn native_checkpoint_refuses_oversized_files_before_loading_them() {
    let root = Scratch::new();
    let directory = files::Directory::open(&root.0).unwrap();
    std::fs::create_dir_all(root.0.join("sessions/2026/10/25")).unwrap();
    let file = std::fs::File::create(root.0.join(ARTIFACT)).unwrap();
    file.set_len(MAX_NATIVE_CHECKPOINT_BYTES as u64 + 1)
        .unwrap();
    assert_eq!(
        directory.read(ARTIFACT),
        Err(NativeCheckpointFailure::TooLarge)
    );
}

#[cfg(unix)]
#[test]
fn required_native_session_blocks_durable_clearing_until_a_real_resume_ack() {
    use crate::agent::{
        AgentInstance, AgentService, AgentServiceStore, GridPosition, ProviderResumeClearOutcome,
    };
    let root = Scratch::new();
    let durable =
        crate::durable_state::DurableKernelStateStore::open(root.0.join("kernel.db")).unwrap();
    let agents = AgentServiceStore::new(AgentService::new());
    let mut agent = AgentInstance::new(
        "agent",
        "agent-ref",
        "session",
        None,
        "codex",
        Some("gpt-6-luna".to_string()),
        Some("low".to_string()),
        None,
        GridPosition::new(0, 0, 1, 1),
    );
    let state = ProviderResumeState::from_codex_thread_id(SESSION).requiring_native_session(
        "codex",
        SESSION,
        Some(checkpoint().binding),
    );
    agent.set_provider_resume_state(state.clone());
    agents.restore_agent(agent);
    assert!(matches!(
        agents
            .clear_provider_resume_state_durably_if_matches(
                &durable,
                "agent",
                "codex",
                SESSION,
                "run",
                "unresponsive"
            )
            .unwrap(),
        ProviderResumeClearOutcome::Required
    ));
    assert_eq!(
        agents
            .get_agent("agent")
            .unwrap()
            .provider_resume_state()
            .codex_thread_id(),
        Some(SESSION)
    );
    let mut confirmed = state;
    confirmed.record_native_resume_accepted("codex", "different-thread");
    assert!(confirmed.required_native_session().is_some());
    confirmed.record_native_resume_accepted("codex", SESSION);
    assert!(confirmed.required_native_session().is_none());
    assert_eq!(confirmed.codex_thread_id(), Some(SESSION));
    agents
        .set_agent_provider_resume_state_durably(&durable, "agent", confirmed, "run")
        .unwrap();
    assert!(matches!(
        agents
            .clear_provider_resume_state_durably_if_matches(
                &durable,
                "agent",
                "codex",
                SESSION,
                "run",
                "ordinary-recovery"
            )
            .unwrap(),
        ProviderResumeClearOutcome::Cleared
    ));
}

#[test]
fn native_checkpoint_launch_601_round_trip_and_hash_preserve_the_constraint() {
    use sha2::Digest;
    let binding = serde_json::to_value(checkpoint().binding).unwrap();
    let wire = serde_json::json!({"LaunchProviderRun": {
        "session_id":"session", "agent_id":null, "adapter_key":"codex", "provider":"codex",
        "account_profile":"profile", "model":"gpt-6-luna", "variant":"low",
        "provider_session_id":SESSION, "native_tui":false, "native_checkpoint_binding":binding
    }});
    let request: crate::local::LocalDaemonRequest = serde_json::from_value(wire.clone()).unwrap();
    let encoded = serde_json::to_value(request).unwrap();
    assert_eq!(encoded, wire);
    assert_eq!(
        format!("{:x}", sha2::Sha256::digest(encoded.to_string().as_bytes())),
        "e3f3777e36db3f245f27af77f5d076dd51e1c7ac00032a4188e4f4bfb17cdd9a"
    );
}

#[cfg(unix)]
#[test]
fn required_native_checkpoint_accepts_provider_appends_but_rejects_changed_history() {
    let root = Scratch::new();
    let directory = files::Directory::open(&root.0).unwrap();
    let original = b"{\"type\":\"session_meta\",\"payload\":{}}\n";
    let mut binding = checkpoint().binding;
    binding.artifact_sha256 = digest(original);
    directory.publish(ARTIFACT, original).unwrap();
    assert_eq!(
        codex::verify_required_artifact(&directory, ARTIFACT, &binding),
        Ok(())
    );
    let mut appended = original.to_vec();
    appended.extend_from_slice(b"{\"type\":\"turn_context\",\"payload\":{}}\n");
    std::fs::write(root.0.join(ARTIFACT), &appended).unwrap();
    assert_eq!(
        codex::verify_required_artifact(&directory, ARTIFACT, &binding),
        Ok(())
    );
    assert_eq!(
        directory.publish(ARTIFACT, original),
        Err(NativeCheckpointFailure::TargetConflict)
    );
    assert_eq!(std::fs::read(root.0.join(ARTIFACT)).unwrap(), appended);
    let mut rewritten = appended.clone();
    rewritten[10] = b'x';
    for changed in [
        original[..original.len() - 2].to_vec(),
        b"different history\n".to_vec(),
        rewritten,
    ] {
        std::fs::write(root.0.join(ARTIFACT), changed).unwrap();
        assert_eq!(
            codex::verify_required_artifact(&directory, ARTIFACT, &binding),
            Err(NativeCheckpointFailure::TargetConflict)
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn review_native_checkpoint_service_plans_before_contacting_managed_endpoint() {
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let root = Scratch::new();
    std::env::set_var("CHARIOX_CODEX_BIN", "/bin/true");
    let request = LaunchProviderRequest::new("session", "codex", "codex", "profile", "gpt-6-luna")
        .with_variant(Some("low".into()))
        .with_working_directory(root.0.clone())
        .with_provider_account_env(std::collections::BTreeMap::from([(
            "CODEX_HOME".into(),
            root.0.display().to_string(),
        )]))
        .with_native_checkpoint(checkpoint());
    let mut service = crate::provider::ProviderProcessService::new();
    let result = service.start_run_provider_only(request);
    std::env::remove_var("CHARIOX_CODEX_BIN");
    let run = result
        .expect("planning a checkpoint launch must not probe its unstarted app-server")
        .into_run();
    assert_eq!(run.state(), crate::provider::ProviderRunState::Starting);
    assert!(
        service.mark_run_running(run.id()).is_err(),
        "unimported checkpoint cannot admit prompts"
    );
    assert_eq!(run.resume_state().codex_thread_id(), Some(SESSION));
    assert!(run.resume_state().required_native_session().is_some());
    assert!(
        !root.0.join(ARTIFACT).exists(),
        "import waits for provider readiness"
    );
    assert!(!serde_json::to_string(&run)
        .unwrap()
        .contains("private native session bytes"));
    assert!(!serde_json::to_string(&run)
        .unwrap()
        .contains("native_checkpoint"));
}

#[cfg(target_os = "linux")]
#[test]
fn mk03_native_checkpoint_reimport_preserves_probe_appends_and_checks_original_history() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use tokio_tungstenite::tungstenite::{accept, Message};
    let root = Scratch::new();
    let cwd = root.0.join("workspace");
    let profile = root.0.join("profile");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&profile).unwrap();
    let version = root.0.join("codex-version");
    std::fs::write(&version, "#!/bin/sh\nprintf 'codex-cli 0.159.3\\n'\n").unwrap();
    std::fs::set_permissions(&version, std::fs::Permissions::from_mode(0o700)).unwrap();
    struct RestoreVersion(Option<std::ffi::OsString>);
    impl Drop for RestoreVersion {
        fn drop(&mut self) {
            match self.0.take() {
                Some(value) => std::env::set_var("CHARIOX_CODEX_BIN", value),
                None => std::env::remove_var("CHARIOX_CODEX_BIN"),
            }
        }
    }
    let _version = RestoreVersion(std::env::var_os("CHARIOX_CODEX_BIN"));
    std::env::set_var("CHARIOX_CODEX_BIN", &version);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("ws://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let artifact = profile.join(ARTIFACT);
    let server_artifact = artifact.clone();
    let server_cwd = cwd.clone();
    let thread = std::thread::spawn(move || {
        while !stopped.load(Ordering::SeqCst) {
            let (stream, _) = match listener.accept() {
                Ok(value) => value,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    continue;
                }
                Err(error) => panic!("checkpoint test listener: {error}"),
            };
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            let mut socket = accept(stream).unwrap();
            while let Ok(message) = socket.read() {
                if !message.is_text() {
                    break;
                }
                let Ok(text) = message.to_text() else {
                    break;
                };
                let request: serde_json::Value = serde_json::from_str(text).unwrap();
                if request.get("id").is_none() {
                    continue;
                }
                let result = match request["method"].as_str().unwrap() {
                    "initialize" => serde_json::json!({"userAgent":"codex-cli 0.159.3"}),
                    "account/read" => {
                        serde_json::json!({"account":{"type":"chatgpt","email":"checkpoint@example.invalid"},"requiresOpenaiAuth":true})
                    }
                    "thread/read" if !server_artifact.exists() => {
                        socket.send(Message::Text(serde_json::json!({"id":request["id"],"error":{"code":-32600,"message":format!("thread not loaded: {SESSION}")}}).to_string().into())).unwrap();
                        continue;
                    }
                    "thread/read" => {
                        serde_json::json!({"thread":{"id":SESSION,"path":server_artifact,"status":{"type":"idle"}}})
                    }
                    "thread/resume" => {
                        std::fs::OpenOptions::new()
                            .append(true)
                            .open(&server_artifact)
                            .unwrap()
                            .write_all(b"{\"type\":\"turn_context\",\"payload\":{}}\n")
                            .unwrap();
                        serde_json::json!({"thread":{"id":SESSION},"model":"gpt-6-luna","cwd":server_cwd})
                    }
                    method => panic!("unexpected checkpoint method: {method}"),
                };
                socket
                    .send(Message::Text(
                        serde_json::json!({"id":request["id"],"result":result})
                            .to_string()
                            .into(),
                    ))
                    .unwrap();
            }
        }
    });
    struct Server {
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            let result = self.thread.take().unwrap().join();
            if !std::thread::panicking() {
                result.unwrap();
            }
        }
    }
    let _server = Server {
        stop,
        thread: Some(thread),
    };
    let request = LaunchProviderRequest::new("session", "codex", "codex", "profile", "gpt-6-luna")
        .with_variant(Some("low".into()))
        .with_working_directory(cwd.clone());
    let run = crate::provider::RuntimeProviderRun::new(
        "checkpoint-import",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test".into(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::from([(
                "CODEX_HOME".into(),
                profile.display().to_string(),
            )]),
            pty_env_remove: Vec::new(),
            working_directory: Some(cwd),
            structured_endpoint: Some(endpoint),
        },
    );
    let mut checkpoint = checkpoint();
    checkpoint.contents =
        format!("{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{SESSION}\"}}}}\n")
            .into_bytes();
    checkpoint.binding.artifact_sha256 = digest(&checkpoint.contents);
    checkpoint.binding.account_sha256 = digest(b"codex:authenticated:checkpoint@example.invalid");
    checkpoint
        .import(&run)
        .expect("readiness import must resume the original thread");
    let after_probe = std::fs::read(&artifact).unwrap();
    assert!(
        after_probe.starts_with(&checkpoint.contents)
            && after_probe.len() > checkpoint.contents.len()
    );
    checkpoint
        .import(&run)
        .expect("the first real launch must accept intact history after its readiness probe");
    let after_launch = std::fs::read(&artifact).unwrap();
    assert!(after_launch.starts_with(&after_probe) && after_launch.len() > after_probe.len());
    let mut wrong_account = checkpoint.clone();
    wrong_account.binding.account_sha256 = "0".repeat(64);
    assert!(matches!(
        wrong_account.import(&run),
        Err(NativeCheckpointFailure::AccountMismatch)
    ));
    std::fs::write(
        &artifact,
        &checkpoint.contents[..checkpoint.contents.len() - 1],
    )
    .unwrap();
    assert!(matches!(
        checkpoint.import(&run),
        Err(NativeCheckpointFailure::TargetConflict)
    ));
    let mut rewritten = checkpoint.contents.clone();
    rewritten[0] ^= 1;
    std::fs::write(&artifact, rewritten).unwrap();
    assert!(matches!(
        checkpoint.import(&run),
        Err(NativeCheckpointFailure::TargetConflict)
    ));
}
