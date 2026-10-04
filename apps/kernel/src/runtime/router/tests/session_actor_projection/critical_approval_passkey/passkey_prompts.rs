//! Protocol 403: a critical approval is a passkey prompt for every terminal
//! of its owner. The first correct passkey or refusal closes it everywhere,
//! a later answer is told it was already answered, a wrong passkey leaves it
//! open and counts, and it expires with its decision.
use super::*;
use crate::local::{PasskeyPrompt, PasskeyPromptKind};
use crate::runtime::state::PASSKEY_ALREADY_ANSWERED;

impl Fixture {
    pub(super) fn prompts(&self, user_id: &str) -> Vec<PasskeyPrompt> {
        self.router.runtime_state.passkey_prompts_for(user_id)
    }

    /// The owner's open prompts, by fixture name.
    pub(super) fn prompt_ids(&self) -> Vec<String> {
        self.prompts(DEFAULT_LOCAL_USER_ID)
            .into_iter()
            .map(|prompt| prompt.interaction_id)
            .collect()
    }

    fn changes(&self) -> u64 {
        self.router.runtime_state.passkey_prompt_change_sequence()
    }

    async fn critical_expiring_in(
        &self,
        name: &str,
        seconds: u64,
    ) -> tokio::sync::oneshot::Receiver<crate::runtime::state::PendingInteractionResolution> {
        self.router
            .runtime_state
            .create_kernel_operation_interaction(
                &self.session,
                DEFAULT_LOCAL_USER_ID,
                RuntimeInteraction::for_kernel_operation(
                    id(name),
                    format!("validation:{}", id(name)),
                    "Approve App action",
                    "An App asks to perform a protected action.",
                    vec![
                        RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
                        RuntimeInteractionChoice::new("approve", "Approve", "allow", None)
                            .requiring_passkey(),
                    ],
                )
                .with_timeout_sec(seconds),
            )
            .await
            .unwrap()
    }
}

fn ids(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| id(name)).collect()
}

#[tokio::test]
async fn a_critical_approval_is_a_passkey_prompt_for_its_owner_alone() {
    let f = Fixture::shared_with("guest");
    let before = f.changes();
    let raised_ms = crate::session::unix_epoch_ms();
    let _decision = f.critical("popup").await;
    assert!(
        f.changes() > before,
        "raising a prompt wakes every terminal"
    );
    let prompts = f.prompts(DEFAULT_LOCAL_USER_ID);
    let [prompt] = prompts.as_slice() else {
        panic!("one prompt expected: {prompts:?}");
    };
    // Only what the kernel registered.
    assert_eq!(prompt.kind, PasskeyPromptKind::CriticalApproval);
    assert_eq!(prompt.session_id, f.session);
    assert_eq!(prompt.session_alias, None);
    assert_eq!(prompt.interaction_id, id("popup"));
    assert_eq!(prompt.title, "Approve App action");
    assert_eq!(prompt.message, "An App asks to perform a protected action.");
    assert_eq!(prompt.approve_choice_id, "approve");
    assert_eq!(prompt.refuse_choice_id, "deny");
    assert!(prompt.requested_at_ms >= raised_ms);
    assert!(
        (raised_ms + 300_000..=crate::session::unix_epoch_ms() + 300_000)
            .contains(&prompt.expires_at_ms)
    );
    // In a shared session the guest gets no popup: only the owner answers.
    assert!(f.prompts("guest").is_empty());
    let guest_answer = f
        .router
        .runtime_state
        .answer_terminal_runtime_interaction(
            &f.session,
            &id("popup"),
            "deny",
            None,
            Some("guest"),
            None,
            None,
            Some(KernelConnectionClass::Terminal),
        )
        .await;
    assert!(guest_answer.is_err());
    // A routine kernel decision is no passkey prompt.
    let _routine = f
        .router
        .runtime_state
        .create_kernel_operation_interaction(
            &f.session,
            DEFAULT_LOCAL_USER_ID,
            RuntimeInteraction::for_kernel_operation(
                id("routine"),
                format!("install:{}", id("routine")),
                "Install App?",
                "Review this release",
                vec![RuntimeInteractionChoice::new(
                    "allow", "Install", "allow", None,
                )],
            ),
        )
        .await
        .unwrap();
    assert_eq!(f.prompt_ids(), ids(&["popup"]));
}

