//! MP-08 / MP-10: after a worker restart on the same ports the home's remote agent holds
//! a lease the worker no longer knows. Skills requests recover it like a plain prompt does.
//! Real local relay and worker kernel with the supplementary managed-dev-stub provider.
use super::support::*;
use crate::skill::{storage::SkillCatalogStorage, v2::*};

struct StaleLeaseFixture {
    home: Arc<Mutex<DaemonApp>>,
    worker: Arc<Mutex<DaemonApp>>,
    router: crate::runtime::router::CommandRouter,
    session_id: String,
    project_id: String,
    attachment_id: String,
    agent_id: String,
    stale_leased_agent_id: String,
    skill: SkillRecord,
    revision: u64,
    stop_relay: oneshot::Sender<()>,
    relay_task: tokio::task::JoinHandle<()>,
    stop_worker: watch::Sender<bool>,
    worker_task: tokio::task::JoinHandle<()>,
    _worktree: crate::test_support::TestWorktree,
}

impl StaleLeaseFixture {
    async fn new() -> Self {
        let server = RelayServer::new(RelayConfig {
            host: "127.0.0.1".into(),
            port: 0,
            shared_token: Some("skills-stale-lease".into()),
        });
        let listener = server.bind_listener().await.unwrap();
        let addr = listener.local_addr().unwrap();
        let registry = server.registry();
        let (stop_relay, stop_rx) = oneshot::channel();
        let relay_task = tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = stop_rx.await;
                })
                .await
                .unwrap();
        });
        let mut config = DaemonConfig::for_tests();
        config.daemon_id = "skills-stale-worker".into();
        config.host_machine_id = "skills-stale-machine".into();
        config.relay_url = Some(format!("ws://{addr}"));
        config.relay_token = Some("skills-stale-lease".into());
        config.accept_remote_leases = true;
        let worker = Arc::new(Mutex::new(DaemonApp::bootstrap(config.clone()).unwrap()));
        let worker_state = worker.lock().await.relay_client_state();
        let (stop_worker, worker_stop_rx) = watch::channel(false);
        let worker_task = tokio::spawn(run_daemon_relay_connector(
            worker.clone(),
            worker_state,
            worker_stop_rx,
        ));
        wait_for_daemon_registration(registry, &config.daemon_id).await;
        let presence =
            relay_discovery::list_live_kernels_for_machine(&config, &config.host_machine_id)
                .await
                .unwrap()
                .into_iter()
                .find(|k| k.kernel_id == config.daemon_id)
                .unwrap();
        let mut home_config = DaemonConfig::for_tests();
        home_config.relay_url = config.relay_url.clone();
        home_config.relay_token = config.relay_token.clone();
        let skills_home = home_config.user_config_path.parent().unwrap().to_path_buf();
        let mut home = DaemonApp::bootstrap(home_config).unwrap();
        let worktree = crate::test_support::TestWorktree::new("skills-stale-lease");
        let (session, agent) = crate::app::KernelSessionService::new(&mut home)
            .create_session(worktree.session_request().with_agent_defaults(
                crate::session::SessionAgentDefaults {
                    provider: "managed-dev-stub".into(),
                    model: Some("native-tui-idle".into()),
                    ..Default::default()
                },
            ))
            .unwrap();
        let attachment = crate::app::KernelSessionService::new(&mut home)
            .attach(AttachRequest::new(
                session.id(),
                "skills-stale-client",
                ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        let service = crate::skill::service::SkillCatalogService::new(
            SkillCatalogStorage::new(&skills_home, home.durable_state_store()).unwrap(),
        );
        let mutation = SkillCatalogMutation {
            scope: SkillScope::Chariox,
            expected_revision: 0,
            actions: vec![SkillCatalogAction::Install {
                source: SkillInstallSource::Content {
                    skill_md:
                        "---\nname: review\ndescription: Review safely\n---\nReview source.\n"
                            .into(),
                },
                collision: SkillCollisionAction::Reject,
                package_id: None,
            }],
        };
        let prepared = service.prepare(&mutation).unwrap();
        let catalog = service.publish_authorized(&mutation, &prepared).unwrap();
        let skill = catalog.catalogs[0].skills[0].clone();
        home.agents()
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: config.daemon_id.clone(),
                    worker_machine_id: config.host_machine_id.clone(),
                    execution_lease_id: "unleased-execution".into(),
                    leased_agent_id: "unleased-agent".into(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .unwrap();
        // A genuine lease, then the worker restarts and forgets it.
        let bound = home
            .refresh_remote_agent_binding_to_worker_kernel(agent.id(), &presence)
            .unwrap();
        let stale_leased_agent_id = bound.remote_execution().unwrap().leased_agent_id.clone();
        RemoteLeaseRuntime::new(&mut *worker.lock().await).forget_live_leases_for_test();
        let session_id = session.id().to_string();
        let project_id = session.project_id().to_string();
        let home = Arc::new(Mutex::new(home));
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            home.clone(),
            crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
        );
        Self {
            home,
            worker,
            router,
            session_id,
            project_id,
            attachment_id: attachment.id().to_string(),
            agent_id: agent.id().to_string(),
            stale_leased_agent_id,
            skill,
            revision: catalog.revision,
            stop_relay,
            relay_task,
            stop_worker,
            worker_task,
            _worktree: worktree,
        }
    }

    async fn skills(&self, request: SkillsV2Request) -> Result<LocalDaemonResponse, DaemonError> {
        let request = LocalDaemonRequest::SkillsV2(request);
        let source = crate::runtime::command::KernelCommandSource::LocalCli;
        let caller = crate::runtime::command::KernelCaller::for_source(&source)
            .with_connection_class(crate::local::KernelConnectionClass::Terminal);
        let command = KernelCommand::from_local_request_with_caller(
            "skills-stale",
            source,
            caller,
            None,
            None,
            &request,
        );
        self.router.dispatch(command, request).await
    }

    async fn leased_agent_id(&self) -> String {
        self.home
            .lock()
            .await
            .agents()
            .get_agent(&self.agent_id)
            .unwrap()
            .remote_execution()
            .unwrap()
            .leased_agent_id
            .clone()
    }

    async fn finish(self) {
        let _ = self.stop_worker.send(true);
        self.worker_task.await.unwrap();
        self.worker
            .lock()
            .await
            .teardown_provider_processes(Some("managed-dev-stub"), true)
            .unwrap();
        let _ = self.stop_relay.send(());
        self.relay_task.await.unwrap();
    }
}

