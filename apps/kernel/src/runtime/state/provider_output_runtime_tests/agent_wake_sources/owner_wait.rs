//! Row 430/431: a machine acquisition card never holds the agent's tool call past the provider's
//! cut; the owner's answer reaches the agent later, and a card nobody answered is timed out, not
//! denied.
use super::*;
use crate::local::{
    ControlPlaneRequest, KernelConnectionClass, LocalDaemonRequest, MachineAllocationCapability,
};

fn acquire(f: &Fixture, caller: &str) -> crate::runtime::command::KernelCommand {
    let mut command = crate::runtime::command::KernelCommand::from_local_request(
        "acquire",
        None,
        None,
        &LocalDaemonRequest::ControlPlane(request()),
    );
    command.agent_id = Some(f.agent.clone());
    command.session_id = Some(f.room.clone());
    command.caller.caller_id = caller.into();
    command.caller.connection_class = Some(KernelConnectionClass::KernelAgent);
    command
}

fn request() -> ControlPlaneRequest {
    ControlPlaneRequest::Acquire {
        capability_revision: "v1".into(),
        request_id: "request".into(),
        class_id: Some("small".into()),
        region: None,
        timeout_s: None,
    }
}

fn capability() -> MachineAllocationCapability {
    MachineAllocationCapability {
        revision: "v1".into(),
        actions: vec!["acquire".into()],
        offer: None,
        offers: vec![],
        job_default_class_id: None,
        job_offers: vec![],
    }
}

async fn ask(f: &Fixture, caller: &str) -> Result<(), DaemonError> {
    f.runtime
        .approve_machine_acquisition(&acquire(f, caller), &request(), Some(&capability()))
        .await
        .map(|_| ())
}

fn code(result: Result<(), DaemonError>) -> String {
    match result {
        Err(DaemonError::MachineRequest { code, .. }) => code,
        other => panic!("expected a machine request refusal, got {other:?}"),
    }
}

fn cards(f: &Fixture) -> Vec<String> {
    f.runtime
        .owned
        .session_store
        .get_session(&f.room)
        .unwrap()
        .active_interactions()
        .iter()
        .map(|card| card.id().to_owned())
        .collect()
}

fn owner(f: &Fixture) -> String {
    f.runtime
        .owned
        .session_store
        .get_session(&f.room)
        .unwrap()
        .owner_user_id()
        .to_owned()
}