#[tokio::test]
async fn a_passkey_prompt_has_one_approve_and_one_refuse_choice() {
    let f = Fixture::new(true);
    let refused = f
        .router
        .runtime_state
        .create_kernel_operation_interaction(
            &f.session,
            DEFAULT_LOCAL_USER_ID,
            RuntimeInteraction::for_kernel_operation(
                id("shape"),
                format!("validation:{}", id("shape")),
                "Approve App action",
                "Two ways to approve",
                vec![
                    RuntimeInteractionChoice::new("once", "Once", "allow", None)
                        .requiring_passkey(),
                    RuntimeInteractionChoice::new("always", "Always", "allow", None)
                        .requiring_passkey(),
                ],
            ),
        )
        .await
        .expect_err("a prompt needs one refuse choice");
    assert!(refused.to_string().contains("passkey prompt"), "{refused}");
    assert!(f.prompt_ids().is_empty());
    assert_eq!(f.active().await, 0);
}

#[tokio::test]
async fn the_first_answer_closes_the_prompt_and_a_later_one_is_already_answered() {
    let f = Fixture::new(true);
    let approved = f.critical("approved").await;
    let refused = f.critical("refused").await;
    assert_eq!(f.prompt_ids(), ids(&["approved", "refused"]));
    let raised = f.changes();
    // A wrong passkey answers nothing: the prompt stays open everywhere.
    Fixture::refused_with(
        f.answer("approved", "approve", Some("wrong"), None).await,
        "PASSKEY_REJECTED",
    );
    assert_eq!(f.prompt_ids(), ids(&["approved", "refused"]));
    // The first correct passkey resolves it and closes it on every terminal.
    f.answer("approved", "approve", Some(PASSKEY), None)
        .await
        .unwrap();
    assert!(f.changes() > raised, "every terminal is told to close it");
    assert_eq!(f.prompt_ids(), ids(&["refused"]));
    assert_eq!(
        approved.await.unwrap().choice_id.as_deref(),
        Some("approve")
    );
    // A later answer, right, wrong or refusing, is already answered: never
    // verified, audited or counted.
    for _ in 0..3 {
        for (choice, passkey) in [
            ("approve", Some(PASSKEY)),
            ("approve", Some("wrong")),
            ("deny", None),
        ] {
            Fixture::refused_with(
                f.answer("approved", choice, passkey, None).await,
                PASSKEY_ALREADY_ANSWERED,
            );
        }
    }
    assert_eq!(f.outcomes("approved"), ["rejected", "verified"]);
    // Refusing first closes the prompt the same way.
    f.answer("refused", "deny", None, None).await.unwrap();
    assert!(f.prompt_ids().is_empty());
    assert_eq!(refused.await.unwrap().choice_id.as_deref(), Some("deny"));
    Fixture::refused_with(
        f.answer("refused", "approve", Some(PASSKEY), None).await,
        PASSKEY_ALREADY_ANSWERED,
    );
    assert!(f.outcomes("refused").is_empty());
    // Six late wrong passkeys locked nobody out.
    let next = f.critical("next").await;
    f.answer("next", "approve", Some(PASSKEY), None)
        .await
        .unwrap();
    assert_eq!(next.await.unwrap().choice_id.as_deref(), Some("approve"));
}

#[tokio::test]
async fn two_terminals_answering_at_once_resolve_the_prompt_once() {
    let f = Fixture::new(true);
    let decision = f.critical("raced").await;
    let passkey = crate::local::ApprovalPasskey::new(PASSKEY);
    let interaction_id = id("raced");
    let answer = || {
        f.router.runtime_state.answer_terminal_runtime_interaction(
            &f.session,
            &interaction_id,
            "approve",
            None,
            Some(DEFAULT_LOCAL_USER_ID),
            Some(&passkey),
            None,
            Some(KernelConnectionClass::Terminal),
        )
    };
    let (first, second) = tokio::join!(answer(), answer());
    let results = [first, second];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let late = results
        .into_iter()
        .find_map(Result::err)
        .unwrap()
        .to_string();
    assert!(late.contains(PASSKEY_ALREADY_ANSWERED), "{late}");
    assert_eq!(
        decision.await.unwrap().choice_id.as_deref(),
        Some("approve")
    );
    // The second passkey waited its turn and was never checked.
    assert_eq!(f.outcomes("raced"), ["verified"]);
}

