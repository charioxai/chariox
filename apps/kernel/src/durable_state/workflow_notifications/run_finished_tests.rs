//! MP-08 / MP-09 / MP-10 / MP-11 (ON-17, AT-40): the end of any workflow run is a
//! followable source, on the real durable writer. Supplementary regressions, not live
//! acceptance.
use super::outside_agent_tests::{accept, append, events, follow};
use super::run_finished::RUN_FINISHED_NODE;
use super::tests::{cleanup, Fixture};
use super::*;
use crate::local::{NotificationSourceKind, OutsideEventValue};
use crate::session::{WorkflowFailureEvent, WorkflowFailureKind, WorkflowRunStatus};

const PROMPT: &str =
    "Handle GitHub pull_request.synchronize for charioxai/chariox #1279 at head 0e8b5e6fd8.";

fn run_source(f: &Fixture, w: &str) -> WorkflowNotificationSource {
    let workflow = f.sessions.resolve_workflow_ref(&f.session, w).unwrap();
    let s = f.sessions.get_session(&f.session).unwrap();
    match f
        .store
        .notify(NotificationOperation::Register(SourceAdmission {
            source: WorkflowNotificationSource {
                kind: NotificationSourceKind::WorkflowRun,
                source_id: format!("run-source-{w}"),
                owner_user_id: "local".into(),
                kernel_id: s.host_daemon_id().into(),
                session_id: f.session.clone(),
                workflow_id: w.into(),
                exit_node_id: RUN_FINISHED_NODE.into(),
                enabled: true,
                available: true,
                name: naming::workflow_name(&workflow),
                exit_ordinal: 0,
                display_name: String::new(),
            },
            workflow_json: encode(&workflow).unwrap(),
        }))
        .unwrap()
    {
        NotificationOutcome::Source(source) => source,
        other => panic!("{other:?}"),
    }
}
/// A run that ends without reaching an exit, with the prompt a webhook gave it.
fn end_run(f: &mut Fixture, w: &str, id: &str, status: WorkflowRunStatus, failure: Option<&str>) {
    let mut run = crate::session::WorkflowRun::new(
        id,
        w,
        "endpoint",
        "node",
        Some(PROMPT.into()),
        None,
        vec![],
        vec![],
    );
    if let Some(message) = failure {
        run.add_failure_event(WorkflowFailureEvent::new(
            WorkflowFailureKind::ProviderFailure,
            "node-run",
            vec![],
            message,
        ));
    }
    run.set_status(status);
    let mut session = f.sessions.get_session(&f.session).unwrap();
    session.create_workflow_run(run);
    f.sessions.restore_session(session);
    f.persist();
}
fn delivered(f: &Fixture, sub: &WorkflowNotificationSubscription) -> Vec<serde_json::Value> {
    while let Some((candidate, env)) = f.candidates(false).into_iter().next() {
        assert_eq!(candidate.subscription_id, sub.subscription_id);
        accept(f, sub, &env).unwrap();
        append(f, sub, &env);
    }
    for (candidate, env) in f.candidates(true) {
        assert_eq!(candidate.subscription_id, sub.subscription_id);
        append(f, sub, &env);
    }
    events(f)
        .into_iter()
        .map(|event| {
            let OutsideEventValue::Notification {
                source_kind,
                output,
                occurrence_id,
                ..
            } = event.value
            else {
                panic!("expected a notification, got {event:?}");
            };
            assert_eq!(source_kind, NotificationSourceKind::WorkflowRun);
            let mut body: serde_json::Value = serde_json::from_str(output.message()).unwrap();
            assert_eq!(body["run_id"], occurrence_id);
            body["occurrence"] = occurrence_id.into();
            body
        })
        .collect()
}

#[test]
fn a_failed_run_notifies_with_its_status_prompt_and_failure_message() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("reviewer");
    let source = run_source(&f, &a);
    let sub = follow(&f, &source);
    end_run(
        &mut f,
        &a,
        "run-failed",
        WorkflowRunStatus::Failed,
        Some("review agent could not post to GitHub"),
    );
    let got = delivered(&f, &sub);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0]["status"], "Failed");
    assert_eq!(got[0]["prompt"], PROMPT);
    assert_eq!(got[0]["failure"], "review agent could not post to GitHub");
    assert_eq!(got[0]["workflow_id"], a.as_str());
    cleanup(f);
}

