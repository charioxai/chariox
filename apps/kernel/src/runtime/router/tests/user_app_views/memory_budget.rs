//! MP-08 / MP-10: opt-in Linux measurement, not an acceptance or production-worker test.
//! Existing signed browser UI/fixed ABI workers; four distinct installation origins.
use super::*;
use futures_util::FutureExt;
use std::{path::Path, time::Duration};

#[test]
#[ignore = "MP-08 / MP-10: disposable home, native sandboxed Chromium or Docker slice required"]
fn app_view_memory_budget_drill() {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .thread_stack_size(32 * 1024 * 1024)
                .build()
                .unwrap()
                .block_on(run());
        })
        .unwrap()
        .join()
        .unwrap();
}

fn phase(root: &Path, topology: &str, views: usize, activity: &str) {
    let value = serde_json::json!({"mp_items":["MP-08","MP-10"],"pid":std::process::id(),
        "topology":topology,"views":views,"activity":activity,"at_ms":crate::session::unix_epoch_ms()});
    std::fs::write(root.join("phase.new"), serde_json::to_vec(&value).unwrap()).unwrap();
    std::fs::rename(root.join("phase.new"), root.join("phase.json")).unwrap();
}

async fn dispatch(router: &CommandRouter, value: serde_json::Value) -> LocalDaemonResponse {
    let req = serde_json::from_value(value).unwrap();
    request(router, "alice", req).await.unwrap()
}