#[tokio::test]
async fn wrong_passkeys_keep_the_prompt_open_and_count_toward_the_lockout() {
    let f = Fixture::new(true);
    let _decision = f.critical("guessed").await;
    for _ in 0..5 {
        Fixture::refused_with(
            f.answer("guessed", "approve", Some("guess"), None).await,
            "PASSKEY_REJECTED",
        );
        assert_eq!(f.prompt_ids(), ids(&["guessed"]));
    }
    // The sixth attempt is locked out, even with the right passkey.
    Fixture::refused_with(
        f.answer("guessed", "approve", Some(PASSKEY), None).await,
        "PASSKEY_RATE_LIMITED",
    );
    assert_eq!(f.prompt_ids(), ids(&["guessed"]));
    assert_eq!(
        f.outcomes("guessed"),
        [
            "rejected",
            "rejected",
            "rejected",
            "rejected",
            "rejected",
            "rate_limited"
        ]
    );
}

#[tokio::test]
async fn an_unanswered_prompt_expires_with_its_decision() {
    let f = Fixture::new(true);
    let decision = f.critical_expiring_in("expiring", 1).await;
    assert_eq!(f.prompt_ids(), ids(&["expiring"]));
    tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
    // Past its deadline it leaves every terminal, before the kernel pump
    // times the decision out, and a passkey for it is not checked.
    assert!(f.prompt_ids().is_empty());
    Fixture::refused_with(
        f.answer("expiring", "approve", Some(PASSKEY), None).await,
        "expired",
    );
    assert!(f.outcomes("expiring").is_empty());
    // The pump then times it out, and whoever raised it is told.
    let before = f.changes();
    f.router
        .runtime_state
        .timeout_runtime_interaction(&f.session, &id("expiring"))
        .await
        .unwrap();
    assert!(f.changes() > before);
    assert_eq!(decision.await.unwrap().status, "timed_out");
}

#[tokio::test]
async fn no_passkey_reaches_a_log_an_audit_or_a_popup() {
    const WRONG: &str = "wrong-passkey-0b7e5c";
    let capture = crate::logging::capture::start();
    let f = Fixture::new(true);
    let decision = f.critical("secret").await;
    let popup = serde_json::to_string(&f.prompts(DEFAULT_LOCAL_USER_ID)).unwrap();
    Fixture::refused_with(
        f.answer("secret", "approve", Some(WRONG), None).await,
        "PASSKEY_REJECTED",
    );
    f.answer("secret", "approve", Some(PASSKEY), Some(5))
        .await
        .unwrap();
    assert_eq!(
        decision.await.unwrap().choice_id.as_deref(),
        Some("approve")
    );
    Fixture::refused_with(
        f.answer("secret", "approve", Some(WRONG), None).await,
        PASSKEY_ALREADY_ANSWERED,
    );
    let records = capture.records();
    assert!(
        records.iter().any(|record| record.contains(&id("secret"))),
        "the answers were logged"
    );
    let events = f.durable.load_events_after(0).unwrap();
    assert!(events
        .iter()
        .any(|event| event.kind == "critical_approval.passkey"));
    let mut stored = Vec::new();
    for suffix in ["", "-wal"] {
        let path = format!("{}{suffix}", f.durable.path().display());
        stored.extend(std::fs::read(path).unwrap_or_default());
    }
    for secret in [PASSKEY, WRONG] {
        assert!(
            !records.iter().any(|record| record.contains(secret)),
            "a log record holds a passkey"
        );
        assert!(events
            .iter()
            .all(|event| !event.payload.to_string().contains(secret)));
        assert!(!stored
            .windows(secret.len())
            .any(|window| window == secret.as_bytes()));
        assert!(!popup.contains(secret));
    }
}
