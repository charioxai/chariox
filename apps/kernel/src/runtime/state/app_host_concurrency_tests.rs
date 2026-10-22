//! MP-08/MP-10/MP-11 #992: host offers share the ordinary App writer/grant boundary.
use super::*;

/// MP-08/MP-10/MP-11 #992 P1: run the lock inversion in a subprocess so
/// a regression cannot strand this test runner's durable writer or Tokio pool.
#[test]
fn mp11_app_host_offer_does_not_lock_out_an_earlier_tool_or_revocation() {
    const CHILD: &str = "CHARIOX_APP_HOST_CONCURRENCY_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(host_offer_concurrency());
        return;
    }
    let root = std::env::temp_dir().join(format!(
        "chariox-host-concurrency-{:016x}",
        rand::random::<u64>()
    ));
    std::fs::create_dir(&root).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!(
                "{}::mp11_app_host_offer_does_not_lock_out_an_earlier_tool_or_revocation",
                module_path!().split_once("::").unwrap().1
            ),
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("TMPDIR", &root)
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(45);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if std::time::Instant::now() >= deadline {
            assert!(child.id() > 1, "refuse an unsafe process signal");
            child.kill().unwrap();
            break child.wait().unwrap();
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    std::fs::remove_dir_all(root).unwrap();
    assert!(
        status.success(),
        "host action blocked the shared writer: {status}"
    );
}

async fn host_offer_concurrency() {
    use crate::runtime::{app_host_broker::AppHostBroker, app_view_authority::AppViewAuthority};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let ViewFixture {
        state,
        agent,
        view,
        _cleanup,
        ..
    } = prepare(Mode::ToolEcho);
    state
        .app_control()
        .lifecycle()
        .start_active_blocking("alice", "installed", tokio::runtime::Handle::current())
        .unwrap();
    wait_worker(&state).await;
    let lease = state
        .app_control()
        .active_app_lease("alice", "installed")
        .unwrap();
    let name = lease
        .catalog()
        .app_catalog()
        .tools()
        .next()
        .unwrap()
        .name
        .clone();
    let mut tracked = state.app_control().views().track_call(&view, &view, None);
    let epoch = state
        .owned
        .kernel_browser_host
        .retain_resource_grant(
            "alice",
            agent.id(),
            UserDomainResource::AppView {
                view_id: view.clone(),
            },
        )
        .unwrap();
    tracked.retain_browser_grant(epoch.clone());
    let authority = AppViewAuthority::new(
        state.owned.agent_store.clone(),
        Some(&agent),
        "installed",
        Some(epoch),
        None,
    );
    let (entered, waiting) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    let released = Mutex::new(released);
    let checks = AtomicUsize::new(0);
    let budget = tracked
        .operation_budget()
        .fixture_observe_checks(Arc::new(move || {
            // enqueue_app_tool's initial check and the writer's three checks:
            // stop immediately before its browser-grant commit.
            if checks.fetch_add(1, Ordering::SeqCst) == 3 {
                entered.send(()).unwrap();
                released.lock().unwrap().recv().unwrap();
            }
        }));
    let store = state.owned.durable_state_store.clone();
    let slot = lease.reserve_call(Duration::from_secs(30)).unwrap();
    let caller = CallerContext {
        actor: Actor::Agent(agent.id().into()),
        room_id: None,
        operation_id: "host-concurrency-tool".into(),
        task_id: None,
        turn_id: None,
    };
    let tool = tokio::task::spawn_blocking(move || {
        store.enqueue_app_tool(
            slot,
            &name,
            serde_json::json!({"text":"ordinary tool"}),
            caller,
            budget,
        )
    });
    waiting.recv_timeout(Duration::from_secs(5)).unwrap();
    eprintln!("MP-08/MP-10/MP-11: ordinary tool held before writer-side grant commit");
    let broker = AppHostBroker::new(
        state.owned.durable_state_store.clone(),
        "alice".into(),
        lease.catalog().clone(),
        state.app_control().admission(),
    );
    let (offering, offered) = std::sync::mpsc::channel();
    let host_budget = tracked
        .operation_budget()
        .fixture_observe_checks(Arc::new(move || {
            let _ = offering.send(());
        }));
    let host = tokio::spawn(async move {
        broker.request_view("host.open_link", serde_json::json!({"url":"https://en.wikipedia.org/wiki/Rust_(programming_language)"}), authority, host_budget).await
    });
    offered.recv_timeout(Duration::from_secs(5)).unwrap();
    eprintln!("MP-08/MP-10/MP-11: granted host action admitted; releasing ordinary tool");
    release.send(()).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        let response = tool.await.unwrap().unwrap();
        let reply = response.receive().await.unwrap();
        state
            .owned
            .durable_state_store
            .accept_app_tool_reply(reply)
            .unwrap();
        assert_eq!(host.await.unwrap().unwrap()["state"], "pending");
        state
            .owned
            .kernel_browser_host
            .revoke_grants("alice", Some(agent.id()));
        assert!(tracked.operation_budget().check().is_err());

        // A second offer waits on a real external SQLite writer. Revoking and
        // regranting before release must neither block nor revive its old epoch.
        grant(&state, &agent, &view);
        let mut queued = state.app_control().views().track_call(&view, &view, None);
        let epoch = state
            .owned
            .kernel_browser_host
            .retain_resource_grant(
                "alice",
                agent.id(),
                UserDomainResource::AppView {
                    view_id: view.clone(),
                },
            )
            .unwrap();
        queued.retain_browser_grant(epoch.clone());
        let authority = AppViewAuthority::new(
            state.owned.agent_store.clone(),
            Some(&agent),
            "installed",
            Some(epoch),
            None,
        );
        let connection =
            rusqlite::Connection::open(state.owned.durable_state_store.path()).unwrap();
        connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        let (entered, waiting) = std::sync::mpsc::channel();
        let checks = AtomicUsize::new(0);
        let budget = queued
            .operation_budget()
            .fixture_observe_checks(Arc::new(move || {
                if checks.fetch_add(1, Ordering::SeqCst) == 1 {
                    entered.send(()).unwrap();
                }
            }));
        let broker = AppHostBroker::new(
            state.owned.durable_state_store.clone(),
            "alice".into(),
            lease.catalog().clone(),
            state.app_control().admission(),
        );
        let pending = tokio::spawn(async move {
            broker
                .request_view(
                    "host.open_link",
                    serde_json::json!({"url":"https://github.com/charioxai/chariox"}),
                    authority,
                    budget,
                )
                .await
        });
        waiting.recv_timeout(Duration::from_secs(3)).unwrap();
        state
            .owned
            .kernel_browser_host
            .revoke_grants("alice", Some(agent.id()));
        grant(&state, &agent, &view);
        connection.execute_batch("ROLLBACK").unwrap();
        assert_eq!(
            pending.await.unwrap().unwrap_err().code,
            "AGENT_APP_NOT_GRANTED"
        );
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM app_host_actions", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    })
    .await;
    if result.is_err() {
        eprintln!("MP-08/MP-10/MP-11: ordinary tool/host offer or revocation failed to complete");
    }
    cleanup_worker(&state).await;
    state.app_control().lifecycle().fixture_clear_workers();
    if result.is_err() {
        eprintln!("MP-08/MP-10/MP-11: ordinary tool and granted host action deadlocked on the durable writer");
        // The subprocess owns the stuck writer. Its worker is already reaped;
        // bypass destructors waiting on that writer, then let the parent clean scratch.
        std::process::exit(1);
    }
}
