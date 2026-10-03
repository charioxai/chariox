//! Throwaway kernel drill through public requests. No live kernel, provider,
//! user package or credentials. The controller fixture only answers health;
//! rejected opens must never reach it.
use super::*;
use chariox_app_package::{verify, VerificationPolicy};
use chariox_app_runtime::release_store::{ReleaseStore, StageBudget};
use std::os::unix::fs::PermissionsExt;

struct Scratch(std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn request(
    router: &CommandRouter,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    router
        .dispatch(remote_command_for_request(&request, Some("alice")), request)
        .await
}

fn open(session_id: &str) -> LocalDaemonRequest {
    LocalDaemonRequest::OpenAppView(crate::local::OpenAppViewRequest {
        session_id: session_id.into(),
        installation_id: "installed".into(),
    })
}

#[test]
fn app_open_errors_in_a_throwaway_kernel_explain_listed_state_and_room_ownership() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(app_open_drill());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn app_open_drill() {
    let root =
        std::env::temp_dir().join(format!("chariox-app-open-{:016x}", rand::random::<u64>()));
    std::fs::create_dir(&root).unwrap();
    let scratch = Scratch(std::fs::canonicalize(root).unwrap());
    let mut config = DaemonConfig::for_tests().with_session_history_root(scratch.0.join("history"));
    config.local_socket_path = scratch.0.join("kernel.sock");
    config.user_config_path = scratch.0.join("config.toml");
    config.user_config.state.path = Some(scratch.0.join("state.db").display().to_string());
    config.user_config.history.operational.path =
        Some(scratch.0.join("history.db").display().to_string());
    config.user_config.artifacts.operational.root =
        Some(scratch.0.join("artifacts").display().to_string());
    config.user_config.artifacts.operational.index_path =
        Some(scratch.0.join("artifacts.db").display().to_string());
    let app = DaemonApp::bootstrap(config).unwrap();
    let store = app.durable_state_store();
    crate::durable_state::app_state::fixture_event_catalog(&store);
    let rooms: Vec<String> = (0..2)
        .map(|_| {
            app.sessions_mut()
                .create_session(
                    CreateSessionRequest::new(
                        scratch.0.to_string_lossy(),
                        scratch.0.to_string_lossy(),
                    )
                    .with_owner_user_id("alice"),
                )
                .unwrap()
                .id()
                .into()
        })
        .collect();
    let mut router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 2);
    let listed = request(
        &router,
        LocalDaemonRequest::ListAppInstallations(crate::local::ListAppInstallationsRequest {
            after: None,
            limit: None,
        }),
    )
    .await
    .unwrap();
    let LocalDaemonResponse::AppInstallationsListed { installations, .. } = listed else {
        panic!("list");
    };
    assert_eq!(installations[0].installation_id, "installed");
    assert!(installations[0].active_release.is_some());
    let unavailable = request(&router, open(&rooms[0]))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        unavailable.contains("installed, but its stored package is unavailable"),
        "{unavailable}"
    );
    assert!(unavailable.contains("/app update"));
    assert!(!unavailable.contains("not found"));

    let (bytes, publisher) = crate::durable_state::app_state::fixture_event_package();
    let package = verify(
        &bytes,
        &VerificationPolicy::new(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, vec![publisher]),
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
    let no_browser = request(&router, open(&rooms[1]))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        no_browser.contains("no browser controller available"),
        "{no_browser}"
    );
    assert!(no_browser.contains("/room bind"));
    assert!(!no_browser.contains("stale revision"));

    let controller = scratch.0.join("controller.sh");
    std::fs::write(&controller, r#"#!/bin/sh
while IFS= read -r request; do
  id=${request#*:}
  id=${id%%,*}
  case "$request" in
    *'"method":"health"'*) printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s,"diagnostic_code":null}}\n' "$id" "$$" ;;
    *'"method":"shutdown"'*) printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null,"diagnostic_code":null}}\n' "$id"; exit 0 ;;
    *) exit 1 ;;
  esac
done
"#).unwrap();
    std::fs::set_permissions(&controller, std::fs::Permissions::from_mode(0o700)).unwrap();
    let processes = crate::runtime::browser_controller_process::BrowserControllerProcessStore::new(
        controller,
        vec![],
        std::time::Duration::from_secs(3),
    );
    processes.acquire(&rooms[0]).unwrap();
    router
        .runtime_state
        .set_browser_controller_process_store_for_test(processes.clone());
    let wrong_room = request(&router, open(&rooms[1]))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        wrong_room.contains(&format!(
            "belongs to Room {}, not Room {}",
            rooms[0], rooms[1]
        )),
        "{wrong_room}"
    );
    assert!(wrong_room.contains("/room view"));
    assert!(!wrong_room.contains("stale revision"));
    // A failed open neither stops nor reassigns the admitted Room's browser.
    assert!(processes.acquire(&rooms[0]).is_ok());
    processes.shutdown().unwrap();
    let stopped = request(&router, open(&rooms[0]))
        .await
        .unwrap_err()
        .to_string();
    assert!(stopped.contains("not started"), "{stopped}");
    assert!(stopped.contains("/room start"));

    let installation = store.get_app_installation("alice", "installed").unwrap();
    store
        .mutate_app_installation(
            "alice",
            crate::durable_state::apps::AppRegistryMutation::Uninstall {
                installation_id: "installed".into(),
                expected_generation: installation.generation,
                now_ms: 99,
            },
        )
        .unwrap();
    let listed = request(
        &router,
        LocalDaemonRequest::ListAppInstallations(crate::local::ListAppInstallationsRequest {
            after: None,
            limit: None,
        }),
    )
    .await
    .unwrap();
    let LocalDaemonResponse::AppInstallationsListed { installations, .. } = listed else {
        panic!("list");
    };
    assert!(installations[0].data_kept);
    assert!(installations[0].active_release.is_none());
    let inactive = request(&router, open(&rooms[0]))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        inactive.contains("uninstalled; its data is kept"),
        "{inactive}"
    );
    assert!(inactive.contains("/app update"));
    assert!(!inactive.contains("not found"));
    let record = store.get_app_installation("alice", "installed").unwrap();
    store
        .mutate_app_installation(
            "alice",
            crate::durable_state::apps::AppRegistryMutation::ForgetData {
                installation_id: "installed".into(),
                expected_generation: record.generation,
            },
        )
        .unwrap();
    let no_data = request(&router, open(&rooms[0]))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        no_data.contains("no active release or retained data"),
        "{no_data}"
    );
    assert!(no_data.contains("/app install"));
}
