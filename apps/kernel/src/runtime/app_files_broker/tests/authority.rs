use super::*;
use crate::durable_state::{app_publishers::AppPublisherMutation, apps::AppRegistryMutation};
use chariox_app_runtime::publisher_trust::TrustDecision;

#[test]
fn retained_private_directory_does_not_bypass_generation_or_signer_revocation() {
    for revoke_signer in [false, true] {
        let fixture = Fixture::new(Mode::Ready);
        fixture.runtime.block_on(async {
            let mut peer = TestPeer::start(fixture.service());
            peer.replace("before", b"preserve").await;
            assert_eq!(
                peer.response().await.1.unwrap(),
                serde_json::json!({"bytesWritten":8})
            );
            if revoke_signer {
                fixture
                    .store
                    .mutate_app_publisher(
                        "alice",
                        AppPublisherMutation::Revoke {
                            publisher_id: "com.example".into(),
                            key_id: "state-key".into(),
                            expected_revision: 1,
                            decision: TrustDecision {
                                decision_id: "file-revoke".into(),
                                authority_ref: "kernel-fixture".into(),
                            },
                            now_ms: 20,
                        },
                    )
                    .unwrap();
                assert_eq!(
                    fixture
                        .store
                        .get_app_installation("alice", "installed")
                        .unwrap()
                        .generation,
                    1
                );
            } else {
                fixture
                    .store
                    .mutate_app_installation(
                        "alice",
                        AppRegistryMutation::Uninstall {
                            installation_id: "installed".into(),
                            expected_generation: 1,
                            now_ms: 20,
                        },
                    )
                    .unwrap();
                assert!(
                    fixture
                        .store
                        .get_app_installation("alice", "installed")
                        .unwrap()
                        .generation
                        > 1
                );
            }
            // The same retained descriptor and catalog still exist. Only the
            // authoritative writer check can reject this already staged write.
            peer.replace("after", b"must not publish").await;
            let error = peer.response().await.1.unwrap_err();
            assert_eq!(error.code, "APP_STALE");
            assert_eq!(error.retryable, Some(false));
            peer.close().await;
        });
        assert_eq!(
            fixture.observed.private_file().unwrap(),
            Some(b"preserve".to_vec())
        );
        assert_eq!(fixture.observed.pending_file_replacements().unwrap(), 0);
    }
}

#[test]
fn backend_broker_rejects_another_real_worker_installation_before_channel_use() {
    use crate::{
        durable_state::app_state::fixture_event_package,
        runtime::{app_backend_broker, app_worker::AppWorkerError},
    };
    use chariox_app_package::{verify, VerificationPolicy};
    use chariox_app_runtime::worker_process::test_fixture::Fixture as NativeFixture;
    let fixture = Fixture::new(Mode::Ready);
    let native = NativeFixture::compile().unwrap();
    let (bytes, publisher) = fixture_event_package();
    let package = verify(
        &bytes,
        &VerificationPolicy::new(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, vec![publisher]),
    )
    .unwrap();
    let (process, observed) = native
        .spawn_blocking(Mode::OtherInstallation, &package)
        .unwrap();
    let result = app_backend_broker::broker(
        fixture.store.clone(),
        "alice".into(),
        fixture.catalog.clone(),
        fixture.admission.clone(),
        &process,
    );
    assert!(matches!(result, Err(AppWorkerError::Identity)));
    drop(process);
    assert!(observed.was_reaped());
    assert!(observed.lease_was_dropped());
    assert_eq!(observed.private_file().unwrap(), None);
}
