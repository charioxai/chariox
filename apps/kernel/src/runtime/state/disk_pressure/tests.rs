use super::*;
use crate::runtime::process_resources::{ProcessSample, SessionProcessState};

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

#[tokio::test]
async fn a_live_agent_can_first_use_its_temp_root_after_a_sweep() {
    if crate::test_support::isolate_environment_test() {
        return;
    }
    let state = crate::test_support::allocation::runtime_with_rooms(1);
    let room = state.owned.session_store.list_all_sessions().pop().unwrap();
    let agent = state
        .owned
        .agent_store
        .get_session_agents(room.id())
        .pop()
        .unwrap();
    let environment = crate::agent_temp::environment(room.id(), agent.id());
    let path = PathBuf::from(&environment[0].1);
    std::fs::write(path.join("stale"), b"disposable").unwrap();
    KernelRuntimeState::trim_temp(Duration::ZERO);
    assert!(
        !path.join("stale").exists(),
        "live agents' idle contents are trimmed"
    );
    let first = std::fs::File::create(path.join("first-later-use"));
    assert!(
        first.is_ok(),
        "a live agent's inherited TMPDIR was removed: {first:?}"
    );
}

fn row(pid: u32, command: &str, written: u64, deleted_open: u64, outlived: bool) -> SessionProcess {
    SessionProcess {
        owner: SessionProcessOwner::Agent("agent-1".into()),
        process: ProcessSample {
            process_id: format!("process-{pid}"),
            pid,
            command: command.into(),
            resident_set_bytes: 0,
            cpu_millis: 0,
            disk_io_bytes: Some(written),
            disk_written_bytes: Some(written),
            deleted_open_bytes: deleted_open,
            outlived,
            sampled_at_ms: 0,
            age_ms: 0,
        },
        state: SessionProcessState::Running,
    }
}

fn holding() -> Holding {
    Holding {
        session_id: "room-1".into(),
        agent_id: "agent-1".into(),
        processes: vec![
            row(41, "python3 fill.py", 3 * GIB, 2 * GIB, true),
            row(42, "sleep 600", 0, 0, false),
        ],
        temp: Some(PathBuf::from("/h/t/room0001/agent001")),
        temp_bytes: 800 * MIB,
        temp_biggest: vec![(PathBuf::from("/h/t/room0001/agent001/cache"), 500 * MIB)],
        workspace: Some(PathBuf::from("/work/repo")),
        workspace_biggest: vec![(PathBuf::from("/work/repo/target"), 4 * GIB)],
    }
}

#[test]
fn the_ask_names_what_the_agent_holds_so_it_can_stop_leftovers() {
    let text = ask_text(&holding(), GIB, 10 * GIB);
    assert!(text.contains("1 GB free"), "{text}");
    assert!(
        text.contains("pid 41 `python3 fill.py`: wrote 3 GB"),
        "{text}"
    );
    assert!(text.contains("keeps 2 GB in deleted files open"), "{text}");
    assert!(
        text.contains("still running after the command that started it ended"),
        "{text}"
    );
    assert!(
        !text.contains("sleep 600"),
        "a process that wrote nothing is not listed: {text}"
    );
    assert!(
        text.contains("$TMPDIR (/h/t/room0001/agent001): 800 MB; biggest: cache (500 MB)"),
        "{text}"
    );
    assert!(text.contains("target (4 GB)"), "{text}");
    assert!(text.contains("Never delete results or source"), "{text}");
}

#[test]
fn holders_are_ranked_by_what_their_processes_wrote_hold_deleted_and_keep_in_temp() {
    assert_eq!(holding().used(), 3 * GIB + 2 * GIB + 800 * MIB);
}

#[test]
fn just_under_the_floor_the_ask_does_not_say_ten_gigabytes_below_ten_gigabytes() {
    let text = ask_text(&holding(), 10 * GIB - 40 * MIB, 10 * GIB);
    assert!(text.contains("10200 MB free, below the 10240 MB"), "{text}");
}