async fn run() {
    let root = std::path::PathBuf::from(std::env::var_os("CHARIOX_APPS_MEMORY_ROOT").unwrap());
    let topology = std::env::var("CHARIOX_APPS_MEMORY_TOPOLOGY").unwrap();
    let native = topology == "host";
    assert!(native || topology == "slice");
    if native {
        assert_ne!(unsafe { libc::geteuid() }, 0);
    }
    let mut config = DaemonConfig::load_from_env();
    config.user_config_path = root.join("config.toml");
    config.local_socket_path = root.join("kernel.sock");
    config.user_config.state.path = Some(root.join("state.db").display().to_string());
    config.user_config.history.operational.path =
        Some(root.join("events.db").display().to_string());
    config.user_config.artifacts.operational.root =
        Some(root.join("artifacts").display().to_string());
    config.user_config.artifacts.operational.index_path =
        Some(root.join("artifacts.db").display().to_string());
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::ProcessMemory;
    config.user_config.slices.root = Some(root.join("slices").display().to_string());
    config.user_config.slices.linux.docker_image =
        Some(std::env::var("CHARIOX_APPS_MEMORY_IMAGE").unwrap());
    config.user_config.slices.linux.build_image = Some(crate::config::SliceImageBuildPolicy::Never);
    config.user_config.slices.linux.cpus = Some("2".into());
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let sessions = app.session_state_store();
    let room = if native {
        None
    } else {
        Some(
            crate::app::KernelSessionService::new(&mut app)
                .create_session(
                    CreateSessionRequest::new(root.to_string_lossy(), root.to_string_lossy())
                        .with_owner_user_id("alice"),
                )
                .unwrap()
                .0
                .id()
                .to_string(),
        )
    };
    let store = app.durable_state_store();
    let (first, bytes, publisher) =
        crate::durable_state::app_state::fixture_browser_tool_package(&store);
    let package = verify(
        &bytes,
        &VerificationPolicy::new(LOCAL_DAEMON_PROTOCOL_VERSION, vec![publisher]),
    )
    .unwrap();
    ReleaseStore::open_or_create(store.path())
        .unwrap()
        .stage(
            &package,
            &bytes,
            StageBudget {
                max_stage_bytes: 1024 * 1024,
                reserved_bytes: 1024 * 1024,
                host_reserve_bytes: 1024 * 1024,
            },
        )
        .unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let fixture = Fixture::compile().unwrap();
    let mut workers = Vec::new();
    let mut observations = Vec::new();
    let ids = ["installed", "memory-2", "memory-3", "memory-4"];
    for (index, id) in ids.iter().enumerate() {
        let catalog = if index == 0 {
            first.clone()
        } else {
            crate::durable_state::app_state::fixture_browser_installation(&store, id)
        };
        let (process, observed) = fixture
            .spawn_for_installation_blocking(Mode::ToolEcho, &package, id)
            .unwrap();
        let (starting, _) = crate::runtime::app_worker::AppWorkerOwner::start_blocking(
            process,
            &package,
            catalog,
            Arc::new(RejectBroker),
            PeerLimits::default(),
            tokio::runtime::Handle::current(),
        )
        .unwrap();
        let mut registered = starting
            .await_registered_blocking(Duration::from_secs(3))
            .unwrap();
        let proof = store
            .confirm_app_activation(
                "alice",
                registered.catalog().clone(),
                registered.take_activation_budget().unwrap(),
            )
            .unwrap();
        let (worker, handle) = registered.activate_blocking(proof).unwrap();
        router
            .runtime_state
            .app_control()
            .publish_app_worker("alice", handle)
            .unwrap();
        workers.push(worker);
        observations.push(observed);
    }
    let mut slice = None;
    let outcome = std::panic::AssertUnwindSafe(async {
        if let Some(room) = &room {
            let LocalDaemonResponse::SliceCreated { slice: created } = dispatch(&router,
                serde_json::json!({"CreateSlice":{"name":"appsbudget","display_mode":"headed","base":"clean"}})).await else { panic!("slice create") };
            slice = Some(created.id.clone());
            std::fs::write(root.join("slice-id.txt"), &created.id).unwrap();
            dispatch(&router, serde_json::json!({"StartSlice":{"slice_ref":created.id}})).await;
            dispatch(&router, serde_json::json!({"BindRoomEnvironmentSlice":{"session_id":room,"slice_ref":created.id}})).await;
            dispatch(&router, serde_json::json!({"StartRoomEnvironment":{"session_id":room,"viewport":{
                "css_width":1280,"css_height":800,"device_scale_factor":1,"desktop_pixel_width":1280,"desktop_pixel_height":800}}})).await;
        } else {
            browser_request(&router, KernelBrowserCommand::Start).await;
            assert!(sessions.list_sessions().is_empty());
            assert!(router.runtime_state.list_slices().is_empty());
        }
        phase(&root, &topology, 0, "warmup");
        tokio::time::sleep(Duration::from_secs(3)).await;
        phase(&root, &topology, 0, "idle");
        tokio::time::sleep(Duration::from_secs(20)).await;
        let mut views = Vec::new();
        for (index, id) in ids.iter().enumerate() {
            if native {
                let LocalDaemonResponse::UserAppViewOpened { view, .. } = dispatch(&router,
                    serde_json::json!({"OpenUserAppView":{"installation_id":id,"host":"kernel_browser"}})).await else { panic!("user view open") };
                views.push(view);
            } else {
                dispatch(&router, serde_json::json!({"OpenAppView":{"session_id":room,"installation_id":id}})).await;
            }
            tokio::time::timeout(Duration::from_secs(10), async {
                while observations[index].tool_invocations() == 0 { tokio::time::sleep(Duration::from_millis(50)).await; }
            }).await.expect("page bridge acknowledgement");
            if [1,2,4].contains(&(index+1)) && (native || index == 3) {
                phase(&root, &topology, index+1, "warmup");
                tokio::time::sleep(Duration::from_secs(3)).await;
                phase(&root, &topology, index+1, "idle");
                tokio::time::sleep(Duration::from_secs(20)).await;
                phase(&root, &topology, index+1, "interacting");
                let until = std::time::Instant::now() + Duration::from_secs(20);
                while std::time::Instant::now() < until {
                    for view in &views {
                        let b = view.browser.as_ref().unwrap();
                        for input in [KernelBrowserInput::Click {x:80,y:125},
                            KernelBrowserInput::Text {text:"memory fixture".into()},
                            KernelBrowserInput::Key {key:"Enter".into()}] {
                            browser_request(&router, KernelBrowserCommand::Input {tab_id:b.tab_id.clone(),generation:b.generation,input}).await;
                        }
                    }
                    // Room calls exercise the exact worker bridge, without provider credentials.
                    if let Some(room) = &room {
                        for (n, id) in ids.iter().take(index+1).enumerate() {
                            let env = router.runtime_state.room_environment_snapshot(room).unwrap();
                            let tabs = env.tabs.iter().filter(|tab| tab.url.starts_with("https://app.")).collect::<Vec<_>>();
                            let tab = tabs[n];
                            let snapshot = router.runtime_state.capture_browser_environment_snapshot(room, &tab.tab_id).await.unwrap();
                            if let Some(node) = snapshot.accessibility_nodes.iter().find(|node| node.role == "textbox" && node.name == "Message") {
                                router.runtime_state.perform_browser_environment_locator_action(room, &node.element_ref,
                                    &format!("memory-{}-{}", id, crate::session::unix_epoch_ms()),
                                    crate::runtime::browser_controller_action::BrowserLocatorAction::Fill { text: "memory fixture".into(), append:false, submit:false, expected_document_url:None }, 5000).await.unwrap();
                            }
                            let snapshot = router.runtime_state.capture_browser_environment_snapshot(room, &tab.tab_id).await.unwrap();
                            let button = snapshot.accessibility_nodes.iter().find(|node| node.role == "button" && node.name == "Call App").unwrap();
                            router.runtime_state.perform_browser_environment_locator_action(room, &button.element_ref,
                                &format!("memory-click-{}-{}", id, crate::session::unix_epoch_ms()),
                                crate::runtime::browser_controller_action::BrowserLocatorAction::Click, 5000).await.unwrap();
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
        }
        if native {
            let mut mirrors = Vec::new();
            for view in &views {
                let b = view.browser.as_ref().unwrap();
                let m = browser_request(&router, KernelBrowserCommand::MirrorSubscribe {
                    tab_id:b.tab_id.clone(), generation:b.generation, device_scale_factor:1,
                }).await;
                mirrors.push((m["subscription_id"].as_str().unwrap().to_string(), b.generation));
            }
            phase(&root, &topology, 4, "mirror_warmup");
            tokio::time::sleep(Duration::from_secs(3)).await;
            phase(&root, &topology, 4, "mirror_idle");
            tokio::time::sleep(Duration::from_secs(20)).await;
            phase(&root, &topology, 4, "mirror_interacting");
            let until = std::time::Instant::now() + Duration::from_secs(20);
            let mut sequences = vec![0; mirrors.len()];
            while std::time::Instant::now() < until {
                for (n, (id,generation)) in mirrors.iter().enumerate() {
                    let b = views[n].browser.as_ref().unwrap();
                    browser_request(&router, KernelBrowserCommand::Input {
                        tab_id:b.tab_id.clone(), generation:b.generation,
                        input:KernelBrowserInput::Click {x:80,y:125},
                    }).await;
                    browser_request(&router, KernelBrowserCommand::Input {
                        tab_id:b.tab_id.clone(), generation:b.generation,
                        input:KernelBrowserInput::Text {text:"mirror fixture".into()},
                    }).await;
                    let frame = browser_request(&router, KernelBrowserCommand::MirrorNext {
                        subscription_id:id.clone(), generation:*generation,
                        after_sequence:sequences[n], drift_nodes:vec![],
                    }).await;
                    sequences[n] = frame["sequence"].as_u64().unwrap_or(sequences[n]);
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            for (id,generation) in mirrors {
                browser_request(&router, KernelBrowserCommand::MirrorClose {subscription_id:id,generation}).await;
            }
        }
        assert_eq!(views.iter().map(|v| &v.origin).collect::<std::collections::BTreeSet<_>>().len(), if native {4} else {0});
        if native { assert!(sessions.list_sessions().is_empty()); }
    }).catch_unwind().await;
    phase(&root, &topology, 0, "cleanup");
    router.runtime_state.shutdown_cleanup().await.unwrap();
    if let Some(id) = &slice {
        dispatch(&router, serde_json::json!({"DeleteSlice":{"slice_ref":id}})).await;
    }
    for worker in workers {
        worker.shutdown_blocking();
    }
    assert!(observations.iter().all(|o| o.was_reaped()));
    phase(&root, &topology, 0, "finished");
    if let Err(error) = outcome {
        std::panic::resume_unwind(error);
    }
}
