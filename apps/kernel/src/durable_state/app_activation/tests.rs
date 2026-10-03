use super::*;
use crate::durable_state::app_publishers::AppPublisherMutation;
use chariox_app_runtime::publisher_trust::TrustDecision;

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "chariox-app-activation-{:016x}",
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
fn budget() -> AppOperationBudget {
    AppOperationBudget::from_supervisor(|| false)
}

#[test]
fn activation_confirmation_checks_the_actual_writer_and_current_owner() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let catalog = super::super::app_state::fixture_event_catalog(&store);
    // The writer is independent of the query connection; this guard must not
    // prevent the activation confirmation from reaching its transaction.
    let reader = store.connection.lock().unwrap();
    let proof = store
        .confirm_app_activation("alice", catalog.clone(), budget())
        .unwrap();
    assert_eq!(proof.owner(), "alice");
    assert_eq!(proof.installation_id(), catalog.installation_id());
    assert_eq!(proof.generation(), catalog.generation());
    assert_eq!(
        proof.package_digest(),
        catalog.app_catalog().package_digest()
    );
    assert_eq!(
        proof.catalog_digest(),
        catalog.app_catalog().catalog_digest()
    );
    assert!(store
        .confirm_app_activation("bob", catalog, budget())
        .is_err());
    drop(reader);
}

#[test]
fn activation_confirmation_does_not_bypass_cancellation_or_publisher_revocation() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let catalog = super::super::app_state::fixture_event_catalog(&store);
    assert!(matches!(
        store.confirm_app_activation(
            "alice",
            catalog.clone(),
            AppOperationBudget::from_supervisor(|| true)
        ),
        Err(AppActivationError::Stopped(_))
    ));
    store
        .mutate_app_publisher(
            "alice",
            AppPublisherMutation::Revoke {
                publisher_id: "com.example".into(),
                key_id: "state-key".into(),
                expected_revision: 1,
                decision: TrustDecision {
                    decision_id: "revoke-activation".into(),
                    authority_ref: "kernel-test".into(),
                },
                now_ms: 2,
            },
        )
        .unwrap();
    assert!(store
        .confirm_app_activation("alice", catalog, budget())
        .is_err());
}