#[test]
fn a_completed_run_notifies_once_and_a_stopped_run_never() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("reviewer");
    let source = run_source(&f, &a);
    let sub = follow(&f, &source);
    f.complete(&a, "run-ok", None, WorkflowRunStatus::Completed, "approved");
    end_run(&mut f, &a, "run-stopped", WorkflowRunStatus::Stopped, None);
    f.persist();
    let got = delivered(&f, &sub);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0]["status"], "Completed");
    assert_eq!(got[0]["occurrence"], "run-ok");
    assert!(got[0].get("failure").is_none(), "{got:?}");
    cleanup(f);
}

#[test]
fn a_run_ending_through_a_notifying_exit_also_reaches_the_exit_source_unchanged() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("reviewer");
    let exit = f.source(&a);
    let run = run_source(&f, &a);
    f.complete(
        &a,
        "run-both",
        None,
        WorkflowRunStatus::Completed,
        "approved",
    );
    let ids: Vec<_> = f
        .candidates(false)
        .into_iter()
        .map(|(_, env)| env.source_id)
        .collect();
    assert!(ids.is_empty(), "no subscriber, no receipt: {ids:?}");
    let occurrences: Vec<String> = {
        let db = f.store.lock_connection("test").unwrap();
        let mut q = db
            .prepare("SELECT source_id FROM workflow_notification_occurrences ORDER BY source_id")
            .unwrap();
        let rows = q.query_map([], |r| r.get(0)).unwrap();
        rows.map(|r| r.unwrap()).collect()
    };
    assert_eq!(occurrences.len(), 2, "{occurrences:?}");
    assert!(occurrences.contains(&exit.source_id) && occurrences.contains(&run.source_id));
    cleanup(f);
}

#[test]
fn an_exit_node_named_like_the_run_source_keeps_its_own_source_and_output() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("reviewer");
    let (b, _, bp) = f.workflow("subscriber");
    let exit = f.source_of_exit(&a, RUN_FINISHED_NODE);
    let run = run_source(&f, &a);
    assert_ne!(
        exit.source_id, run.source_id,
        "the run source took over the exit's source"
    );
    assert_eq!(exit.kind, NotificationSourceKind::WorkflowExit);
    assert_eq!(run.kind, NotificationSourceKind::WorkflowRun);
    let listed = f.store.notification_inventory("local").unwrap().0;
    for (source, kind) in [
        (&exit, NotificationSourceKind::WorkflowExit),
        (&run, NotificationSourceKind::WorkflowRun),
    ] {
        assert!(
            listed
                .iter()
                .any(|s| s.source_id == source.source_id && s.kind == kind),
            "{:?} is not listed as {kind:?}: {listed:?}",
            source.source_id
        );
    }
    f.attach(&exit, &b, &bp);
    f.attach(&run, &b, &bp);
    f.complete_through(
        &a,
        RUN_FINISHED_NODE,
        "run-named",
        None,
        WorkflowRunStatus::Completed,
        "approved",
    );
    let outputs: std::collections::BTreeMap<String, String> = f
        .candidates(false)
        .into_iter()
        .map(|(_, env)| (env.source_id, env.output.message().to_owned()))
        .collect();
    assert_eq!(
        outputs.get(&exit.source_id).map(String::as_str),
        Some("approved"),
        "{outputs:?}"
    );
    let run_body: serde_json::Value =
        serde_json::from_str(&outputs[&run.source_id]).expect("the run source reports the run");
    assert_eq!(run_body["status"], "Completed");
    cleanup(f);
}

/// A run holding its entry node on a named thing: it neither ended nor stopped.
fn hold_run(f: &mut Fixture, w: &str, id: &str, reason: crate::session::WorkflowNodeWaitingReason) {
    let mut session = f.sessions.get_session(&f.session).unwrap();
    if session.workflow_run(id).is_none() {
        let node = crate::session::WorkflowNodeRun::new(
            format!("{id}-node-run"),
            "node",
            "agent",
            0,
            crate::session::WorkflowNodeRunStatus::Ready,
        );
        let mut run = crate::session::WorkflowRun::new(
            id,
            w,
            "endpoint",
            "node",
            Some(PROMPT.into()),
            None,
            vec![node],
            vec![],
        );
        run.set_status(WorkflowRunStatus::Waiting);
        session.create_workflow_run(run);
    }
    let run = session.workflow_run_mut(id).unwrap();
    run.node_runs_mut()[0].set_waiting_reason(reason);
    f.sessions.restore_session(session);
    f.persist();
}

