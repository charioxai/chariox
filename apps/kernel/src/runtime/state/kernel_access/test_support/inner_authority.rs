use super::worker_spy::WorkerSpy;
use super::*;
use crate::runtime::state::browser_controller_action_execution_runtime_state::computer_input_reconcile_test_support::{TestRoom, TestTools, install_screen_tool};
use crate::transport::room_browser_controller::{RoomBrowserControllerCommand as Command, RoomComputerInputAction as Input};
use std::sync::{Arc, atomic::Ordering};
use tokio::sync::{Mutex, Notify};
use tokio::time::timeout;

macro_rules! local_regression {
    ($name:ident, $kind:expr) => {
        #[test]
        fn $name() {
            std::thread::Builder::new()
                .stack_size(64 * 1024 * 1024)
                .spawn(|| {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .max_blocking_threads(1)
                        .build()
                        .unwrap()
                        .block_on(revoked_input_helper($kind));
                })
                .unwrap()
                .join()
                .unwrap();
        }
    };
}
local_regression!(
    kernel_access_inner_keyboard_helper_rechecks_blocking_queue,
    "keyboard"
);
local_regression!(
    kernel_access_inner_pointer_helper_rechecks_blocking_queue,
    "pointer"
);
local_regression!(
    kernel_access_inner_clipboard_helper_rechecks_blocking_queue,
    "clipboard"
);