/// The answer events the agent's inbox holds for its owner wait.
async fn answers(f: &Fixture) -> Vec<serde_json::Value> {
    let wake = f
        .store()
        .agent_wakes(Some(&f.room), Some(&f.agent))
        .unwrap()
        .into_iter()
        .find(|wake| wake.kind == "owner_wait")
        .expect("the answer has a wake");
    for _ in 0..200 {
        let answers: Vec<_> = f
            .store()
            .agent_inbox(&f.room, &f.agent, 0)
            .unwrap()
            .into_iter()
            .filter(|event| event.source_id == wake.id)
            .map(|event| event.payload)
            .collect();
        if !answers.is_empty() {
            return answers;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!(
        "no answer event: wake={:?} inbox={:?}",
        f.store().agent_wake(&wake.id).unwrap(),
        f.store().agent_inbox(&f.room, &f.agent, 0).unwrap()
    );
}

async fn waiting(f: &Fixture) -> String {
    let started = tokio::time::Instant::now();
    let result = ask(f, "agent-tool").await;
    assert!(
        started.elapsed() <= super::super::super::owner_wait::IN_CALL + Duration::from_secs(1),
        "the tool call returns inside the provider's cut"
    );
    assert_eq!(code(result), "waiting_for_owner");
    let [card] = cards(f).try_into().expect("one open card");
    card
}

#[tokio::test(start_paused = true)]
async fn an_unanswered_machine_card_returns_waiting_and_the_approval_reaches_the_agent() {
    let f = fixture("owner-wait-approve", true).await;
    let task = f.begin(&f.prompt_id);
    let card = waiting(&f).await;
    let wake = f
        .store()
        .agent_wakes(Some(&f.room), Some(&f.agent))
        .unwrap()
        .into_iter()
        .find(|wake| wake.kind == "owner_wait")
        .expect("the answer has a wake on the asking task");
    assert_eq!(wake.task_id, task.task_id);
    assert_eq!(wake.state, "running");
    // Asking again while the card is open shows the same card.
    assert_eq!(code(ask(&f, "agent-tool").await), "waiting_for_owner");
    assert_eq!(cards(&f), vec![card.clone()]);
    f.runtime
        .resolve_terminal_runtime_interaction(&f.room, &card, "approve", None, Some(&owner(&f)))
        .await
        .unwrap();
    let answers = answers(&f).await;
    assert!(
        answers
            .iter()
            .any(|a| a.to_string().contains("\"approved\"")),
        "{answers:?}"
    );
    // The approval is recorded: the retry acquires with no second card.
    ask(&f, "agent-tool").await.unwrap();
    assert!(cards(&f).is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_machine_card_the_kernel_times_out_reaches_the_agent_as_timed_out_never_denied() {
    let f = fixture("owner-wait-timeout", true).await;
    f.begin(&f.prompt_id);
    waiting(&f).await;
    tokio::time::sleep(Duration::from_secs(300)).await;
    let answers = answers(&f).await;
    let text = serde_json::Value::from(answers).to_string();
    assert!(text.contains("\"timed_out\""), "{text}");
    assert!(!text.contains("denied"), "{text}");
    assert!(cards(&f).is_empty(), "the kernel withdrew the card");
    // The owner's view says it ran out; it never reads as a denial.
    let receipts = f
        .runtime
        .owned
        .session_store
        .get_session(&f.room)
        .unwrap()
        .machine_approval_receipts()
        .to_vec();
    assert_eq!(receipts.len(), 1, "{receipts:?}");
    assert_eq!(
        receipts[0].state,
        crate::session::MachineApprovalReceiptState::Expired
    );
}

#[tokio::test(start_paused = true)]
async fn a_job_acquisition_waits_for_the_whole_card_and_times_out_not_denied() {
    let f = fixture("owner-wait-job-timeout", true).await;
    let job = ControlPlaneRequest::AcquireJob {
        capability_revision: "v1".into(),
        request_id: "job-request".into(),
        job_ref: "job-1".into(),
        needs: crate::local::JobNeeds::default(),
        timeout_s: 60,
    };
    let mut capability = capability();
    capability.job_default_class_id = Some("build".into());
    capability.job_offers = vec![crate::local::JobMachineOffer {
        class_id: "build".into(),
        vcpu: 4,
        usable_memory_mib: 7000,
        disk_gib: 40,
        rate_per_minute: crate::local::JobMachineRate {
            amount_micros: 600,
            currency: "USD".into(),
        },
    }];
    let started = tokio::time::Instant::now();
    let result = f
        .runtime
        .approve_machine_acquisition(&acquire(&f, "job:job-1"), &job, Some(&capability))
        .await
        .map(|_| ());
    // The job is no provider tool call: it holds for the whole 300 s card, then reports it unanswered.
    assert!(started.elapsed() >= Duration::from_secs(300));
    assert_eq!(code(result), "acquisition_unanswered");
}

#[tokio::test(start_paused = true)]
async fn an_explicit_deny_reaches_the_agent_as_denied() {
    let f = fixture("owner-wait-deny", true).await;
    f.begin(&f.prompt_id);
    let card = waiting(&f).await;
    f.runtime
        .resolve_terminal_runtime_interaction(&f.room, &card, "deny", None, Some(&owner(&f)))
        .await
        .unwrap();
    let text = serde_json::Value::from(answers(&f).await).to_string();
    assert!(text.contains("\"denied\""), "{text}");
}

/// Cardregion (hot reload read as the person's Cancel): a restart while the owner's card is open
/// is told as what it was, never as a cancel, a denial or a time-out.
#[tokio::test(start_paused = true)]
async fn a_restart_while_the_card_is_open_never_reads_as_cancelled_denied_or_timed_out() {
    let f = fixture("owner-wait-restart", true).await;
    f.begin(&f.prompt_id);
    waiting(&f).await;
    let wake = f
        .store()
        .agent_wakes(Some(&f.room), Some(&f.agent))
        .unwrap()
        .into_iter()
        .find(|wake| wake.kind == "owner_wait")
        .expect("the answer has a wake");
    // The old kernel stops under the open card: the wait is not settled as a time-out (the next
    // kernel reports it).
    f.runtime.shutdown_cleanup().await.unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    let told_by_old_kernel = f
        .store()
        .agent_inbox(&f.room, &f.agent, 0)
        .unwrap()
        .into_iter()
        .filter(|event| event.source_id == wake.id)
        .count();
    assert_eq!(told_by_old_kernel, 0, "the old kernel says nothing as it stops");
    assert_eq!(f.wake_state(&wake.id), "running");
    // The new kernel recovers the wake the old one armed (the scheduler's start-up recovery).
    let recovery = f
        .runtime
        .owned
        .agent_wakes
        .drivers
        .get("owner_wait")
        .unwrap()
        .recover(&wake);
    let super::super::super::wake_driver::Recovery::Lost(reason) = recovery else {
        panic!("the restart ended the wait");
    };
    f.runtime.withdraw_owner_wait_card(&wake);
    f.runtime
        .report_lost_wake(
            &wake,
            crate::session::unix_epoch_ms(),
            &format!("the kernel restarted: {reason}"),
        )
        .unwrap();
    let told = f
        .store()
        .agent_inbox(&f.room, &f.agent, 0)
        .unwrap()
        .into_iter()
        .filter(|event| event.source_id == wake.id)
        .map(|event| event.payload.to_string().to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        told.contains("restart") && told.contains("same request_id"),
        "{told}"
    );
    for word in ["cancel", "denied", "timed_out", "timed out"] {
        assert!(!told.contains(word), "{word}: {told}");
    }
    // The card whose buttons would answer nothing is gone.
    assert!(cards(&f).is_empty());
}
