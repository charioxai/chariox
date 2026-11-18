//! A view larger than one relay reply opens, whichever host shows it: the reply never carries the
//! view's files, so a client far away on the relay gets its answer however big the view is.
use super::*;
use crate::{local::*, transport::relay_client::MAX_REPLY_BYTES};
use chariox_app_package::{verify, VerificationPolicy};
use chariox_app_runtime::release_store::{ReleaseStore, StageBudget};
use std::os::unix::fs::PermissionsExt;

const VIEW_BYTES: usize = 3 * 1024 * 1024 / 2;

struct Scratch(std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn request(router: &CommandRouter, request: LocalDaemonRequest) -> LocalDaemonResponse {
    let mut command = remote_command_for_request(&request, Some("alice"));
    command.caller.connection_class = Some(KernelConnectionClass::Terminal);
    router.dispatch(command, request).await.unwrap()
}

fn open(host: Option<UserAppViewHost>) -> LocalDaemonRequest {
    LocalDaemonRequest::OpenUserAppView(OpenUserAppViewRequest {
        installation_id: "installed".into(),
        host,
    })
}

/// What the relay would carry back for this answer.
fn relayed_bytes(response: &LocalDaemonResponse) -> usize {
    serde_json::to_vec(response).unwrap().len()
}

#[test]
fn a_view_over_one_relay_reply_opens_on_every_host_and_the_kernel_browser_still_gets_its_files() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap()
                .block_on(large_view_drill());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn large_view_drill() {
    let root =
        std::env::temp_dir().join(format!("chariox-large-view-{:016x}", rand::random::<u64>()));
    std::fs::create_dir(&root).unwrap();
    let scratch = Scratch(std::fs::canonicalize(root).unwrap());
    let mut config = DaemonConfig::for_tests().with_session_history_root(scratch.0.join("history"));
    let socket = crate::test_support::TestSocketDirectory::new();
    config.local_socket_path = socket.path().join("kernel.sock");
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
    let (bytes, publisher) =
        crate::durable_state::app_state::fixture_large_view_package(VIEW_BYTES);
    crate::durable_state::app_state::fixture_event_catalog_from_package(
        &store,
        (bytes.clone(), publisher.clone()),
    );
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
                max_stage_bytes: 8 * 1024 * 1024,
                reserved_bytes: 8 * 1024 * 1024,
                host_reserve_bytes: 8 * 1024 * 1024,
            },
        )
        .unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 2);

    // The kernel browser host: a controller that records what it is asked to open.
    let seen = scratch.0.join("controller-open.json");
    let controller = scratch.0.join("controller.sh");
    // awk, not a shell `read`: a shell reads a pipe one byte at a time, too slowly for a request that holds the whole view.
    std::fs::write(
        &controller,
        format!(
            r#"#!/bin/sh
exec awk '
BEGIN {{ "echo $PPID" | getline pid }}
{{
  id = $0; sub(/^[^:]*:/, "", id); sub(/,.*/, "", id)
  if (index($0, "\"method\":\"health\"")) {{
    printf "{{\"id\":%s,\"ok\":true,\"result\":{{\"state\":\"ready\",\"process_id\":%s,\"diagnostic_code\":null}}}}\n", id, pid
  }} else if (index($0, "\"method\":\"browser.app.open\"")) {{
    print $0 > "{}"; close("{}")
    label = $0; sub(/.*\"origin_label\":\"/, "", label); sub(/\".*/, "", label)
    printf "{{\"id\":%s,\"ok\":true,\"result\":{{\"target_id\":\"large-target\",\"origin\":\"https://app.%s.invalid\",\"tab_id\":\"host-tab-large\",\"generation\":1}}}}\n", id, label
  }} else if (index($0, "\"method\":\"browser.app.calls\"")) {{
    printf "{{\"id\":%s,\"ok\":true,\"result\":{{\"calls\":[]}}}}\n", id
  }} else {{
    printf "{{\"id\":%s,\"ok\":true,\"result\":{{}}}}\n", id
  }}
  fflush()
}}'
"#,
            seen.display(), seen.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&controller, std::fs::Permissions::from_mode(0o700)).unwrap();
    let host = router.runtime_state.kernel_browser_host_for_test();
    host.install_fixture_backend("alice", &controller, &scratch.0);
    host.protected_request(
        "alice",
        None,
        "host.browser",
        serde_json::json!({"op":"start"}),
        serde_json::json!({"values":[],"targets":[],"unknown":false}),
    )
    .unwrap();

    for host in [
        None,
        Some(UserAppViewHost::ClientNative),
        Some(UserAppViewHost::KernelBrowser),
    ] {
        let opened = request(&router, open(host)).await;
        let size = relayed_bytes(&opened);
        assert!(
            size < MAX_REPLY_BYTES,
            "{host:?}: the open reply is {size} bytes, over one relay reply"
        );
        let LocalDaemonResponse::UserAppViewOpened { view, frontend } = opened.clone() else {
            panic!("{host:?}: open answered {opened:?}");
        };
        assert_eq!(frontend.entry, "index.html");
        assert!(
            frontend.assets.is_empty(),
            "{host:?}: the reply holds no view file"
        );
        let fetched = request(
            &router,
            LocalDaemonRequest::GetUserAppViewFrontend(UserAppViewRequest {
                view_id: view.view_id.clone(),
            }),
        )
        .await;
        let size = relayed_bytes(&fetched);
        assert!(
            size < MAX_REPLY_BYTES,
            "{host:?}: the frontend reply is {size} bytes"
        );
        request(
            &router,
            LocalDaemonRequest::CloseUserAppView(UserAppViewRequest {
                view_id: view.view_id,
            }),
        )
        .await;
    }

    // The files went where they are shown: to the kernel browser, in the kernel.
    let asked = std::fs::read_to_string(&seen).unwrap();
    assert!(
        asked.len() > VIEW_BYTES,
        "the kernel browser was handed the view's files"
    );
    assert!(asked.contains("data-2.bin"));
}