#[test]
fn skills_picker_catalog_recovers_a_lease_the_restarted_worker_forgot() {
    run_async_with_large_test_stack("skills-stale-lease-catalog", || async {
        let _relay_guard = relay_client_test_guard().await;
        let _home = RelayTestHome::new();
        let fixture = StaleLeaseFixture::new().await;
        let response = fixture
            .skills(SkillsV2Request::Catalog {
                project_id: Some(fixture.project_id.clone()),
                agent_id: Some(fixture.agent_id.clone()),
            })
            .await;
        let rebound = fixture.leased_agent_id().await;
        let stale = fixture.stale_leased_agent_id.clone();
        fixture.finish().await;
        assert!(
            matches!(&response, Ok(LocalDaemonResponse::SkillsV2 {
                result: SkillsV2Response::Catalog { skills, .. }
            }) if skills.len() == 1),
            "the picker must list skills after a worker restart: {response:?}"
        );
        assert_ne!(rebound, stale, "the stale lease must be replaced");
    });
}

#[test]
fn skill_submit_recovers_a_lease_the_restarted_worker_forgot() {
    run_async_with_large_test_stack("skills-stale-lease-submit", || async {
        let _relay_guard = relay_client_test_guard().await;
        let _home = RelayTestHome::new();
        let fixture = StaleLeaseFixture::new().await;
        let response = fixture
            .skills(SkillsV2Request::Submit {
                session_id: fixture.session_id.clone(),
                attachment_id: fixture.attachment_id.clone(),
                target_agent_id: fixture.agent_id.clone(),
                prompt: "$review inspect source".into(),
                attachments: vec![],
                selections: vec![SkillSelection::Chariox {
                    skill_id: fixture.skill.id.clone(),
                    name: fixture.skill.name.clone(),
                    catalog_revision: fixture.revision,
                }],
            })
            .await;
        let mut accepted = false;
        for _ in 0..1600 {
            let agent = fixture
                .home
                .lock()
                .await
                .agents()
                .get_agent(&fixture.agent_id)
                .unwrap();
            if agent
                .remote_execution()
                .is_some_and(|b| b.active_worker_provider_run_id.is_some())
            {
                accepted = true;
                break;
            }
            sleep(Duration::from_millis(25)).await;
        }
        let rebound = fixture.leased_agent_id().await;
        let stale = fixture.stale_leased_agent_id.clone();
        fixture.finish().await;
        assert!(
            response.is_ok(),
            "skill submit admission failed: {response:?}"
        );
        assert!(accepted, "the skill turn must reach the recreated lease");
        assert_ne!(rebound, stale, "the stale lease must be replaced");
    });
}
