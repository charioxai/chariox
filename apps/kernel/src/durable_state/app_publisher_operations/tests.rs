use super::*;
use chariox_app_runtime::publisher_trust::{PublisherTrustRegistry, TrustDecision};
use std::time::Duration;

fn database() -> Connection {
    let mut connection = Connection::open_in_memory().unwrap();
    PublisherTrustRegistry::new(&mut connection)
        .initialize()
        .unwrap();
    initialize(&connection).unwrap();
    connection
}
fn budget() -> AppOperationBudget {
    AppOperationBudget::from_supervisor(|| false)
}
fn input() -> PublisherEnrollmentInput {
    PublisherEnrollmentInput {
        session_id: "session".into(),
        publisher_id: "local.developer".into(),
        key_id: "development".into(),
        public_key: ed25519_dalek::SigningKey::from_bytes(&[19; 32])
            .verifying_key()
            .to_bytes(),
        expected_revision: 0,
    }
}
fn begin(
    connection: &mut Connection,
    owner: &str,
    request: &str,
    input: PublisherEnrollmentInput,
) -> Result<PublisherOperation> {
    match apply::execute(
        connection,
        Command::Begin {
            owner: owner.into(),
            request: request.into(),
            input,
            budget: budget(),
        },
    )? {
        Reply::Operation(value) => Ok(value),
        _ => panic!("unexpected reply"),
    }
}
fn arm(
    connection: &mut Connection,
    owner: &str,
    request: &str,
    id: &str,
) -> PublisherApprovalChallenge {
    match apply::execute(
        connection,
        Command::Arm {
            owner: owner.into(),
            request: request.into(),
            interaction: id.into(),
            deadline: Instant::now() + Duration::from_secs(60),
            budget: budget(),
        },
    )
    .unwrap()
    {
        Reply::Review(PublisherReview::Prompt(value)) => value,
        _ => panic!("expected fresh prompt"),
    }
}
fn decide(
    connection: &mut Connection,
    challenge: PublisherApprovalChallenge,
    accepted: bool,
) -> Result<PublisherOperation> {
    match apply::execute(
        connection,
        Command::Decide {
            challenge: Arc::new(challenge),
            accepted,
            budget: budget(),
        },
    )? {
        Reply::Operation(value) => Ok(value),
        _ => panic!("unexpected reply"),
    }
}
fn key_count(connection: &Connection) -> i64 {
    connection
        .query_row("SELECT count(*) FROM app_publisher_keys", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn immutable_input_replay_and_owner_scope() {
    let mut connection = database();
    let first = begin(&mut connection, "alice", "request", input()).unwrap();
    assert_eq!(
        first,
        begin(&mut connection, "alice", "request", input()).unwrap()
    );
    for changed in [
        PublisherEnrollmentInput {
            session_id: "other".into(),
            ..input()
        },
        PublisherEnrollmentInput {
            expected_revision: 1,
            ..input()
        },
        PublisherEnrollmentInput {
            key_id: "other".into(),
            ..input()
        },
    ] {
        assert_eq!(
            begin(&mut connection, "alice", "request", changed),
            Err(PublisherOperationError::Conflict)
        );
    }
    assert_eq!(
        store::load(&connection, "bob", "request"),
        Err(PublisherOperationError::NotFound)
    );
    begin(&mut connection, "bob", "request", input()).unwrap();
    assert_eq!(
        key_count(&connection),
        0,
        "a submitted public key is not consent"
    );
}

#[test]
fn stale_prompt_and_original_deadline_cannot_be_renewed() {
    let mut connection = database();
    begin(&mut connection, "alice", "request", input()).unwrap();
    let old = arm(&mut connection, "alice", "request", "old");
    let mut current = arm(&mut connection, "alice", "request", "new");
    assert_eq!(
        decide(&mut connection, old, true),
        Err(PublisherOperationError::Conflict)
    );
    current.deadline = Instant::now();
    assert_eq!(
        decide(&mut connection, current, true),
        Err(PublisherOperationError::Stopped)
    );
    assert_eq!(key_count(&connection), 0);
}

#[test]
fn approval_receipt_and_trust_are_atomic() {
    let mut connection = database();
    begin(&mut connection, "alice", "request", input()).unwrap();
    let challenge = arm(&mut connection, "alice", "request", "approval");
    connection.execute_batch("CREATE TRIGGER reject_operation BEFORE UPDATE OF phase ON app_publisher_operations WHEN NEW.phase='approved' BEGIN SELECT RAISE(ABORT,'fixture'); END").unwrap();
    assert_eq!(
        decide(&mut connection, challenge, true),
        Err(PublisherOperationError::Storage)
    );
    assert_eq!(key_count(&connection), 0);
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM app_publisher_decisions", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        store::load(&connection, "alice", "request").unwrap().phase,
        PublisherOperationPhase::Pending
    );
}

#[test]
fn historical_approval_replay_never_reenrolls_revoked_key() {
    let mut connection = database();
    begin(&mut connection, "alice", "request", input()).unwrap();
    let challenge = arm(&mut connection, "alice", "request", "approval");
    let approved = decide(&mut connection, challenge, true).unwrap();
    assert_eq!(approved.phase, PublisherOperationPhase::Approved);
    PublisherTrustRegistry::new(&mut connection)
        .revoke(
            "alice",
            "local.developer",
            "development",
            1,
            &TrustDecision {
                decision_id: "later-revocation".into(),
                authority_ref: "kernel".into(),
            },
            50,
        )
        .unwrap();
    assert_eq!(
        begin(&mut connection, "alice", "request", input()).unwrap(),
        approved
    );
    assert!(
        !PublisherTrustRegistry::new(&mut connection)
            .get("alice", "local.developer", "development")
            .unwrap()
            .enrolled
    );
    assert!(matches!(
        apply::execute(
            &mut connection,
            Command::Cancel {
                owner: "alice".into(),
                request: "request".into(),
                budget: budget()
            }
        ),
        Err(PublisherOperationError::Conflict)
    ));
}

#[test]
fn cancellation_and_denial_do_not_enroll_or_allow_late_approval() {
    let mut connection = database();
    begin(&mut connection, "alice", "cancel", input()).unwrap();
    let cancelled = arm(&mut connection, "alice", "cancel", "cancel-nonce");
    apply::execute(
        &mut connection,
        Command::Cancel {
            owner: "alice".into(),
            request: "cancel".into(),
            budget: budget(),
        },
    )
    .unwrap();
    assert_eq!(
        decide(&mut connection, cancelled, true),
        Err(PublisherOperationError::Conflict)
    );
    begin(&mut connection, "alice", "deny", input()).unwrap();
    let denied = arm(&mut connection, "alice", "deny", "deny-nonce");
    assert_eq!(
        decide(&mut connection, denied, false).unwrap().phase,
        PublisherOperationPhase::Denied
    );
    assert_eq!(key_count(&connection), 0);
}

#[test]
fn pending_capacity_is_owned_and_terminal_history_stays_replayable() {
    let mut connection = database();
    for n in 0..8 {
        begin(&mut connection, "alice", &format!("request-{n}"), input()).unwrap();
    }
    assert_eq!(
        begin(&mut connection, "alice", "ninth", input()),
        Err(PublisherOperationError::Limit)
    );
    begin(&mut connection, "bob", "first", input()).unwrap();
    apply::execute(
        &mut connection,
        Command::Cancel {
            owner: "alice".into(),
            request: "request-0".into(),
            budget: budget(),
        },
    )
    .unwrap();
    begin(&mut connection, "alice", "ninth", input()).unwrap();
    assert_eq!(
        begin(&mut connection, "alice", "request-0", input())
            .unwrap()
            .phase,
        PublisherOperationPhase::Cancelled
    );
}

#[test]
fn existing_publisher_decisions_and_other_requests_cannot_reuse_review_nonce() {
    let mut connection = database();
    begin(&mut connection, "alice", "first", input()).unwrap();
    begin(&mut connection, "alice", "second", input()).unwrap();
    let first = arm(&mut connection, "alice", "first", "shared");
    let arm_existing = |request: &str| Command::Arm {
        owner: "alice".into(),
        request: request.into(),
        interaction: "shared".into(),
        deadline: Instant::now() + Duration::from_secs(60),
        budget: budget(),
    };
    assert!(matches!(
        apply::execute(&mut connection, arm_existing("second")),
        Err(PublisherOperationError::Conflict)
    ));
    // An independent trusted writer command cannot make an old receipt stand
    // in for the fresh human decision expected by this operation.
    PublisherTrustRegistry::new(&mut connection)
        .enroll(
            "alice",
            &input().publisher().unwrap(),
            0,
            &TrustDecision {
                decision_id: "shared".into(),
                authority_ref: "kernel_operation_human".into(),
            },
            1,
        )
        .unwrap();
    PublisherTrustRegistry::new(&mut connection)
        .revoke(
            "alice",
            "local.developer",
            "development",
            1,
            &TrustDecision {
                decision_id: "revoke".into(),
                authority_ref: "kernel".into(),
            },
            2,
        )
        .unwrap();
    assert_eq!(
        decide(&mut connection, first, true),
        Err(PublisherOperationError::Conflict)
    );
    assert_eq!(
        store::load(&connection, "alice", "first").unwrap().phase,
        PublisherOperationPhase::Pending
    );
    assert!(
        !PublisherTrustRegistry::new(&mut connection)
            .get("alice", "local.developer", "development")
            .unwrap()
            .enrolled
    );
}

#[test]
fn uncertain_commit_requests_writer_stop_even_when_response_receiver_disappears() {
    let fatal = Arc::new(AtomicBool::new(false));
    let (response, receive) = mpsc::channel();
    let observed = fatal.clone();
    let reader = std::thread::spawn(move || {
        assert!(matches!(
            receive.recv().unwrap(),
            Err(PublisherOperationError::CommitUnknown)
        ));
        assert!(
            observed.load(Ordering::Acquire),
            "uncertain reply must not precede the reader fence"
        );
    });
    assert!(matches!(
        respond(
            response,
            Err(PublisherOperationError::CommitUnknown),
            &fatal
        ),
        super::super::app_event_delivery::WriterDisposition::Stop
    ));
    reader.join().unwrap();
    let fatal = AtomicBool::new(false);
    let (response, receive) = mpsc::channel();
    drop(receive);
    assert!(matches!(
        respond(
            response,
            Err(PublisherOperationError::CommitUnknown),
            &fatal
        ),
        super::super::app_event_delivery::WriterDisposition::Stop
    ));
    assert!(fatal.load(Ordering::Acquire));
    let fatal = AtomicBool::new(false);
    let (response, _receive) = mpsc::channel();
    assert!(matches!(
        respond(response, Err(PublisherOperationError::Conflict), &fatal),
        super::super::app_event_delivery::WriterDisposition::Continue
    ));
    assert!(!fatal.load(Ordering::Acquire));
}
