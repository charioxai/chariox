//! Opt-in native pressure drill. Run only in an isolated Linux storage/kernel
//! service with public signed fixtures and an enrolled runtime. No provider runs.
use super::*;
use crate::durable_state::{
    app_installation_staging::AppVerifiedInstallationMutation,
    app_publishers::AppPublisherMutation,
    apps::{AppRegistryMutation, AppRegistryOutcome},
    DurableKernelStateStore,
};
use crate::runtime::{app_control::AppControlService, app_operation_budget::AppOperationBudget};
use base64::{engine::general_purpose::STANDARD, Engine};
use chariox_app_package::{verify, TrustedPublisher, VerificationPolicy};
use chariox_app_runtime::{
    app_catalog::{Actor, CallerContext},
    installation::{CapabilityApproval, CapabilityDecision, VerifiedInstallCandidate},
    publisher_trust::TrustDecision,
    release_store::{ReleaseStore, StageBudget},
};
use serde_json::{json, Value};

async fn call(
    store: DurableKernelStateStore,
    control: AppControlService,
    id: String,
    tool: &'static str,
    input: Value,
) -> Value {
    let lease = control.active_app_lease("alice", &id).unwrap();
    let name = lease
        .catalog()
        .app_catalog()
        .tools()
        .find(|spec| spec.local_name == tool)
        .unwrap()
        .name
        .clone();
    let slot = lease.reserve_call(Duration::from_secs(30)).unwrap();
    let admission = store.clone();
    let response = tokio::task::spawn_blocking(move || {
        admission.enqueue_app_tool(
            slot,
            &name,
            input,
            CallerContext {
                actor: Actor::Human("alice".into()),
                room_id: Some("wake-pressure".into()),
                operation_id: format!("pressure-{}", rand::random::<u64>()),
                task_id: None,
                turn_id: None,
            },
            AppOperationBudget::from_supervisor(|| false),
        )
    })
    .await
    .unwrap()
    .unwrap();
    let reply = response.receive().await.unwrap();
    tokio::task::spawn_blocking(move || store.accept_app_tool_reply(reply))
        .await
        .unwrap()
        .unwrap()
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires isolated native App storage/delegated cgroup and a quiet host"]
async fn native_wake_pressure_four_starts_four_calls() {
    let functional_only = std::env::var_os("CHARIOX_WAKE_FUNCTIONAL_ONLY").is_some();
    let startup_budget = Duration::from_secs(if functional_only { 90 } else { 30 });
    let fixtures = PathBuf::from(std::env::var("CHARIOX_WAKE_FIXTURES").unwrap());
    let root = PathBuf::from(std::env::var("CHARIOX_WAKE_DRILL_ROOT").unwrap());
    let output = PathBuf::from(std::env::var("CHARIOX_WAKE_DRILL_OUTPUT").unwrap());
    let mcp = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let mut config = daemon_config_for_runtime_mcp_listener(&mcp);
    config.user_config.state.path = Some(root.join("state.db").display().to_string());
    let app = Arc::new(Mutex::new(DaemonApp::bootstrap(config).unwrap()));
    let store = app.lock().await.durable_state_store();
    let router = Arc::new(CommandRouter::with_interactive_capacity(app.clone(), 4));
    let control = router.runtime_state().app_control().clone();
    let public: Value =
        serde_json::from_slice(&std::fs::read(fixtures.join("publisher-public.json")).unwrap())
            .unwrap();
    let key: [u8; 32] = STANDARD
        .decode(public["publicKey"].as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap();
    let publisher = TrustedPublisher {
        publisher_id: public["publisher"]["id"].as_str().unwrap().into(),
        key_id: public["publisher"]["keyId"].as_str().unwrap().into(),
        public_key: ed25519_dalek::VerifyingKey::from_bytes(&key).unwrap(),
    };
    let install = store.clone();
    let ids = tokio::task::spawn_blocking(move || {
        install
            .mutate_app_publisher(
                "alice",
                AppPublisherMutation::Enroll {
                    publisher: publisher.clone(),
                    expected_revision: 0,
                    now_ms: 1,
                    decision: TrustDecision {
                        decision_id: "pressure-enroll".into(),
                        authority_ref: "isolated-drill".into(),
                    },
                },
            )
            .unwrap();
        let trust = install
            .trusted_app_publisher("alice", &publisher.publisher_id, &publisher.key_id)
            .unwrap();
        let mut ids = Vec::new();
        for n in 0..4 {
            let bytes = std::fs::read(fixtures.join(format!("fixture-{n}.cxapp"))).unwrap();
            let package = verify(
                &bytes,
                &VerificationPolicy::new(
                    crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
                    vec![publisher.clone()],
                ),
            )
            .unwrap();
            let id = format!("wake_pressure_{n}");
            let AppRegistryOutcome::Update(record) = install
                .mutate_verified_app_installation(
                    "alice",
                    AppVerifiedInstallationMutation::CreateAndStage {
                        installation_id: id.clone(),
                        candidate: VerifiedInstallCandidate::from_verified(&package, &trust)
                            .unwrap(),
                        now_ms: 1,
                    },
                )
                .unwrap()
            else {
                panic!("stage")
            };
            ReleaseStore::open_or_create(install.path())
                .unwrap()
                .stage(
                    &package,
                    &bytes,
                    StageBudget {
                        max_stage_bytes: 32 * 1024 * 1024,
                        reserved_bytes: 32 * 1024 * 1024,
                        host_reserve_bytes: 8 * 1024 * 1024 * 1024,
                    },
                )
                .unwrap();
            for op in [
                AppRegistryMutation::Decide {
                    token: record.token.clone(),
                    decision: CapabilityDecision::Approved {
                        approval: CapabilityApproval {
                            decision_id: "pressure-approve".into(),
                            authority_ref: "isolated-drill".into(),
                        },
                    },
                    now_ms: 2,
                },
                AppRegistryMutation::Quiesce {
                    token: record.token.clone(),
                    now_ms: 3,
                },
                AppRegistryMutation::MarkPrepared {
                    token: record.token.clone(),
                    now_ms: 4,
                },
            ] {
                install.mutate_app_installation("alice", op).unwrap();
            }
            install
                .mutate_verified_app_installation(
                    "alice",
                    AppVerifiedInstallationMutation::Commit {
                        token: record.token,
                        now_ms: 5,
                    },
                )
                .unwrap();
            ids.push(id);
        }
        ids
    })
    .await
    .unwrap();
    // Native workers register over MCP before they can become ready.
    let (stop, stopped) = oneshot::channel();
    let server_router = router.clone();
    let server = tokio::spawn(run_kernel_websocket_server_with_bound_listeners(
        server_router,
        TcpListener::from_std({
            listener.set_nonblocking(true).unwrap();
            listener
        })
        .unwrap(),
        TcpListener::from_std({
            mcp.set_nonblocking(true).unwrap();
            mcp
        })
        .unwrap(),
        KernelLocalAuth::Unconfigured,
        async {
            let _ = stopped.await;
        },
    ));
    let mut starts = Vec::new();
    for id in &ids {
        let lifecycle = control.lifecycle().clone();
        let id = id.clone();
        let handle = tokio::runtime::Handle::current();
        starts.push(tokio::task::spawn_blocking(move || {
            let requested = crate::session::unix_epoch_ms();
            let deadline = std::time::Instant::now() + startup_budget;
            let mut busy = 0;
            loop {
                match lifecycle.start_active_blocking("alice", &id, handle.clone()) {
                    Ok(_) => return json!({"installation":id,"requestedAtMs":requested,"acceptedAtMs":crate::session::unix_epoch_ms(),"busyRetries":busy}),
                    Err(crate::runtime::app_lifecycle::LifecycleError::Busy) if std::time::Instant::now() < deadline => {
                        busy += 1;
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    Err(error) => panic!("native start failed: {error}"),
                }
            }
        }));
    }
    let mut start_results = Vec::new();
    for start in starts {
        start_results.push(start.await.unwrap());
    }
    timeout(startup_budget, async {
        while ids
            .iter()
            .any(|id| control.active_app_lease("alice", id).is_none())
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let mut growth = Vec::new();
    for id in &ids {
        growth.push(tokio::spawn(call(
            store.clone(),
            control.clone(),
            id.clone(),
            "grow",
            json!({"mib":420}),
        )));
    }
    for grow in growth {
        grow.await.unwrap();
    }
    let mut calls = Vec::new();
    for id in &ids {
        calls.push(tokio::spawn(call(
            store.clone(),
            control.clone(),
            id.clone(),
            "hold",
            json!({"ms":5000}),
        )));
    }
    let mut call_results = Vec::new();
    for call in calls {
        call_results.push(call.await.unwrap());
    }
    let latest_begin = call_results
        .iter()
        .map(|c| c["began"].as_u64().unwrap())
        .max()
        .unwrap();
    let earliest_end = call_results
        .iter()
        .map(|c| c["ended"].as_u64().unwrap())
        .min()
        .unwrap();
    assert!(latest_begin < earliest_end, "all four calls must overlap");
    assert_eq!(control.active_app_leases(None, 16).len(), 4);
    let mut samples = Vec::new();
    for n in 0..20 {
        let due = crate::session::unix_epoch_ms() + 1200;
        let wake_id = format!("pressure-{n}");
        for id in &ids[1..] {
            call(
                store.clone(),
                control.clone(),
                id.clone(),
                "schedule",
                json!({"id":wake_id,"dueAtMs":due}),
            )
            .await;
        }
        let backlog = tokio::spawn(call(
            store.clone(),
            control.clone(),
            ids[0].clone(),
            "hold",
            json!({"ms":2000}),
        ));
        tokio::time::sleep(Duration::from_millis(2000)).await;
        for id in &ids[1..] {
            let usage = call(
                store.clone(),
                control.clone(),
                id.clone(),
                "usage",
                json!({}),
            )
            .await;
            let matches = usage["wakes"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|w| w["id"] == wake_id)
                .count();
            assert!(matches <= 1, "a healthy wake must not be delivered twice");
            let wake = usage["wakes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|w| w["id"] == wake_id)
                .cloned();
            samples.push(json!({"installation":id,"dueAtMs":due,"wake":wake,"latenessMs":wake.as_ref().map(|w|w["at"].as_u64().unwrap().saturating_sub(due))}));
        }
        backlog.await.unwrap();
    }
    stop.send(()).unwrap();
    timeout(Duration::from_secs(30), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let mut lateness: Vec<u64> = samples
        .iter()
        .filter_map(|s| s["latenessMs"].as_u64())
        .collect();
    lateness.sort_unstable();
    let p95 = lateness
        .get((lateness.len() * 95).div_ceil(100).saturating_sub(1))
        .copied();
    std::fs::write(output, serde_json::to_vec_pretty(&json!({"timingEligible":!functional_only,"starts":start_results,"liveCap":4,"pressureMiB":420,"overlapMs":earliest_end-latest_begin,"calls":call_results,"n":lateness.len(),"p95Ms":p95,"maxMs":lateness.last(),"samples":samples})).unwrap()).unwrap();
    assert_eq!(lateness.len(), 60, "every wake must be delivered");
    if !functional_only {
        assert!(p95.unwrap() <= 1000, "unchanged K-06 lateness budget");
    }
}
