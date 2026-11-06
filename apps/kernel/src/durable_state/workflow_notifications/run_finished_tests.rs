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
                name: String::new(),
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
