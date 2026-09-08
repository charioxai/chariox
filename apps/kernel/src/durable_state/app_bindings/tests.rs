use super::*;
use crate::durable_state::{
    app_publishers::AppPublisherMutation,
    apps::{AppRegistryMutation, AppRegistryOutcome},
};
use chariox_app_runtime::{
    installation::{CapabilityApproval, CapabilityDecision},
    publisher_trust::TrustDecision,
};
use std::path::PathBuf;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "chariox-app-binding-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn open(&self) -> DurableKernelStateStore {
        DurableKernelStateStore::open_owned(self.0.join("kernel.sqlite")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn decision(id: &str) -> TrustDecision {
    TrustDecision {
        decision_id: id.into(),
        authority_ref: "kernel-fixture".into(),
    }
}

#[test]
fn binding_requires_owned_verified_active_installation_without_a_worker() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let _catalog = crate::durable_state::app_state::fixture_catalog(&store);
    let active = store.check_app_binding("alice", "installed").unwrap();
    assert_eq!(active.generation, 1);
    assert!(store.check_app_binding("bob", "installed").is_err());
    assert!(store.check_app_binding("alice", "missing").is_err());

    // Legacy metadata-only activation is not package verification evidence.
    let AppRegistryOutcome::Update(record) = store
        .mutate_app_installation(
            "alice",
            AppRegistryMutation::CreateAndStage {
                installation_id: "unverified".into(),
                release: active.release.clone(),
                now_ms: 10,
            },
        )
        .unwrap()
    else {
        panic!("expected stage")
    };
    for mutation in [
        AppRegistryMutation::Decide {
            token: record.token.clone(),
            decision: CapabilityDecision::Approved {
                approval: CapabilityApproval {
                    decision_id: "raw-fixture".into(),
                    authority_ref: "fixture".into(),
                },
            },
            now_ms: 11,
        },
        AppRegistryMutation::Quiesce {
            token: record.token.clone(),
            now_ms: 12,
        },
        AppRegistryMutation::MarkPrepared {
            token: record.token.clone(),
            now_ms: 13,
        },
        AppRegistryMutation::Commit {
            token: record.token,
            now_ms: 14,
        },
    ] {
        store.mutate_app_installation("alice", mutation).unwrap();
    }
    assert!(store.check_app_binding("alice", "unverified").is_err());
    assert_eq!(
        store.check_app_binding("alice", "installed").unwrap(),
        active
    );
}

#[test]
fn unchanged_generation_does_not_survive_publisher_revoke_or_reenrollment() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let _catalog = crate::durable_state::app_state::fixture_catalog(&store);
    let trust = store
        .trusted_app_publisher("alice", "com.example", "state-key")
        .unwrap();
    store
        .mutate_app_publisher(
            "alice",
            AppPublisherMutation::Revoke {
                publisher_id: "com.example".into(),
                key_id: "state-key".into(),
                expected_revision: 1,
                decision: decision("revoke"),
                now_ms: 20,
            },
        )
        .unwrap();
    assert!(store.check_app_binding("alice", "installed").is_err());
    store
        .mutate_app_publisher(
            "alice",
            AppPublisherMutation::Enroll {
                publisher: trust.publisher().clone(),
                expected_revision: 2,
                decision: decision("reenroll"),
                now_ms: 21,
            },
        )
        .unwrap();
    assert_eq!(
        store
            .get_app_installation("alice", "installed")
            .unwrap()
            .generation,
        1
    );
    assert!(store.check_app_binding("alice", "installed").is_err());
}

#[test]
fn removed_installations_and_malformed_identity_do_not_poison_the_writer() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let _catalog = crate::durable_state::app_state::fixture_catalog(&store);
    for id in ["", "has space", "line\nfeed"] {
        assert!(store.check_app_binding("alice", id).is_err());
    }
    assert!(store.check_app_binding("alice", "installed").is_ok());
    store
        .mutate_app_installation(
            "alice",
            AppRegistryMutation::Uninstall {
                installation_id: "installed".into(),
                expected_generation: 1,
                now_ms: 30,
            },
        )
        .unwrap();
    assert!(store.check_app_binding("alice", "installed").is_err());
    assert!(store.list_app_installations("alice", None, 10).is_ok());
}