/// Retained launches share one agent's disk usage and must not consume the
/// other holders' recipient slots. A later workspace selection is authoritative.
#[tokio::test]
async fn repeated_launches_do_not_duplicate_disk_pressure_recipients() {
    if crate::test_support::isolate_environment_test() {
        return;
    }
    let state = crate::test_support::allocation::runtime_with_rooms(3);
    let current = crate::test_support::TestWorktree::new("disk-pressure-current");
    let old = crate::test_support::TestWorktree::new("disk-pressure-old");
    let rooms = state.owned.session_store.list_all_sessions();
    let mut agents = Vec::new();
    for (index, room) in rooms.iter().enumerate() {
        let agent = state
            .owned
            .agent_store
            .get_session_agents(room.id())
            .pop()
            .unwrap();
        state
            .owned
            .agent_store
            .update_agent_config(
                agent.id(),
                None,
                None,
                None,
                Some(Some(current.path().display().to_string())),
            )
            .unwrap();
        let environment = crate::agent_temp::environment(room.id(), agent.id());
        let temp = PathBuf::from(&environment[0].1);
        let mut file = std::fs::File::create(temp.join("rebuildable")).unwrap();
        use std::io::Write;
        let chunk = vec![1; MIB as usize];
        for _ in 0..(96 - index as u64 * 8) {
            file.write_all(&chunk).unwrap();
        }
        // Three past launches for the biggest holder, one for each other holder.
        for launch in 0..if index == 0 { 3 } else { 1 } {
            let request = crate::provider::LaunchProviderRequest::new(
                room.id(),
                "codex",
                "codex",
                "default",
                "test",
            )
            .with_agent_id(agent.id());
            let mut run = crate::provider::RuntimeProviderRun::new(
                format!("run-{index}-{launch}"),
                &request,
                crate::provider::ProviderLaunchResult {
                    endpoint_mode: crate::provider::AgentEndpointMode::External,
                    process_label: "retained-launch".into(),
                    pty_target: None,
                    pty_program: None,
                    pty_args: Vec::new(),
                    pty_env: Default::default(),
                    pty_env_remove: Vec::new(),
                    working_directory: Some(old.path().to_path_buf()),
                    structured_endpoint: None,
                },
            );
            run.mark_ended();
            state.owned.provider_store.write().insert_run_for_test(run);
        }
        // Keep the cleanup turn queued, as for an agent doing other work.
        let busy = crate::session::PromptQueueItem::new(
            format!("busy-{index}"),
            "test",
            agent.id(),
            "current work",
            crate::session::PromptStatus::Running,
        );
        state
            .owned
            .prompt_state_owner
            .submit_prepared_prompt(room, busy, false)
            .unwrap();
        agents.push(agent.id().to_string());
    }
    assert_eq!(state.owned.provider_store.list_runs().len(), 5);
    // Only the volume measurement is simulated; recipients and disk scans use
    // the real stores and bounded files, without filling the host's disk.
    let mut pace = Pace::default();
    let pressured = state.pressured_agents(|_| Some((1, GIB, 10 * GIB)), &pace.asked);
    let recipients = pressured
        .iter()
        .map(|(holding, _, _)| holding.agent_id.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        recipients, agents,
        "one cleanup turn per holder, in size order"
    );
    for (holding, _, _) in &pressured {
        assert_eq!(holding.workspace.as_deref(), Some(current.path()));
    }
    let overlapping = state.pressured_agents(|_| Some((1, GIB, 10 * GIB)), &pace.asked);
    state.ask_pressured_agents(pressured, &mut pace).await;
    state.ask_pressured_agents(overlapping, &mut pace).await;
    assert!(state
        .pressured_agents(|_| Some((1, GIB, 10 * GIB)), &pace.asked)
        .is_empty());
    for room in &rooms {
        let agent = state
            .owned
            .agent_store
            .get_session_agents(room.id())
            .pop()
            .unwrap();
        let session = state.owned.session_store.get_session(room.id()).unwrap();
        let (_, queued) = state
            .owned
            .prompt_state_owner
            .state_parts(&session, agent.id());
        assert_eq!(
            queued.len(),
            1,
            "one cleanup turn per agent, including overlapping passes"
        );
    }
}

