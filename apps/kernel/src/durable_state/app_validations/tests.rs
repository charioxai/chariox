use super::*;

struct Fixture(std::path::PathBuf, DurableKernelStateStore);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "chariox-validations-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir(&root).unwrap();
        let store = DurableKernelStateStore::open_owned(root.join("kernel.sqlite")).unwrap();
        Self(root, store)
    }
    fn create(&self, id: &str, amount: u64, expires_ms: u64) -> ValidationOperation {
        let (parameters, digest) = canonical(&serde_json::json!({"to": "acct", "amount": amount}));
        self.1
            .app_validation(ValidationCommand::Create(ValidationOperation {
                operation_id: id.into(),
                owner: "alice".into(),
                installation: "pay".into(),
                generation: 3,
                action: "send_payment".into(),
                parameters,
                digest,
                state: ValidationState::Pending,
                expires_ms,
            }))
            .unwrap()
            .unwrap()
    }
    fn consume(
        &self,
        id: &str,
        generation: u64,
        action: &str,
        now_ms: u64,
    ) -> Result<Option<ValidationOperation>, &'static str> {
        self.1.app_validation(ValidationCommand::Consume {
            owner: "alice".into(),
            installation: "pay".into(),
            generation,
            action: action.into(),
            operation_id: id.into(),
            now_ms,
        })
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn canonical_parameters_ignore_key_order() {
    let a = canonical(&serde_json::json!({"b": 1, "a": {"y": 2, "x": [3, {"q": 1, "p": 0}]}}));
    let b = canonical(&serde_json::json!({"a": {"x": [3, {"p": 0, "q": 1}], "y": 2}, "b": 1}));
    assert_eq!(a, b);
    assert_ne!(a.1, canonical(&serde_json::json!({"b": 2, "a": {}})).1);
}

#[test]
fn an_approval_is_decided_once_and_consumed_once_for_its_exact_binding() {
    let f = Fixture::new();
    f.create("op-1", 10, 1_000_000);
    assert_eq!(
        f.consume("op-1", 3, "send_payment", 10),
        Err("VALIDATION_REQUIRED"),
        "pending is not approved"
    );
    let approved =
        f.1.app_validation(ValidationCommand::Decide {
            operation_id: "op-1".into(),
            approved: true,
            now_ms: 100,
        })
        .unwrap()
        .unwrap();
    assert_eq!(approved.state, ValidationState::Approved);
    // A later reply (another terminal, a replay) cannot change the decision.
    let again =
        f.1.app_validation(ValidationCommand::Decide {
            operation_id: "op-1".into(),
            approved: false,
            now_ms: 101,
        })
        .unwrap()
        .unwrap();
    assert_eq!(again.state, ValidationState::Approved);
    assert_eq!(
        f.consume("op-1", 4, "send_payment", 200),
        Err("VALIDATION_REQUIRED"),
        "other generation"
    );
    assert_eq!(
        f.consume("op-1", 3, "refund", 200),
        Err("VALIDATION_REQUIRED"),
        "other action"
    );
    assert_eq!(
        f.consume("op-1", 3, "send_payment", 200)
            .unwrap()
            .unwrap()
            .state,
        ValidationState::Consumed
    );
    assert_eq!(
        f.consume("op-1", 3, "send_payment", 201),
        Err("VALIDATION_REQUIRED"),
        "single use"
    );
}

#[test]
fn denial_and_expiry_prevent_use_and_status_is_owner_scoped() {
    let f = Fixture::new();
    f.create("denied", 1, 1_000_000);
    f.1.app_validation(ValidationCommand::Decide {
        operation_id: "denied".into(),
        approved: false,
        now_ms: 1,
    })
    .unwrap();
    assert_eq!(
        f.consume("denied", 3, "send_payment", 2),
        Err("VALIDATION_REQUIRED")
    );
    f.create("late", 2, 50);
    let late =
        f.1.app_validation(ValidationCommand::Decide {
            operation_id: "late".into(),
            approved: true,
            now_ms: 60,
        })
        .unwrap()
        .unwrap();
    assert_eq!(
        late.state,
        ValidationState::Pending,
        "an expired operation takes no decision"
    );
    f.1.app_validation(ValidationCommand::Expire { now_ms: 60 })
        .unwrap();
    assert_eq!(
        f.1.app_validation_status("alice", "pay", "late")
            .unwrap()
            .unwrap()
            .state,
        ValidationState::Expired
    );
    assert!(f
        .1
        .app_validation_status("bob", "pay", "late")
        .unwrap()
        .is_none());
    assert!(f
        .1
        .app_validation_status("alice", "other", "late")
        .unwrap()
        .is_none());
}

#[test]
fn open_operations_are_bounded_per_installation() {
    let f = Fixture::new();
    for index in 0..MAX_OPEN {
        f.create(&format!("op-{index}"), index as u64, 1_000_000);
    }
    let (parameters, digest) = canonical(&serde_json::json!({}));
    assert_eq!(
        f.1.app_validation(ValidationCommand::Create(ValidationOperation {
            operation_id: "one-more".into(),
            owner: "alice".into(),
            installation: "pay".into(),
            generation: 3,
            action: "send_payment".into(),
            parameters,
            digest,
            state: ValidationState::Pending,
            expires_ms: 1_000_000,
        })),
        Err("LIMIT_EXCEEDED")
    );
    assert_eq!(
        f.1.pending_app_validations(100).unwrap().len(),
        MAX_OPEN as usize
    );
}
