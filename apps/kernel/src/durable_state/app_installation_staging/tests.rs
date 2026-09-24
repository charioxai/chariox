use super::*;
use crate::durable_state::{
    app_publishers::AppPublisherMutation,
    apps::{AppRegistryError, AppRegistryMutation},
};
use chariox_app_package::{pack, verify, Limits, Manifest, TrustedPublisher, VerificationPolicy};
use chariox_app_runtime::{
    installation::{CapabilityApproval, CapabilityDecision},
    publisher_trust::{PublisherTrustError, TrustDecision},
};
use ed25519_dalek::SigningKey;
use serde_json::json;
use std::{collections::BTreeMap, path::PathBuf};

struct Database(PathBuf);
impl Database {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "chariox-verified-install-writer-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn open(&self) -> DurableKernelStateStore {
        DurableKernelStateStore::open_owned(self.0.join("kernel.db")).unwrap()
    }
}
impl Drop for Database {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn decision(id: &str) -> TrustDecision {
    TrustDecision {
        decision_id: id.into(),
        authority_ref: "kernel-decision-fixture".into(),
    }
}
fn publisher() -> TrustedPublisher {
    TrustedPublisher {
        publisher_id: "com.example".into(),
        key_id: "developer-1".into(),
        public_key: SigningKey::from_bytes(&[27; 32]).verifying_key(),
    }
}
fn enroll(store: &DurableKernelStateStore, owner: &str, revision: u64, id: &str) {
    store
        .mutate_app_publisher(
            owner,
            AppPublisherMutation::Enroll {
                publisher: publisher(),
                expected_revision: revision,
                decision: decision(id),
                now_ms: 1,
            },
        )
        .unwrap();
}
fn candidate(
    store: &DurableKernelStateStore,
    owner: &str,
    version: &str,
) -> VerifiedInstallCandidate {
    let manifest: Manifest = serde_json::from_value(json!({
        "schema":"chariox.app.v1", "appId":"com.example.installed", "version":version,
        "publisher":{"id":"com.example","keyId":"developer-1","name":"Developer"},
        "sdkVersion":"0.7.0", "appContractVersion":1, "minKernelProtocol":500,
        "resourcePolicy":"chariox.app.resources.v1", "runtime":{"engine":"node","entry":"runtime/main.js"},
        "ui":{"entry":"ui/index.html"}, "capabilities":{}
    })).unwrap();
    let files = BTreeMap::from([
        (
            "runtime/main.js".into(),
            b"export default function register() {}".to_vec(),
        ),
        (
            "ui/index.html".into(),
            b"<!doctype html><title>writer fixture</title>".to_vec(),
        ),
    ]);
    let bytes = pack(
        &manifest,
        &files,
        &SigningKey::from_bytes(&[27; 32]),
        &Limits::default(),
    )
    .unwrap();
    let trust = store
        .trusted_app_publisher(owner, "com.example", "developer-1")
        .unwrap();
    let policy = VerificationPolicy::new(500, vec![trust.publisher().clone()]);
    VerifiedInstallCandidate::from_verified(&verify(&bytes, &policy).unwrap(), &trust).unwrap()
}
fn token(outcome: AppRegistryOutcome) -> StageToken {
    match outcome {
        AppRegistryOutcome::Update(record) => record.token,
        other => panic!("expected update, got {other:?}"),
    }
}
fn prepare(store: &DurableKernelStateStore, token: &StageToken) {
    store
        .mutate_app_installation(
            "alice",
            AppRegistryMutation::Decide {
                token: token.clone(),
                decision: CapabilityDecision::Approved {
                    approval: CapabilityApproval {
                        decision_id: "capability-fixture".into(),
                        authority_ref: "kernel-decision-fixture".into(),
                    },
                },
                now_ms: 2,
            },
        )
        .unwrap();
    store
        .mutate_app_installation(
            "alice",
            AppRegistryMutation::Quiesce {
                token: token.clone(),
                now_ms: 3,
            },
        )
        .unwrap();
    store
        .mutate_app_installation(
            "alice",
            AppRegistryMutation::MarkPrepared {
                token: token.clone(),
                now_ms: 4,
            },
        )
        .unwrap();
}

fn assert_plain_commit_rejected(store: &DurableKernelStateStore, token: &StageToken) {
    assert!(matches!(
        store.mutate_app_installation(
            "alice",
            AppRegistryMutation::Commit {
                token: token.clone(),
                now_ms: 5,
            }
        ),
        Err(AppRegistryError::Registry(InstallationError::Invalid(
            "verified activation required"
        )))
    ));
}

#[test]
fn verified_writer_uses_its_connection_and_refuses_to_refresh_an_old_stages_trust() {
    let database = Database::new();
    let store = database.open();
    enroll(&store, "alice", 0, "enroll");
    let candidate_v1 = candidate(&store, "alice", "1.0.0");
    let reader = store.connection.lock().unwrap();
    let first = token(
        store
            .mutate_verified_app_installation(
                "alice",
                AppVerifiedInstallationMutation::CreateAndStage {
                    installation_id: "installation".into(),
                    candidate: candidate_v1,
                    now_ms: 1,
                },
            )
            .unwrap(),
    );
    prepare(&store, &first);
    assert_plain_commit_rejected(&store, &first);
    // Both verification-stage checks and the commit's fresh trust snapshot use
    // the writer; holding the separate read-only connection cannot deadlock it.
    let AppRegistryOutcome::Active(active) = store
        .mutate_verified_app_installation(
            "alice",
            AppVerifiedInstallationMutation::Commit {
                token: first.clone(),
                now_ms: 5,
            },
        )
        .unwrap()
    else {
        panic!("expected active generation")
    };
    drop(reader);
    let second = token(
        store
            .mutate_verified_app_installation(
                "alice",
                AppVerifiedInstallationMutation::Stage {
                    installation_id: "installation".into(),
                    expected_generation: 1,
                    candidate: candidate(&store, "alice", "2.0.0"),
                    now_ms: 10,
                },
            )
            .unwrap(),
    );
    prepare(&store, &second);
    store
        .mutate_app_publisher(
            "alice",
            AppPublisherMutation::Revoke {
                publisher_id: "com.example".into(),
                key_id: "developer-1".into(),
                expected_revision: 1,
                decision: decision("revoke"),
                now_ms: 20,
            },
        )
        .unwrap();
    assert!(matches!(
        store.mutate_verified_app_installation(
            "alice",
            AppVerifiedInstallationMutation::Commit {
                token: second.clone(),
                now_ms: 21
            }
        ),
        Err(AppVerifiedInstallationError::Verified(
            VerifiedStageError::Trust(PublisherTrustError::Revoked)
        ))
    ));
    assert_plain_commit_rejected(&store, &second);
    let preserved = store.get_app_installation("alice", "installation").unwrap();
    assert_eq!(preserved.active.as_ref(), Some(&active));
    assert_eq!(preserved.pending_generation, Some(second.generation));
    assert!(preserved.admission_paused);
    enroll(&store, "alice", 2, "reenroll");
    assert!(matches!(
        store.mutate_verified_app_installation(
            "alice",
            AppVerifiedInstallationMutation::Commit {
                token: second.clone(),
                now_ms: 22
            }
        ),
        Err(AppVerifiedInstallationError::Verified(
            VerifiedStageError::Trust(PublisherTrustError::Conflict)
        ))
    ));
    store
        .mutate_app_installation(
            "alice",
            AppRegistryMutation::Abort {
                token: second,
                reason: "trust revision changed".into(),
                now_ms: 23,
            },
        )
        .unwrap();
    let third = token(
        store
            .mutate_verified_app_installation(
                "alice",
                AppVerifiedInstallationMutation::Stage {
                    installation_id: "installation".into(),
                    expected_generation: 1,
                    candidate: candidate(&store, "alice", "2.0.0"),
                    now_ms: 24,
                },
            )
            .unwrap(),
    );
    prepare(&store, &third);
    let AppRegistryOutcome::Active(active) = store
        .mutate_verified_app_installation(
            "alice",
            AppVerifiedInstallationMutation::Commit {
                token: third,
                now_ms: 25,
            },
        )
        .unwrap()
    else {
        panic!("expected active generation")
    };
    assert_eq!(active.generation, 3);
    drop(store);
    let reopened = database.open();
    assert_eq!(
        reopened
            .get_app_installation("alice", "installation")
            .unwrap()
            .active,
        Some(active)
    );
}

#[test]
fn verified_commit_rejects_other_owner_and_plain_metadata_foundation_stages() {
    let database = Database::new();
    let store = database.open();
    enroll(&store, "alice", 0, "enroll");
    enroll(&store, "bob", 0, "enroll");
    let proof = candidate(&store, "alice", "1.0.0");
    let release = proof.release_metadata().clone();
    let verified = token(
        store
            .mutate_verified_app_installation(
                "alice",
                AppVerifiedInstallationMutation::CreateAndStage {
                    installation_id: "verified".into(),
                    candidate: proof,
                    now_ms: 1,
                },
            )
            .unwrap(),
    );
    prepare(&store, &verified);
    assert!(matches!(
        store.mutate_verified_app_installation(
            "bob",
            AppVerifiedInstallationMutation::Commit {
                token: verified,
                now_ms: 10
            }
        ),
        Err(AppVerifiedInstallationError::Verified(
            VerifiedStageError::Installation(InstallationError::NotFound)
        ))
    ));
    let plain = token(
        store
            .mutate_app_installation(
                "alice",
                AppRegistryMutation::CreateAndStage {
                    installation_id: "plain".into(),
                    release,
                    now_ms: 1,
                },
            )
            .unwrap(),
    );
    prepare(&store, &plain);
    assert!(matches!(
        store.mutate_verified_app_installation(
            "alice",
            AppVerifiedInstallationMutation::Commit {
                token: plain,
                now_ms: 10
            }
        ),
        Err(AppVerifiedInstallationError::Verified(
            VerifiedStageError::Installation(InstallationError::NotFound)
        ))
    ));
    assert!(store
        .get_app_installation("alice", "plain")
        .unwrap()
        .active
        .is_none());
}

#[test]
fn rejected_verified_operation_preserves_adjacent_ordinary_writer_batches() {
    let database = Database::new();
    let store = database.open();
    enroll(&store, "alice", 0, "enroll");
    let stage = token(
        store
            .mutate_verified_app_installation(
                "alice",
                AppVerifiedInstallationMutation::CreateAndStage {
                    installation_id: "installation".into(),
                    candidate: candidate(&store, "alice", "1.0.0"),
                    now_ms: 1,
                },
            )
            .unwrap(),
    );
    prepare(&store, &stage);
    store
        .mutate_app_publisher(
            "alice",
            AppPublisherMutation::Revoke {
                publisher_id: "com.example".into(),
                key_id: "developer-1".into(),
                expected_revision: 1,
                decision: decision("revoke"),
                now_ms: 20,
            },
        )
        .unwrap();
    let (before_tx, before_rx) = mpsc::channel();
    let (rejected_tx, rejected_rx) = mpsc::channel();
    let (after_tx, after_rx) = mpsc::channel();
    let event = |id: &str, response| {
        DurableWriterRequest::Ordinary(super::super::DurableWriteRequest {
            operation: super::super::DurableWriteOperation::Event {
                event_id: id.into(),
                kind: "app.verified.test".into(),
                subject_id: None,
                timestamp_ms: 1,
                payload_json: "{}".into(),
            },
            response,
        })
    };
    store.writer.enqueue(event("before", before_tx)).unwrap();
    store
        .writer
        .enqueue(DurableWriterRequest::VerifiedApp(Box::new(
            AppVerifiedInstallationRequest {
                owner_id: "alice".into(),
                mutation: AppVerifiedInstallationMutation::Commit {
                    token: stage,
                    now_ms: 21,
                },
                response: rejected_tx,
            },
        )))
        .unwrap();
    store.writer.enqueue(event("after", after_tx)).unwrap();
    assert!(before_rx.recv().unwrap().is_ok());
    assert!(matches!(
        rejected_rx.recv().unwrap(),
        Err(VerifiedStageError::Trust(PublisherTrustError::Revoked))
    ));
    assert!(after_rx.recv().unwrap().is_ok());
    assert_eq!(
        store
            .load_events_by_kind("app.verified.test")
            .unwrap()
            .len(),
        2
    );
}