/// MP-08 / MP-09 / MP-10 / MP-11: an unanswered ordinary cold-launch
/// Vault interaction must not delay timers belonging to another Room.
#[tokio::test]
async fn a_disk_pressure_unlock_does_not_stall_another_rooms_timer() {
    if crate::test_support::isolate_environment_test() {
        return;
    }
    use crate::durable_state::agent_lifecycle::{AgentWake, Operation};
    let workspace = crate::test_support::TestWorktree::new("disk-wake-workspace");
    let private = crate::test_support::TestWorktree::new("disk-wake-private");
    let vault = private.path().join("vault.json");
    crate::secret::create_chariox_encrypted_vault_for_test(&vault, "test passphrase").unwrap();
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::CharioxEncrypted;
    config.user_config.credential_vault.path = vault.display().to_string();
    config.user_config.credential_vault.unlock_policy =
        crate::config::CredentialVaultUnlockPolicy::KernelInit;
    let mut app = crate::test_support::bootstrap_authenticated_app(config).unwrap();
    let mut rooms = Vec::new();
    for _ in 0..2 {
        rooms.push(
            crate::app::KernelSessionService::new(&mut app)
                .create_session(workspace.session_request())
                .unwrap(),
        );
    }
    use crate::project_environment::*;
    ProjectEnvironmentStore::new(&app.config().private_runtime_state_root())
        .save(&StoredProjectEnvironment {
            source: None,
            manifest: ProjectEnvironmentManifest {
                schema_version: 1,
                project_id: rooms[0].0.project_id().into(),
                evidence_digest: ProjectEnvironmentEvidence::default().digest(),
                entries: vec![],
                private_files: vec![],
                toolchain_hints: vec![],
                package_hints: vec![],
                service_hints: vec![],
            },
            evidence: ProjectEnvironmentEvidence::default(),
            reported_missing: Default::default(),
            reviewed_manifest: None,
            last_review: None,
        })
        .unwrap();
    let mut state =
        super::super::workflow_prompt_queue_owned_state::tests::runtime_state_from_app(app);
    static VOLUME_CHECKS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    state.disk_volume_probe = Some(|_| {
        VOLUME_CHECKS.fetch_add(1, Ordering::Relaxed);
        Some((1, GIB, 10 * GIB))
    });
    let (holder_room, holder) = &rooms[0];
    let environment = crate::agent_temp::environment(holder_room.id(), holder.id());
    let scratch = PathBuf::from(&environment[0].1).join("rebuildable");
    use std::io::Write;
    let mut file = std::fs::File::create(&scratch).unwrap();
    let chunk = vec![1; MIB as usize];
    for _ in 0..65 {
        file.write_all(&chunk).unwrap();
    }
    // Retain the live descriptor so the pressure trim cannot remove this holding.
    let scheduler = {
        let state = state.clone();
        tokio::spawn(async move { state.run_agent_wake_scheduler().await })
    };
    let result = async {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let room = state
                    .owned
                    .session_store
                    .get_session(holder_room.id())
                    .unwrap();
                if room
                    .active_interactions()
                    .iter()
                    .any(|i| i.id().starts_with("vault-unlock-"))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("disk-pressure cold launch must request the real Vault unlock");
        let checks_at_unlock = VOLUME_CHECKS.load(Ordering::Relaxed);
        let (room, agent) = &rooms[1];
        let store = &state.owned.durable_state_store;
        let now = crate::session::unix_epoch_ms();
        store
            .agent_lifecycle(Operation::Begin {
                owner: "test-owner".into(),
                room: room.id().into(),
                agent: agent.id().into(),
                prompt: "timer-task".into(),
                run: Some("timer-run".into()),
                now,
            })
            .unwrap();
        store
            .agent_lifecycle(Operation::CreateWake {
                task: "timer-task".into(),
                prompt: "timer-task".into(),
                wake: AgentWake {
                    id: "unrelated-timer".into(),
                    task_id: "timer-task".into(),
                    room_id: room.id().into(),
                    agent_id: agent.id().into(),
                    registration_id: "completion-unrelated-timer".into(),
                    kind: "timer".into(),
                    label: "unrelated Room".into(),
                    state: String::new(),
                    created_at_ms: now,
                    verified_at_ms: None,
                    next_due_ms: Some(now + 100),
                    interval_ms: None,
                    command: vec![],
                    match_text: None,
                    matched_at_ms: None,
                    pid: None,
                    exit_code: None,
                    summary: String::new(),
                    detail: serde_json::Value::Null,
                    fire_count: 0,
                    missed_fires: 0,
                    last_fired_at_ms: None,
                    last_sequence: None,
                    last_delivery: None,
                    last_delivered_at_ms: None,
                    last_acknowledged_at_ms: None,
                    alerted_sequence: None,
                },
            })
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let wakes = store
                    .agent_wakes(Some(room.id()), Some(agent.id()))
                    .unwrap();
                if wakes.iter().any(|wake| wake.fire_count == 1) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("another Room's timer must fire within a normal tick while Vault stays unanswered");
        // Stay unanswered beyond a maintenance period: no second pass may
        // start while this one owns prompt admission.
        tokio::time::sleep(Duration::from_secs(6)).await;
        assert_eq!(
            VOLUME_CHECKS.load(Ordering::Relaxed),
            checks_at_unlock,
            "a blocked maintenance pass must remain single-flight"
        );
        let room = state
            .owned
            .session_store
            .get_session(holder_room.id())
            .unwrap();
        assert_eq!(room.active_interactions().len(), 1);
        assert!(room.active_interactions()[0]
            .id()
            .starts_with("vault-unlock-"));
        let now = crate::session::unix_epoch_ms();
        let tick = state.wake_scheduler_health(now)["last_tick_at_ms"]
            .as_u64()
            .unwrap();
        assert!(
            now.saturating_sub(tick) < 2_000,
            "scheduler heartbeat must continue"
        );
    };
    // Join before releasing fixture files, including when an assertion panics.
    use futures_util::FutureExt;
    let outcome = std::panic::AssertUnwindSafe(result).catch_unwind().await;
    scheduler.abort();
    let _ = scheduler.await;
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if state
                .owned
                .session_store
                .get_session(holder_room.id())
                .unwrap()
                .active_interactions()
                .is_empty()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("scheduler shutdown must cancel the maintenance Vault wait");
    assert!(
        state
            .owned
            .durable_state_store
            .agent_tasks(Some(holder_room.id()), Some(holder.id()))
            .unwrap()
            .is_empty(),
        "cancelled maintenance must withdraw its unsubmitted notification task"
    );
    drop(file);
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}