async fn revoked_input_helper(kind: &str) {
    let worktree = crate::test_support::TestWorktree::new("access-input-helper");
    let tools = TestTools::new("access-input-helper");
    let marker = worktree.path().join("input-effect");
    std::fs::write(
        &tools.screen_tool,
        format!(
            "#!/bin/sh\nset -eu\ncase \"$1\" in computer-type-stdin|computer-clipboard-write-stdin) cat >/dev/null;; esac\nprintf effect > '{}'\n",
            marker.display()
        ),
    )
    .unwrap();
    let _environment = install_screen_tool(&tools.screen_tool);
    let room = TestRoom::new("access-input-helper");
    let state = &room.runtime;
    let grant = state.insert_access_grant_for_test(&room.session_id);
    let request =
        LocalDaemonRequest::StopRoomEnvironment(crate::local::StopRoomEnvironmentRequest {
            session_id: room.session_id.clone(),
        });
    let authorized = state.with_external_command_authority(Some((&grant, &request)));
    let command = Command::ComputerInput {
        action_id: "input-action".into(),
        actor_id: crate::session::agent_environment_actor_id(&room.agent_id),
        runtime_generation: 1,
        viewport_revision: 1,
        desktop_pixel_width: 1280,
        desktop_pixel_height: 800,
        action: match kind {
            "keyboard" => Input::KeyboardText {
                input: crate::transport::room_browser_controller::RoomComputerKeyboardInput::new(
                    "fixture".into(),
                ),
            },
            "pointer" => Input::PointerMove { x: 20, y: 20 },
            "clipboard" => Input::ClipboardWrite {
                text: crate::transport::room_browser_controller::RoomComputerClipboardText::new(
                    "fixture".into(),
                ),
            },
            _ => unreachable!(),
        },
    };
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let blocker = tokio::task::spawn_blocking(move || {
        started.send(()).unwrap();
        blocked.recv().unwrap();
    });
    ready.await.unwrap();
    let pending = authorized.room_browser_controller_command(&room.session_id, command.clone());
    tokio::pin!(pending);
    tokio::select! { biased; result = &mut pending => panic!("input did not queue: {result:?}"), _ = tokio::task::yield_now() => {} }
    state
        .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
        .unwrap();
    release.send(()).unwrap();
    blocker.await.unwrap();
    let result = timeout(Duration::from_secs(3), pending).await.unwrap();
    assert!(
        !marker.exists(),
        "revoked actual input helper created its effect marker"
    );
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("grant revoked or expired"));
    state
        .room_browser_controller_command(&room.session_id, command)
        .await
        .unwrap();
    assert!(marker.exists(), "trusted helper did not run");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kernel_access_inner_native_recovery_rechecks_app_wait() {
    revoked_native_recovery(false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kernel_access_inner_native_recovery_rechecks_discovery_wait() {
    revoked_native_recovery(true).await;
}

async fn revoked_native_recovery(discovery: bool) {
    let worktree = crate::test_support::TestWorktree::new("access-stale-native");
    let worker = WorkerSpy::new(discovery);
    worker.native_recovery.store(true, Ordering::SeqCst);
    let mut config = crate::config::DaemonConfig::for_tests();
    config.relay_url = Some(worker.url.clone());
    config.relay_token = Some("stale-native-fixture".into());
    let mut app = crate::test_support::bootstrap_authenticated_app(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(
            worktree
                .session_request()
                .with_agent_defaults(crate::session::SessionAgentDefaults::new("dev-stub")),
        )
        .unwrap();
    app.agents_mut()
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: worker.id.clone(),
                worker_machine_id: "fixture-machine".into(),
                execution_lease_id: "stale-lease".into(),
                leased_agent_id: "stale-agent".into(),
                active_worker_provider_run_id: Some("stale-run".into()),
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let router =
        crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(app.clone(), 32);
    let mut state = router.runtime_state();
    let probe = Arc::new(Notify::new());
    state.observe_app_lock_wait_for_test(probe.clone());
    let grant = state.insert_access_grant_for_test(session.id());
    let before = state.owned.agent_store.get_agent(agent.id()).unwrap();
    let request = crate::local::LaunchProviderRunRequest {
        session_id: session.id().into(),
        agent_id: Some(agent.id().into()),
        adapter_key: "dev-stub".into(),
        provider: "dev-stub".into(),
        account_profile: "default".into(),
        model: "default".into(),
        variant: None,
        structured_endpoint: None,
        provider_session_id: None,
        native_tui: true,
    };
    let pending = tokio::spawn({
        let state = state.clone();
        let request = request.clone();
        let grant = grant.clone();
        async move {
            let launch = state.launch_remote_native_provider_run_with_grant(
                &request,
                crate::session::DEFAULT_LOCAL_USER_ID,
                Some(&grant),
            );
            // MP-08 / MP-10 / MP-11: the stale-binding callbacks must fit the default task stack.
            assert!(std::mem::size_of_val(&launch) <= 64 * 1024);
            launch.await
        }
    });
    timeout(
        Duration::from_secs(3),
        worker.native_launch_started.notified(),
    )
    .await
    .unwrap();
    let guard = if discovery {
        None
    } else {
        Some(app.lock().await)
    };
    worker.release_native_launch.notify_one();
    if discovery {
        timeout(Duration::from_secs(3), worker.discovery_started.notified())
            .await
            .unwrap();
    } else {
        timeout(Duration::from_secs(3), probe.notified())
            .await
            .unwrap();
    }
    state
        .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
        .unwrap();
    drop(guard);
    worker.release_discovery.notify_one();
    let result = timeout(Duration::from_secs(5), pending)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        worker.requests.load(Ordering::SeqCst),
        1,
        "revoked recovery created worker lease/spawn effects"
    );
    assert_eq!(
        state.owned.agent_store.get_agent(agent.id()).unwrap(),
        before,
        "revoked recovery changed home binding"
    );
    let error = result.unwrap_err().to_string();
    assert!(
        error.contains("grant revoked or expired"),
        "native recovery result: {error}"
    );
    state
        .launch_remote_native_provider_run(&request, crate::session::DEFAULT_LOCAL_USER_ID)
        .await
        .unwrap();
    assert_eq!(
        worker.requests.load(Ordering::SeqCst),
        5,
        "trusted native recovery did not create lease, spawn and retry launch"
    );
}

#[test]
fn kernel_access_inner_room_stop_rechecks_supervisor_wait() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .max_blocking_threads(2)
                .build()
                .unwrap()
                .block_on(revoked_supervisor_stop());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn revoked_supervisor_stop() {
    let worktree = crate::test_support::TestWorktree::new("access-supervisor");
    let tools = TestTools::new("access-supervisor");
    let started = worktree.path().join("backend-wait");
    let release = worktree.path().join("release-backend");
    let script = format!(
        r#"const fs=require('node:fs');
require('node:readline').createInterface({{input:process.stdin}}).on('line',line=>{{
 const {{id,method}}=JSON.parse(line);let result;
 if(method==='health')result={{state:'ready',process_id:process.pid,diagnostic_code:null}};
 else if(method==='shutdown')result={{state:'stopped',process_id:null,diagnostic_code:null}};
 else {{fs.writeFileSync({started},'waiting');while(!fs.existsSync({release}))Atomics.wait(new Int32Array(new SharedArrayBuffer(4)),0,0,10);result={{}};}}
 process.stdout.write(JSON.stringify({{id,ok:true,result}})+'\n',()=>{{if(method==='shutdown')process.exit(0);}});
}});
"#,
        started = serde_json::to_string(&started).unwrap(),
        release = serde_json::to_string(&release).unwrap()
    );
    std::fs::write(&tools.controller_tool, script).unwrap();
    let mut room = TestRoom::new("access-supervisor");
    room.enable_browser_controller(&tools).await;
    let mut processes = room.runtime.owned.browser_controller_processes.clone();
    let before = processes.snapshot().unwrap().unwrap();
    let backend = tokio::task::spawn_blocking({
        let processes = processes.clone();
        let session = room.session_id.clone();
        move || {
            processes.app_view(
                &session,
                &crate::runtime::browser_controller_app_view::BrowserAppViewRequest::Layout {
                    target_id: "fixture".into(),
                    page: crate::runtime::browser_controller_app_view::AppViewPage {
                        width: 800,
                        height: 600,
                    },
                },
            )
        }
    });
    timeout(Duration::from_secs(3), async {
        while !started.exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let probe = Arc::new(Notify::new());
    processes.observe_supervisor_lock_wait_for_test(probe.clone());
    room.runtime
        .set_browser_controller_process_store_for_test(processes.clone());
    let grant = room.runtime.insert_access_grant_for_test(&room.session_id);
    let request =
        LocalDaemonRequest::StopRoomEnvironment(crate::local::StopRoomEnvironmentRequest {
            session_id: room.session_id.clone(),
        });
    let authorized = room
        .runtime
        .with_external_command_authority(Some((&grant, &request)));
    let pending = authorized.stop_managed_room_environment_runtime(&room.session_id);
    tokio::pin!(pending);
    tokio::select! { biased; result=&mut pending=>panic!("stop did not wait: {result:?}"), signal=timeout(Duration::from_secs(3),probe.notified())=>{signal.unwrap();} }
    room.runtime
        .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
        .unwrap();
    std::fs::write(&release, "release").unwrap();
    backend.await.unwrap().unwrap();
    let result = timeout(Duration::from_secs(3), pending).await.unwrap();
    let after = processes.snapshot().unwrap().unwrap();
    // Stop our fixture controller before any assertion can fail.
    room.runtime
        .stop_browser_controller_process(&room.session_id)
        .await
        .unwrap();
    assert_eq!(
        after, before,
        "revoked queued stop terminated the controller"
    );
    assert_eq!(
        room.runtime
            .room_environment_snapshot(&room.session_id)
            .unwrap()
            .lifecycle,
        crate::session::EnvironmentLifecycle::Failed,
        "denied release must settle the started Stop as Failed"
    );
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("grant revoked or expired"));
    assert_eq!(
        processes.snapshot().unwrap().unwrap().state,
        crate::runtime::browser_controller_process::BrowserControllerProcessState::Stopped,
        "trusted stop did not work"
    );
}