fn occurrences(f: &Fixture, source: &WorkflowNotificationSource) -> Vec<String> {
    let db = f.store.lock_connection("test").unwrap();
    let mut q = db
        .prepare("SELECT occurrence_id FROM workflow_notification_occurrences WHERE source_id=?1 ORDER BY rowid")
        .unwrap();
    let rows = q.query_map([&source.source_id], |r| r.get(0)).unwrap();
    rows.map(|r| r.unwrap()).collect()
}

#[test]
fn a_finished_run_carries_its_number_workflow_name_and_typed_outcome() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("reviewer");
    let (b, _, bp) = f.workflow("subscriber");
    let source = run_source(&f, &a);
    f.attach(&source, &b, &bp);
    end_run(
        &mut f,
        &a,
        "run-failed",
        WorkflowRunStatus::Failed,
        Some("provider down"),
    );
    let body: serde_json::Value = serde_json::from_str(
        f.candidates(false)
            .into_iter()
            .next()
            .expect("the run is announced")
            .1
            .output
            .message(),
    )
    .unwrap();
    assert_eq!(body["number"], 1);
    assert_eq!(body["workflow_name"], "reviewer");
    assert_eq!(
        body["outcome"],
        serde_json::json!({"kind": "failed", "reason": "provider_failure", "detail": "provider down"})
    );
    cleanup(f);
}

#[test]
fn a_run_entering_waiting_on_a_named_thing_notifies_once_per_node_and_reason() {
    use crate::session::WorkflowNodeWaitingReason::{
        ProviderProfileUnauthenticated, ProviderProfileUsageExhausted,
    };
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("reviewer");
    let source = run_source(&f, &a);
    hold_run(&mut f, &a, "run-held", ProviderProfileUsageExhausted);
    hold_run(&mut f, &a, "run-held", ProviderProfileUsageExhausted);
    f.persist();
    assert_eq!(
        occurrences(&f, &source).len(),
        1,
        "the same hold is one occurrence however often the run is written"
    );
    hold_run(&mut f, &a, "run-held", ProviderProfileUnauthenticated);
    assert_eq!(
        occurrences(&f, &source).len(),
        2,
        "another reason is another need"
    );
    cleanup(f);
}

#[test]
fn the_person_is_alerted_once_for_a_run_that_needs_them_and_an_agent_responsible_is_not() {
    use crate::session::WorkflowNodeWaitingReason::ProviderProfileUsageExhausted;
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("reviewer");
    let (b, _, _) = f.workflow("handled");
    let session = f.sessions.get_session(&f.session).unwrap();
    let alert = match f
        .store
        .notify(NotificationOperation::Register(SourceAdmission {
            source: WorkflowNotificationSource {
                kind: NotificationSourceKind::OwnerAction,
                source_id: "owner-action".into(),
                owner_user_id: "local".into(),
                kernel_id: session.host_daemon_id().into(),
                session_id: f.session.clone(),
                workflow_id: "owner_action".into(),
                exit_node_id: "blocked".into(),
                enabled: true,
                available: true,
                name: "Agent needs your action".into(),
                exit_ordinal: 0,
                display_name: "Agent needs your action".into(),
            },
            workflow_json: String::new(),
        }))
        .unwrap()
    {
        NotificationOutcome::Source(source) => source,
        other => panic!("{other:?}"),
    };
    let mut session = f.sessions.get_session(&f.session).unwrap();
    session.workflow_mut(&b).unwrap().set_responsible(
        crate::session::WorkflowResponsible::Agent {
            kernel_id: "daemon-test".into(),
            room_id: f.session.clone(),
            agent_id: "agent".into(),
            label: "agent".into(),
        },
        None,
    );
    f.sessions.restore_session(session);

    end_run(
        &mut f,
        &a,
        "run-one",
        WorkflowRunStatus::Failed,
        Some("provider down"),
    );
    f.persist();
    hold_run(&mut f, &a, "run-two", ProviderProfileUsageExhausted);
    end_run(
        &mut f,
        &b,
        "run-three",
        WorkflowRunStatus::Failed,
        Some("provider down"),
    );
    assert_eq!(
        occurrences(&f, &alert),
        vec![
            "run-run-one-failed:ProviderFailure".to_owned(),
            "run-run-two-waiting:node:ProviderProfileUsageExhausted".to_owned()
        ],
        "one alert per person-owned run and need; none for the agent's workflow"
    );
    cleanup(f);
}
