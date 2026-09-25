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
        // The active generation the operations below belong to.
        rusqlite::Connection::open(store.path())
            .unwrap()
            .execute_batch(
                "INSERT INTO app_installations(installation_id,app_id,owner_id,generation,allocated_generation,active_json)
                 VALUES('pay','com.example.pay','alice',3,3,'{}')",
            )
            .unwrap();
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
    // The pump sees one operation per installation, so another App's newer
    // request is never hidden behind this backlog.
    let (parameters, digest) = canonical(&serde_json::json!({}));
    f.1.app_validation(ValidationCommand::Create(ValidationOperation {
        operation_id: "other-app".into(),
        owner: "bob".into(),
        installation: "notes".into(),
        generation: 1,
        action: "publish".into(),
        parameters,
        digest,
        state: ValidationState::Pending,
        expires_ms: 1_000_000,
    }))
    .unwrap();
    let pending: Vec<_> =
        f.1.pending_app_validations(100)
            .unwrap()
            .into_iter()
            .map(|operation| operation.operation_id)
            .collect();
    assert_eq!(pending, ["op-0", "other-app"]);
}

#[test]
fn finished_operations_are_removed_after_retention() {
    let f = Fixture::new();
    f.create("denied", 1, 1_000_000);
    f.create("open", 2, u64::MAX / 4);
    f.1.app_validation(ValidationCommand::Decide {
        operation_id: "denied".into(),
        approved: false,
        now_ms: 1,
    })
    .unwrap();
    f.1.app_validation(ValidationCommand::Expire { now_ms: 2 })
        .unwrap();
    assert!(f
        .1
        .app_validation_status("alice", "pay", "denied")
        .unwrap()
        .is_some());
    f.1.app_validation(ValidationCommand::Expire {
        now_ms: 2 + FINISHED_RETENTION_MS,
    })
    .unwrap();
    assert!(f
        .1
        .app_validation_status("alice", "pay", "denied")
        .unwrap()
        .is_none());
    assert!(f
        .1
        .app_validation_status("alice", "pay", "open")
        .unwrap()
        .is_some());
}

#[test]
fn an_updated_or_uninstalled_apps_open_operations_expire() {
    let f = Fixture::new();
    f.create("open", 1, u64::MAX / 4);
    f.1.app_validation(ValidationCommand::Expire { now_ms: 2 })
        .unwrap();
    let state = || {
        f.1.app_validation_status("alice", "pay", "open")
            .unwrap()
            .unwrap()
            .state
    };
    assert_eq!(state(), ValidationState::Pending);
    rusqlite::Connection::open(f.1.path())
        .unwrap()
        .execute_batch("UPDATE app_installations SET generation=4,allocated_generation=4")
        .unwrap();
    f.1.app_validation(ValidationCommand::Expire { now_ms: 3 })
        .unwrap();
    assert_eq!(state(), ValidationState::Expired);
}
