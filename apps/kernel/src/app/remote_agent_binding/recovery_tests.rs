use super::*;
use crate::session::CreateSessionRequest;
use chariox_relay::{RelayConfig, RelayServer};
use sha2::{Digest, Sha256};
use tokio::sync::{watch, Mutex};

struct Fixture {
    home: Arc<Mutex<DaemonApp>>,
    workers: Vec<Arc<Mutex<DaemonApp>>>,
    presences: Vec<RelayKernelPresence>,
    slices: Vec<crate::slice::SliceRecord>,
    agent_id: String,
    binding: RemoteAgentBinding,
    shutdown: Vec<watch::Sender<bool>>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    workspace_root: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for shutdown in &self.shutdown {
            let _ = shutdown.send(true);
        }
        for task in &self.tasks {
            task.abort();
        }
        let _ = std::fs::remove_dir_all(&self.workspace_root);
    }
}

impl Fixture {
    async fn new() -> Self {
        Self::with_slice_identity(true).await
    }

    async fn with_slice_identity(slice_identity: bool) -> Self {
        Self::with_options(slice_identity, false).await
    }

    async fn with_options(slice_identity: bool, hostile_alias: bool) -> Self {
        Self::with_worker_options(slice_identity, hostile_alias, false).await
    }

    async fn with_worker_options(
        slice_identity: bool,
        hostile_alias: bool,
        only_second_slice: bool,
    ) -> Self {
        let server = RelayServer::new(RelayConfig {
            host: "127.0.0.1".into(),
            port: 0,
            shared_token: Some("recovery-fixture".into()),
        });
        let listener = server.bind_listener().await.unwrap();
        let address = listener.local_addr().unwrap();
        let registry = server.registry();
        let relay_url = format!("ws://{address}");
        let mut tasks = vec![tokio::spawn(async move {
            server
                .run_listener_until(listener, std::future::pending::<()>())
                .await
                .unwrap();
        })];
        let fixture_id = format!("{:032x}", rand::random::<u128>());
        let workspace_root =
            std::env::temp_dir().join(format!("chariox-slice-recovery-{fixture_id}"));
        let workspace_a = workspace_root.join("a");
        let workspace_b = workspace_root.join("b");
        std::fs::create_dir_all(&workspace_a).unwrap();
        std::fs::create_dir_all(&workspace_b).unwrap();
        let mut config = DaemonConfig::for_tests();
        config.daemon_id = format!("recovery-home-{fixture_id}");
        config.host_machine_id = format!("shared-parent-machine-{fixture_id}");
        config.relay_url = Some(relay_url.clone());
        config.relay_token = Some("recovery-fixture".into());
        config.relay_request_timeout_ms = 2_000;
        let mut home = DaemonApp::bootstrap(config.clone()).unwrap();
        let (room_a, agent) = crate::app::KernelSessionService::new(&mut home)
            .create_session(CreateSessionRequest::new(
                workspace_a.display().to_string(),
                workspace_a.display().to_string(),
            ))
            .unwrap();
        let (room_b, _) = crate::app::KernelSessionService::new(&mut home)
            .create_session(CreateSessionRequest::new(
                workspace_b.display().to_string(),
                workspace_b.display().to_string(),
            ))
            .unwrap();
        let mut workers = Vec::new();
        let mut shutdown = Vec::new();
        let mut presences = Vec::new();
        let mut slices = Vec::new();
        let mut canonical_a = String::new();
        for (index, room) in [room_a.id(), room_b.id()].into_iter().enumerate() {
            let mut worker_config = DaemonConfig::for_tests();
            worker_config.daemon_id = if slice_identity && (!only_second_slice || index == 1) {
                format!(
                    "slice:{:x}:{}{fixture_id}",
                    Sha256::digest(config.host_machine_id.as_bytes()),
                    if index == 0 {
                        "f".repeat(32)
                    } else {
                        "0".repeat(32)
                    }
                )
            } else {
                format!("ordinary-worker-{fixture_id}-{index}")
            };
            if index == 0 {
                canonical_a = worker_config.daemon_id.clone();
            }
            worker_config.daemon_alias = Some(if hostile_alias && index == 1 {
                canonical_a.clone()
            } else {
                format!("slice:room-{index}")
            });
            worker_config.host_machine_id = config.host_machine_id.clone();
            worker_config.relay_url = Some(relay_url.clone());
            worker_config.relay_token = config.relay_token.clone();
            worker_config.relay_heartbeat_ms = 50;
            let presence = RelayKernelPresence {
                kernel_id: worker_config.daemon_id.clone(),
                machine_id: config.host_machine_id.clone(),
                machine_alias: None,
                relay_alias: None,
                kernel_alias: worker_config.daemon_alias.clone(),
                available_providers: vec![agent.provider().into()],
                provider_accounts: Vec::new(),
                capabilities: Vec::new(),
                accepting_remote_leases: true,
                leased_agent_count: if index == 0 { 8 } else { 0 },
                local_session_count: 0,
                public_key: worker_config.relay_public_key.clone(),
            };
            let worker = Arc::new(Mutex::new(DaemonApp::bootstrap(worker_config).unwrap()));
            let state = worker.lock().await.relay_client_state();
            let (tx, rx) = watch::channel(false);
            tasks.push(tokio::spawn(
                crate::transport::relay_client::run_daemon_relay_connector(
                    worker.clone(),
                    state,
                    rx,
                ),
            ));
            shutdown.push(tx);
            let slice = home
                .slices()
                .create(
                    &config.daemon_id,
                    &config.host_machine_id,
                    crate::slice::CreateSliceInput {
                        source_slice_ref: None,
                        name: format!("room-{index}"),
                        backend: crate::slice::SliceBackendKind::LocalDocker,
                        os: "linux".into(),
                        display_mode: crate::slice::SliceDisplayMode::Headed,
                        display_backend: Default::default(),
                        workspace_id: None,
                        worktree_id: None,
                        workspace_mount: Some("/workspace".into()),
                        development: None,
                        worker_kernel_ref: Some(presence.kernel_id.clone()),
                        display_url: None,
                        provider_auth: Vec::new(),
                        from_saved_state: None,
                        now_ms: 1,
                    },
                )
                .unwrap();
            home.slices()
                .set_worker_presence(
                    &slice.id,
                    Some(presence.kernel_id.clone()),
                    Some(config.host_machine_id.clone()),
                    vec![agent.provider().into()],
                    2,
                )
                .unwrap();
            home.slices()
                .set_relay_endpoint(
                    &slice.id,
                    Some(crate::slice::SliceRelayEndpoint {
                        url: relay_url.clone(),
                        private: false,
                    }),
                    2,
                )
                .unwrap();
            home.slices()
                .set_development_publication(
                    &slice.id,
                    crate::slice::SliceDevelopmentPublication {
                        publication_id: format!("recovery-{index}"),
                        destination_root: workspace_root.display().to_string(),
                        primary_repository_path: if index == 0 {
                            workspace_a.display().to_string()
                        } else {
                            workspace_b.display().to_string()
                        },
                        repository_paths: Vec::new(),
                    },
                    2,
                )
                .unwrap();
            home.slices()
                .bind_environment(room, &slice.id, 3, |_| Ok(()))
                .unwrap();
            workers.push(worker);
            presences.push(presence);
            slices.push(slice);
        }
        for presence in &presences {
            tokio::time::timeout(Duration::from_secs(3), async {
                while registry.read().await.daemon(&presence.kernel_id).is_none() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("worker should register");
        }
        home.remote_relay_inventory_projection_store()
            .update(Vec::new(), presences.clone());
        let binding = RemoteAgentBinding {
            worker_kernel_id: presences[0].kernel_id.clone(),
            worker_machine_id: config.host_machine_id,
            execution_lease_id: "missing-original-lease".into(),
            leased_agent_id: "missing-original-agent".into(),
            active_worker_provider_run_id: None,
            relay_url: None,
            relay_token: None,
            relay_peer_protocol_version: Some(RELAY_PEER_PROTOCOL_VERSION),
        };
        home.agents()
            .bind_remote_execution(agent.id(), binding.clone())
            .unwrap();
        Self {
            home: Arc::new(Mutex::new(home)),
            workers,
            presences,
            slices,
            agent_id: agent.id().into(),
            binding,
            shutdown,
            tasks,
            workspace_root,
        }
    }

    async fn assert_foreign_worker_untouched(&self) {
        let worker = self.workers[1].lock().await;
        assert_eq!(
            worker.next_execution_lease_number, 0,
            "recovery must never send lease creation to another Room"
        );
        assert!(
            worker.execution_leases.is_empty(),
            "recovery contacted the lower-load worker owned by another Room"
        );
        assert!(worker.leased_agents.is_empty());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn slice_recovery_off_lock_keeps_recorded_worker_when_other_room_is_less_loaded() {
    let fixture = Fixture::new().await;
    let plan = fixture
        .home
        .lock()
        .await
        .prepare_remote_agent_binding_refresh(&fixture.agent_id, &fixture.binding)
        .unwrap();
    let refresh = DaemonApp::execute_remote_agent_binding_refresh(plan)
        .await
        .unwrap();
    assert_eq!(
        refresh.remote_execution.worker_kernel_id, fixture.binding.worker_kernel_id,
        "missing A lease must be recreated on A"
    );
    fixture.assert_foreign_worker_untouched().await;
    let slices = fixture.home.lock().await.slices();
    assert!(
        slices
            .try_begin_operation(&fixture.slices[0].id, "slice.state.save")
            .is_err(),
        "admission guard must survive network I/O until commit"
    );
    let (result, committed) = fixture
        .home
        .lock()
        .await
        .commit_remote_agent_binding_refresh(&refresh);
    assert!(committed);
    assert_eq!(
        result.unwrap().remote_execution().unwrap().worker_kernel_id,
        fixture.binding.worker_kernel_id
    );
    drop(refresh);
    assert!(
        slices
            .try_begin_operation(&fixture.slices[0].id, "slice.state.save")
            .is_ok(),
        "commit completion releases admission"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn slice_recovery_synchronous_keeps_recorded_worker_when_other_room_is_less_loaded() {
    let fixture = Fixture::new().await;
    let rebound = fixture
        .home
        .lock()
        .await
        .refresh_remote_agent_binding(&fixture.agent_id)
        .unwrap();
    assert_eq!(
        rebound.remote_execution().unwrap().worker_kernel_id,
        fixture.binding.worker_kernel_id
    );
    fixture.assert_foreign_worker_untouched().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn slice_recovery_explicit_worker_refuses_another_room() {
    let fixture = Fixture::new().await;
    let result = fixture
        .home
        .lock()
        .await
        .refresh_remote_agent_binding_to_worker_kernel(&fixture.agent_id, &fixture.presences[1]);
    assert!(result.is_err(), "explicit recovery cannot move A to B");
    fixture.assert_foreign_worker_untouched().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn slice_recovery_prepare_refuses_operation_in_progress_and_foreign_room() {
    let fixture = Fixture::new().await;
    let slices = fixture.home.lock().await.slices();
    let guard = slices
        .try_begin_operation(&fixture.slices[0].id, "slice.state.save")
        .unwrap();
    assert!(
        fixture
            .home
            .lock()
            .await
            .prepare_remote_agent_binding_refresh(&fixture.agent_id, &fixture.binding)
            .is_err(),
        "save must fence recovery before any peer request"
    );
    drop(guard);
    let mut records = slices.list();
    records
        .iter_mut()
        .find(|record| record.id == fixture.slices[0].id)
        .unwrap()
        .environment_session_id = Some("foreign-room".into());
    slices.restore_records(records);
    assert!(
        fixture
            .home
            .lock()
            .await
            .prepare_remote_agent_binding_refresh(&fixture.agent_id, &fixture.binding)
            .is_err(),
        "foreign Room must fence recovery"
    );
    fixture.assert_foreign_worker_untouched().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn slice_recovery_missing_record_cannot_fall_back_to_parent_machine() {
    let fixture = Fixture::new().await;
    let slices = fixture.home.lock().await.slices();
    slices.delete(&fixture.slices[0].id).unwrap();
    assert!(
        fixture
            .home
            .lock()
            .await
            .prepare_remote_agent_binding_refresh(&fixture.agent_id, &fixture.binding)
            .is_err(),
        "missing canonical slice record is not ordinary Machine placement"
    );
    assert!(fixture
        .home
        .lock()
        .await
        .refresh_remote_agent_binding(&fixture.agent_id)
        .is_err());
    fixture.assert_foreign_worker_untouched().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn slice_recovery_off_lock_unavailable_worker_cannot_fail_over_to_other_room() {
    let fixture = Fixture::new().await;
    let mut presences = fixture.presences.clone();
    presences[0].accepting_remote_leases = false;
    fixture
        .home
        .lock()
        .await
        .remote_relay_inventory_projection_store()
        .update(Vec::new(), presences);
    let plan = fixture
        .home
        .lock()
        .await
        .prepare_remote_agent_binding_refresh(&fixture.agent_id, &fixture.binding)
        .unwrap();
    assert!(DaemonApp::execute_remote_agent_binding_refresh(plan)
        .await
        .is_err());
    fixture.assert_foreign_worker_untouched().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn slice_recovery_synchronous_unavailable_worker_cannot_fail_over_to_other_room() {
    let fixture = Fixture::new().await;
    let mut presences = fixture.presences.clone();
    presences[0].accepting_remote_leases = false;
    fixture
        .home
        .lock()
        .await
        .remote_relay_inventory_projection_store()
        .update(Vec::new(), presences);
    assert!(fixture
        .home
        .lock()
        .await
        .refresh_remote_agent_binding(&fixture.agent_id)
        .is_err());
    fixture.assert_foreign_worker_untouched().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn slice_recovery_ordinary_machine_failover_still_uses_eligible_lower_load_worker() {
    let fixture = Fixture::with_slice_identity(false).await;
    fixture
        .home
        .lock()
        .await
        .slices()
        .restore_records(Vec::new());
    let rebound = fixture
        .home
        .lock()
        .await
        .refresh_remote_agent_binding(&fixture.agent_id)
        .unwrap();
    assert_eq!(
        rebound.remote_execution().unwrap().worker_kernel_id,
        fixture.presences[1].kernel_id
    );
    assert_eq!(fixture.workers[1].lock().await.leased_agents.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn slice_recovery_prepare_holds_room_admission_until_plan_is_dropped() {
    let fixture = Fixture::new().await;
    let slices = fixture.home.lock().await.slices();
    let plan = fixture
        .home
        .lock()
        .await
        .prepare_remote_agent_binding_refresh(&fixture.agent_id, &fixture.binding)
        .unwrap();
    assert!(slices
        .try_begin_operation(&fixture.slices[0].id, "slice.state.save")
        .is_err());
    drop(plan);
    assert!(slices
        .try_begin_operation(&fixture.slices[0].id, "slice.state.save")
        .is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn slice_recovery_foreign_room_rejected_by_every_entry_point() {
    let fixture = Fixture::new().await;
    let slices = fixture.home.lock().await.slices();
    let mut records = slices.list();
    records
        .iter_mut()
        .find(|record| record.id == fixture.slices[0].id)
        .unwrap()
        .environment_session_id = Some("foreign-room".into());
    slices.restore_records(records);
    let mut home = fixture.home.lock().await;
    assert!(home
        .prepare_remote_agent_binding_refresh(&fixture.agent_id, &fixture.binding)
        .is_err());
    assert!(home
        .refresh_remote_agent_binding(&fixture.agent_id)
        .is_err());
    assert!(home
        .refresh_remote_agent_binding_to_worker_kernel(&fixture.agent_id, &fixture.presences[0])
        .is_err());
    drop(home);
    assert_eq!(
        fixture.workers[0].lock().await.next_execution_lease_number,
        0
    );
    fixture.assert_foreign_worker_untouched().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn slice_recovery_existing_start_guard_admits_only_its_slice_and_room() {
    let fixture = Fixture::new().await;
    let slices = fixture.home.lock().await.slices();
    let wrong_guard = slices
        .try_begin_operation(&fixture.slices[1].id, "slice.start")
        .unwrap();
    assert!(fixture
        .home
        .lock()
        .await
        .refresh_remote_agent_binding_to_worker_kernel_with_operation(
            &fixture.agent_id,
            &fixture.presences[0],
            Some(&wrong_guard)
        )
        .is_err());
    drop(wrong_guard);
    let guard = slices
        .try_begin_operation(&fixture.slices[0].id, "slice.start")
        .unwrap();
    assert!(fixture
        .home
        .lock()
        .await
        .refresh_remote_agent_binding_to_worker_kernel(&fixture.agent_id, &fixture.presences[0])
        .is_err());
    assert!(fixture
        .home
        .lock()
        .await
        .refresh_remote_agent_binding_to_worker_kernel_with_operation(
            &fixture.agent_id,
            &fixture.presences[1],
            Some(&guard)
        )
        .is_err());
    assert_eq!(
        fixture.workers[0].lock().await.next_execution_lease_number,
        0
    );
    fixture.assert_foreign_worker_untouched().await;
    let rebound = fixture
        .home
        .lock()
        .await
        .refresh_remote_agent_binding_to_worker_kernel_with_operation(
            &fixture.agent_id,
            &fixture.presences[0],
            Some(&guard),
        )
        .unwrap();
    assert_eq!(
        rebound.remote_execution().unwrap().worker_kernel_id,
        fixture.binding.worker_kernel_id
    );
    assert_eq!(
        fixture.workers[0].lock().await.next_execution_lease_number,
        1
    );
    fixture.assert_foreign_worker_untouched().await;
    drop(guard);
    assert!(slices
        .guard_environment_use(
            &fixture.slices[0].id,
            Some(rebound.session_id()),
            "slice.agent.recover"
        )
        .is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn slice_recovery_existing_start_guard_cannot_admit_foreign_room() {
    let fixture = Fixture::new().await;
    let slices = fixture.home.lock().await.slices();
    let guard = slices
        .try_begin_operation(&fixture.slices[0].id, "slice.start")
        .unwrap();
    let mut records = slices.list();
    records
        .iter_mut()
        .find(|record| record.id == fixture.slices[0].id)
        .unwrap()
        .environment_session_id = Some("foreign-room".into());
    slices.restore_records(records);
    assert!(fixture
        .home
        .lock()
        .await
        .refresh_remote_agent_binding_to_worker_kernel_with_operation(
            &fixture.agent_id,
            &fixture.presences[0],
            Some(&guard)
        )
        .is_err());
    assert_eq!(
        fixture.workers[0].lock().await.next_execution_lease_number,
        0
    );
    fixture.assert_foreign_worker_untouched().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn slice_selector_projected_alias_cannot_shadow_canonical_worker() {
    let fixture = Fixture::with_options(true, true).await;
    let home = fixture.home.lock().await;
    home.remote_relay_inventory_projection_store().update(
        Vec::new(),
        vec![fixture.presences[1].clone(), fixture.presences[0].clone()],
    );
    let selected = home
        .select_remote_kernel_by_ref_with_config(
            &fixture.presences[0].kernel_id,
            &fixture.presences[0].available_providers[0],
            home.config(),
        )
        .unwrap();
    assert_eq!(
        selected.kernel_id, fixture.presences[0].kernel_id,
        "foreign worker alias must not override the persisted canonical slice ID"
    );
    drop(home);
    fixture.assert_foreign_worker_untouched().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn slice_selector_live_alias_cannot_shadow_canonical_worker() {
    let fixture = Fixture::with_options(true, true).await;
    let home = fixture.home.lock().await;
    home.remote_relay_inventory_projection_store()
        .update(Vec::new(), Vec::new());
    let actual_metadata = home
        .block_on_relay_future(relay_discovery::get_live_kernel(
            home.config(),
            &fixture.presences[0].kernel_id,
        ))
        .unwrap();
    assert_eq!(
        actual_metadata.kernel_id, fixture.presences[1].kernel_id,
        "fixture must reproduce relay metadata alias shadowing"
    );
    let result = home.select_remote_kernel_by_ref_with_config(
        &fixture.presences[0].kernel_id,
        "dev-stub",
        home.config(),
    );
    assert!(
        matches!(
            result,
            Err(DaemonError::LocalTransport {
                operation: "select slice worker",
                ..
            })
        ),
        "canonical metadata mismatch must be rejected, got {result:?}"
    );
    drop(home);
    fixture.assert_foreign_worker_untouched().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn slice_selector_ordinary_explicit_alias_remains_supported() {
    let fixture = Fixture::with_slice_identity(false).await;
    let home = fixture.home.lock().await;
    home.slices().restore_records(Vec::new());
    let selected = home
        .select_remote_kernel_by_ref_with_config(
            "slice:room-1",
            &fixture.presences[1].available_providers[0],
            home.config(),
        )
        .unwrap();
    assert_eq!(selected.kernel_id, fixture.presences[1].kernel_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn slice_selector_move_to_canonical_worker_does_not_treat_it_as_machine() {
    let fixture = Fixture::new().await;
    let mut home = fixture.home.lock().await;
    let agent = home
        .agents()
        .clear_remote_execution(&fixture.agent_id)
        .unwrap();
    let moved = home
        .move_agent_to_remote(
            agent.session_id(),
            agent.id(),
            &fixture.presences[0].kernel_id,
        )
        .unwrap();
    assert_eq!(
        moved.remote_execution().unwrap().worker_kernel_id,
        fixture.presences[0].kernel_id
    );
    drop(home);
    fixture.assert_foreign_worker_untouched().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn slice_selector_recovery_refuses_observed_id_disagreeing_with_persisted_identity() {
    let fixture = Fixture::new().await;
    let mut home = fixture.home.lock().await;
    let mut records = home.slices().list();
    records
        .iter_mut()
        .find(|record| record.id == fixture.slices[0].id)
        .unwrap()
        .worker_kernel_id = Some("forged-observed-id".into());
    home.slices().restore_records(records);
    let mut binding = fixture.binding.clone();
    binding.worker_kernel_id = "forged-observed-id".into();
    home.agents()
        .bind_remote_execution(&fixture.agent_id, binding.clone())
        .unwrap();
    assert!(
        home.prepare_remote_agent_binding_refresh(&fixture.agent_id, &binding)
            .is_err(),
        "persisted canonical identity must win over previously observed metadata"
    );
    drop(home);
    fixture.assert_foreign_worker_untouched().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn slice_recovery_off_lock_ordinary_machine_failover_still_uses_lower_load_worker() {
    let fixture = Fixture::with_slice_identity(false).await;
    fixture
        .home
        .lock()
        .await
        .slices()
        .restore_records(Vec::new());
    let plan = fixture
        .home
        .lock()
        .await
        .prepare_remote_agent_binding_refresh(&fixture.agent_id, &fixture.binding)
        .unwrap();
    let refresh = DaemonApp::execute_remote_agent_binding_refresh(plan)
        .await
        .unwrap();
    assert_eq!(
        refresh.remote_execution.worker_kernel_id,
        fixture.presences[1].kernel_id
    );
    let (result, committed) = fixture
        .home
        .lock()
        .await
        .commit_remote_agent_binding_refresh(&refresh);
    assert!(committed);
    assert_eq!(
        result.unwrap().remote_execution().unwrap().worker_kernel_id,
        fixture.presences[1].kernel_id
    );
    assert_eq!(
        fixture.workers[1].lock().await.next_execution_lease_number,
        1
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn slice_machine_selection_cannot_move_ordinary_agent_into_room_slice() {
    let fixture = Fixture::with_worker_options(true, false, true).await;
    let mut home = fixture.home.lock().await;
    home.slices()
        .restore_records(vec![home.slices().resolve(&fixture.slices[1].id).unwrap()]);
    let agent = home
        .agents()
        .clear_remote_execution(&fixture.agent_id)
        .unwrap();
    let moved = home
        .move_agent_to_remote(
            agent.session_id(),
            agent.id(),
            &fixture.binding.worker_machine_id,
        )
        .unwrap();
    assert_eq!(
        moved.remote_execution().unwrap().worker_kernel_id,
        fixture.presences[0].kernel_id,
        "generic Machine placement must exclude the lower-load Room slice"
    );
    drop(home);
    fixture.assert_foreign_worker_untouched().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn slice_machine_selection_off_lock_cannot_fail_over_into_room_slice() {
    let fixture = Fixture::with_worker_options(true, false, true).await;
    let home = fixture.home.lock().await;
    home.slices()
        .restore_records(vec![home.slices().resolve(&fixture.slices[1].id).unwrap()]);
    let plan = home
        .prepare_remote_agent_binding_refresh(&fixture.agent_id, &fixture.binding)
        .unwrap();
    drop(home);
    let refresh = DaemonApp::execute_remote_agent_binding_refresh(plan)
        .await
        .unwrap();
    assert_eq!(
        refresh.remote_execution.worker_kernel_id, fixture.presences[0].kernel_id,
        "ordinary recovery cannot choose the lower-load Room slice"
    );
    fixture.assert_foreign_worker_untouched().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn slice_machine_selection_synchronous_excludes_recorded_custom_ssh_worker() {
    let fixture = Fixture::with_slice_identity(false).await;
    let mut home = fixture.home.lock().await;
    let mut slice = home.slices().resolve(&fixture.slices[1].id).unwrap();
    slice.backend = crate::slice::SliceBackendKind::SshDocker;
    home.slices().restore_records(vec![slice]);
    let refreshed = home
        .refresh_remote_agent_binding(&fixture.agent_id)
        .unwrap();
    assert_eq!(
        refreshed.remote_execution().unwrap().worker_kernel_id,
        fixture.presences[0].kernel_id,
        "recorded custom slice identity must also be excluded from generic Machine placement"
    );
    drop(home);
    fixture.assert_foreign_worker_untouched().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn slice_machine_selection_off_lock_excludes_recorded_custom_ssh_worker() {
    let fixture = Fixture::with_slice_identity(false).await;
    let home = fixture.home.lock().await;
    let mut slice = home.slices().resolve(&fixture.slices[1].id).unwrap();
    slice.backend = crate::slice::SliceBackendKind::SshDocker;
    home.slices().restore_records(vec![slice]);
    let plan = home
        .prepare_remote_agent_binding_refresh(&fixture.agent_id, &fixture.binding)
        .unwrap();
    drop(home);
    let refresh = DaemonApp::execute_remote_agent_binding_refresh(plan)
        .await
        .unwrap();
    assert_eq!(
        refresh.remote_execution.worker_kernel_id,
        fixture.presences[0].kernel_id
    );
    fixture.assert_foreign_worker_untouched().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn slice_recovery_recorded_machine_disagreement_is_rejected_before_contact() {
    let fixture = Fixture::new().await;
    let mut home = fixture.home.lock().await;
    let mut records = home.slices().list();
    records
        .iter_mut()
        .find(|record| record.id == fixture.slices[0].id)
        .unwrap()
        .worker_machine_id = Some("different-recorded-machine".into());
    home.slices().restore_records(records);
    assert!(home
        .prepare_remote_agent_binding_refresh(&fixture.agent_id, &fixture.binding)
        .is_err());
    assert!(home
        .refresh_remote_agent_binding(&fixture.agent_id)
        .is_err());
    assert!(home
        .refresh_remote_agent_binding_to_worker_kernel(&fixture.agent_id, &fixture.presences[0])
        .is_err());
    drop(home);
    assert_eq!(
        fixture.workers[0].lock().await.next_execution_lease_number,
        0
    );
    fixture.assert_foreign_worker_untouched().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn slice_machine_selection_reserved_worker_without_local_record_is_excluded() {
    let fixture = Fixture::with_worker_options(true, false, true).await;
    let mut home = fixture.home.lock().await;
    home.slices().restore_records(Vec::new());
    let agent = home
        .agents()
        .clear_remote_execution(&fixture.agent_id)
        .unwrap();
    let moved = home
        .move_agent_to_remote(
            agent.session_id(),
            agent.id(),
            &fixture.binding.worker_machine_id,
        )
        .unwrap();
    assert_eq!(
        moved.remote_execution().unwrap().worker_kernel_id,
        fixture.presences[0].kernel_id
    );
    drop(home);
    fixture.assert_foreign_worker_untouched().await;
}
