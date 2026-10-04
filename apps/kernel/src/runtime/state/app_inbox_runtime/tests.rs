//! Regression for accepted work held through a native crash-loop quarantine.
use super::*;
use crate::{
    durable_state::{
        app_state::{fixture_inbox_installation, AppStateOperation},
        app_wakes::{AppWakeOperation, AppWakeOutcome},
    },
    runtime::{app_operation_budget::AppOperationBudget, router::CommandRouter},
    DaemonApp, DaemonConfig,
};
use chariox_app_package::{verify, VerificationPolicy};
use chariox_app_runtime::{
    app_inbox::MAX_ATTEMPTS,
    managed_state::{Wake, WakeChange},
    release_store::{ReleaseStore, StageBudget},
};
use serde_json::json;
use std::{sync::Arc, time::Duration};

struct Scratch(std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        fn writable(path: &std::path::Path) {
            use std::os::unix::fs::PermissionsExt;
            if std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir()) {
                let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
                if let Ok(children) = std::fs::read_dir(path) {
                    for child in children.flatten() {
                        writable(&child.path());
                    }
                }
            }
        }
        // Only this disposable test kernel's sealed release roots.
        writable(&self.0);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn wait(mut ready: impl FnMut() -> bool) {
    let end = tokio::time::Instant::now() + Duration::from_secs(30);
    while !ready() {
        assert!(
            tokio::time::Instant::now() < end,
            "native lifecycle timed out"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn counts(state: &KernelRuntimeState) -> InboxCounts {
    let AppInboxOutcome::Routes(routes) = state
        .owned
        .durable_state_store
        .app_inbox(AppInboxOperation::Routes {
            owner: "alice".into(),
            installation: "installed".into(),
        })
        .unwrap()
    else {
        panic!("inbox routes")
    };
    routes.into_iter().next().unwrap().1
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn quarantined_inbox_and_wake_keep_attempts_and_deliver_once_after_explicit_start() {
    let config = DaemonConfig::for_tests();
    let _scratch = Scratch(config.durable_state_path().parent().unwrap().to_owned());
    let app = DaemonApp::bootstrap(config).unwrap();
    let store = app.durable_state_store();
    let control = app.app_control_service();
    let observations = control.lifecycle().fixture_inbox_workers();
    let (bytes, publisher) = fixture_inbox_installation(&store, "alice");
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
    let router =
        CommandRouter::with_interactive_capacity(Arc::new(tokio::sync::Mutex::new(app)), 4);
    let state = router.runtime_state();
    store
        .app_inbox(AppInboxOperation::CreateRoute {
            route: InboxRoute {
                route_id: "crash".into(),
                owner_id: "alice".into(),
                installation_id: "installed".into(),
                event_name: "received".into(),
                source_event_type: "fixture.crash".into(),
                source_event_version: 1,
                active: true,
                source: None,
            },
            now_ms: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    control
        .lifecycle()
        .start_active_blocking("alice", "installed", tokio::runtime::Handle::current())
        .unwrap();
    // Four acknowledged native crashes; restarts retain the real 1/4/16 s backoff.
    for crash in 1..=4 {
        wait(|| control.active_app_lease("alice", "installed").is_some()).await;
        state
            .accept_app_inbox_occurrence(
                "alice",
                "installed",
                "crash",
                &format!("crash-{crash}"),
                json!({"text":"crash"}),
            )
            .await
            .unwrap();
        wait(|| counts(&state).delivered == crash).await;
        wait(|| {
            store
                .app_worker_status("alice", "installed")
                .unwrap()
                .is_some_and(|s| {
                    s.failures == crash as u32
                        && s.phase
                            == crate::durable_state::app_worker_lifecycle::WorkerPhase::Failed
                })
        })
        .await;
        if crash < 4 {
            wait(|| {
                let _ = control.lifecycle().start_on_demand_blocking(
                    "alice",
                    "installed",
                    tokio::runtime::Handle::current(),
                );
                control.active_app_lease("alice", "installed").is_some()
            })
            .await;
        }
    }
    assert!(store
        .app_worker_status("alice", "installed")
        .unwrap()
        .unwrap()
        .is_quarantined());
    // Both maintenance and a due planner remove the stale idle-start catalog.
    control.lifecycle().fixture_dormant_catalog();
    assert!(control.is_app_dormant("alice", "installed"));
    state.prune_dormant_apps().await;
    assert!(!control.is_app_dormant("alice", "installed"));
    control.lifecycle().fixture_dormant_catalog();
    let catalog = store
        .active_app_event_catalog("alice", "installed")
        .unwrap();
    store
        .execute_app_state(
            "alice",
            catalog,
            AppStateOperation::Schedule {
                wakes: vec![WakeChange::Set(Wake {
                    id: "held-wake".into(),
                    due_at_ms: crate::session::unix_epoch_ms(),
                    revision: "r1".into(),
                })],
                wakes_count_as_use: false,
            },
            AppOperationBudget::from_supervisor(|| false),
        )
        .unwrap();
    assert!(!state
        .accept_app_inbox_occurrence(
            "alice",
            "installed",
            "crash",
            "control",
            json!({"text":"kept"})
        )
        .await
        .unwrap());
    // Advance the production delivery pass beyond every retry deadline and the
    // former eight-attempt budget, without waiting minutes in the test suite.
    let now = crate::session::unix_epoch_ms();
    for pass in 1..=MAX_ATTEMPTS + 2 {
        let at = now + u64::from(pass) * 60_000;
        state.app_inbox_pass(at).await;
        let AppWakeOutcome::Due(due) = store
            .app_wakes(AppWakeOperation::Due {
                now_ms: at,
                limit: 8,
            })
            .unwrap()
        else {
            panic!("held wake")
        };
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].attempts, 0);
        let (deliver, held, _) = state
            .plan_app_delivery(due, at, |w| (w.owner_id.clone(), w.installation_id.clone()))
            .await
            .unwrap();
        assert!(deliver.is_empty());
        for (wake, settle) in held {
            assert_eq!(
                settle,
                Settle::Postponed(at + 60_000),
                "quarantine uses the long stopped-work wait"
            );
            store
                .app_wakes(AppWakeOperation::Postponed {
                    wake,
                    until_ms: at + 60_000,
                })
                .unwrap();
        }
        assert_eq!(
            observations.lock().unwrap().len(),
            4,
            "quarantine must not restart the worker"
        );
    }
    let held = counts(&state);
    assert!(!control.is_app_dormant("alice", "installed"));
    assert_eq!((held.pending, held.failed, held.delivered), (1, 0, 4));
    let AppInboxOutcome::Due(due) = store
        .app_inbox(AppInboxOperation::Due {
            now_ms: now + u64::from(MAX_ATTEMPTS + 3) * 60_000,
            limit: 8,
        })
        .unwrap()
    else {
        panic!("held inbox")
    };
    assert_eq!(due.len(), 1);
    assert_eq!(
        due[0].attempts, 0,
        "quarantine must not consume delivery attempts"
    );
    control
        .lifecycle()
        .start_active_blocking("alice", "installed", tokio::runtime::Handle::current())
        .unwrap();
    wait(|| control.active_app_lease("alice", "installed").is_some()).await;
    let recovered_at = now + 11 * 60_000;
    let AppWakeOutcome::Due(due) = store
        .app_wakes(AppWakeOperation::Due {
            now_ms: recovered_at,
            limit: 8,
        })
        .unwrap()
    else {
        panic!("recovered wake")
    };
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].attempts, 0);
    control
        .active_app_lease("alice", "installed")
        .unwrap()
        .deliver_wake(&due[0].wake, true, false, Duration::from_secs(5))
        .await
        .unwrap();
    store
        .app_wakes(AppWakeOperation::Delivered(due[0].clone()))
        .unwrap();
    assert_eq!(
        store
            .app_wakes(AppWakeOperation::Due {
                now_ms: recovered_at + 60_000,
                limit: 8
            })
            .unwrap(),
        AppWakeOutcome::Due(Vec::new())
    );
    state.app_inbox_pass(recovered_at).await;
    assert_eq!(
        (
            counts(&state).pending,
            counts(&state).failed,
            counts(&state).delivered
        ),
        (0, 0, 5)
    );
    assert!(state
        .accept_app_inbox_occurrence(
            "alice",
            "installed",
            "crash",
            "control",
            json!({"text":"kept"})
        )
        .await
        .unwrap());
    state.app_inbox_pass(recovered_at + 60_000).await;
    let frames: Vec<_> = observations
        .lock()
        .unwrap()
        .iter()
        .flat_map(|o| o.lifecycle_frames().unwrap())
        .collect();
    assert_eq!(frames.iter().filter(|f| f["method"] == "events.deliver" && f["params"]["occurrence_id"] == "control").count(), 1);
    assert_eq!(
        frames
            .iter()
            .filter(|f| f["method"] == "schedule.wake" && f["params"]["id"] == "held-wake")
            .count(),
        1
    );
    assert_eq!(counts(&state).delivered, 5);
    assert_eq!(
        store
            .app_worker_status("alice", "installed")
            .unwrap()
            .unwrap()
            .failures,
        0
    );
    control.lifecycle().shutdown_blocking().unwrap();
    assert!(observations
        .lock()
        .unwrap()
        .iter()
        .all(|o| o.was_reaped() && o.lease_was_dropped()));
}
