//! MP-08/MP-10/MP-11: signed installed catalog -> A2 -> native Wasm worker.
use super::*;

#[path = "../../../../../../../../packages/app-wasm-worker/tests/support/package.rs"]
mod installed_package;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires source App and adjacent native worker; use installed-ui-actions-drill.sh"]
async fn signed_installed_ui_actions_reach_the_real_view_context() {
    use crate::durable_state::{
        app_installation_staging::AppVerifiedInstallationMutation,
        app_publishers::AppPublisherMutation,
        apps::{AppRegistryMutation, AppRegistryOutcome},
    };
    use chariox_app_package::{verify_app, VerificationPolicy};
    use chariox_app_runtime::{
        installation::{CapabilityApproval, CapabilityDecision, VerifiedInstallCandidate},
        publisher_trust::TrustDecision,
        release_store::{ReleaseStore, StageBudget},
    };

    let mut rig = rig_with_installation(false);
    let store = rig.runtime.owned.durable_state_store.clone();
    let core = std::fs::read(
        std::env::var_os("CHARIOX_WORKER_DRILL_CORE").expect("build the source App first"),
    )
    .unwrap();
    let (bytes, publisher) = installed_package::package(&core);
    let package = verify_app(
        &bytes,
        &VerificationPolicy::new(
            crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
            vec![publisher.clone()],
        ),
    )
    .unwrap();
    store
        .mutate_app_publisher(
            "alice",
            AppPublisherMutation::Enroll {
                publisher: publisher.clone(),
                expected_revision: 0,
                decision: TrustDecision {
                    decision_id: "enroll-source".into(),
                    authority_ref: "owner".into(),
                },
                now_ms: 1,
            },
        )
        .unwrap();
    let trust = store
        .trusted_app_publisher("alice", &publisher.publisher_id, &publisher.key_id)
        .unwrap();
    let releases = ReleaseStore::open_or_create(store.path()).unwrap();
    let reserved = releases.required_reservation(&package, &bytes).unwrap();
    releases
        .stage(
            &package,
            &bytes,
            StageBudget {
                max_stage_bytes: 512 << 20,
                reserved_bytes: reserved,
                host_reserve_bytes: 4 << 30,
            },
        )
        .unwrap();
    let AppRegistryOutcome::Update(record) = store
        .mutate_verified_app_installation(
            "alice",
            AppVerifiedInstallationMutation::CreateAndStage {
                installation_id: APP.into(),
                candidate: VerifiedInstallCandidate::from_verified(&package, &trust).unwrap(),
                now_ms: 2,
            },
        )
        .unwrap()
    else {
        panic!("expected a staged signed installation")
    };
    let token = record.token;
    for mutation in [
        AppRegistryMutation::Decide {
            token: token.clone(),
            decision: CapabilityDecision::Approved {
                approval: CapabilityApproval {
                    decision_id: "approve-source".into(),
                    authority_ref: "owner".into(),
                },
            },
            now_ms: 3,
        },
        AppRegistryMutation::Quiesce {
            token: token.clone(),
            now_ms: 4,
        },
        AppRegistryMutation::MarkPrepared {
            token: token.clone(),
            now_ms: 5,
        },
    ] {
        store.mutate_app_installation("alice", mutation).unwrap();
    }
    store
        .mutate_verified_app_installation(
            "alice",
            AppVerifiedInstallationMutation::Commit {
                token: token.clone(),
                now_ms: 6,
            },
        )
        .unwrap();
    // Read the stored, reverified release rather than the package kept above.
    let catalog = store.active_app_event_catalog("alice", APP).unwrap();
    assert_eq!(catalog.app_catalog().ui_actions().len(), 1);
    assert_eq!(catalog.app_catalog().ui_actions()[0].name, "inspect");
    rig.grant().await;
    rig.bind(&rig.session, token.generation);
    // A registry commit alone records no intent to run. Use the real lifecycle
    // start, as installation does, before A2 waits for the native worker.
    rig.runtime
        .app_control()
        .lifecycle()
        .start_active_blocking("alice", APP, tokio::runtime::Handle::current())
        .unwrap();
    // No hand-filled ViewContext or worker fixture: A2 reads the installation's
    // verified catalog from its actual running Wasm worker.
    let context = rig.runtime.view_context(&rig.agent, TARGET).await.unwrap();
    assert_eq!(context.declared, catalog.app_catalog().ui_actions());
    assert!(rig
        .runtime
        .app_control()
        .active_app_lease("alice", APP)
        .is_some());
    let visible = rig
        .runtime
        .app_view_tool(&rig.run, &rig.agent, CATALOG, json!({}))
        .await
        .unwrap();
    assert_eq!(visible["views"][0]["actions"][0]["name"], "inspect");
    assert_eq!(visible["views"][0]["actions"].as_array().unwrap().len(), 1);
    rig.runtime
        .app_control()
        .lifecycle()
        .stop_blocking("alice", APP)
        .unwrap();
}
