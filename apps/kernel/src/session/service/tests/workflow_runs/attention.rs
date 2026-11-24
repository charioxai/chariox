use super::*;
use crate::session::{
    WorkflowFailureEvent, WorkflowFailureKind, WorkflowNodeWaitingReason, WorkflowRun,
    WorkflowRunOutcome, WorkflowWaitingOn,
};

struct Runs {
    service: SessionService,
    session_id: String,
    endpoint: TestWorkflowEndpoint,
}

impl Runs {
    fn new(alias: &str) -> Self {
        let mut service = SessionService::new(&test_config());
        let session = service
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        seed_agents(&mut service, session.id(), &["agent-1"]);
        let endpoint = workflow_with_endpoint(&mut service, session.id(), alias, "agent-1");
        Self {
            service,
            session_id: session.id().to_owned(),
            endpoint,
        }
    }

    fn run(&mut self) -> WorkflowRun {
        self.service
            .invoke_workflow_endpoint(
                &self.session_id,
                self.endpoint.workflow.id(),
                self.endpoint.endpoint.id(),
                Some("go".into()),
            )
            .expect("run is created")
    }

    fn get(&self, run: &WorkflowRun) -> WorkflowRun {
        self.service
            .resolve_workflow_run_ref(&self.session_id, run.id())
            .unwrap()
    }

    fn fail(&mut self, run: &WorkflowRun, kind: WorkflowFailureKind, message: &str) {
        let node_run = run.node_runs()[0].id().to_owned();
        self.service
            .record_workflow_failure_event(
                &self.session_id,
                run.id(),
                WorkflowFailureEvent::new(kind, node_run.clone(), Vec::new(), message),
            )
            .unwrap();
        self.service
            .fail_workflow_node_run(&self.session_id, run.id(), &node_run)
            .unwrap();
    }
}

#[test]
fn runs_are_numbered_per_workflow_from_one() {
    let mut runs = Runs::new("review");
    assert_eq!(
        [
            runs.run().number(),
            runs.run().number(),
            runs.run().number()
        ],
        [1, 2, 3]
    );
    let other = workflow_with_endpoint(
        &mut runs.service,
        &runs.session_id.clone(),
        "other",
        "agent-1",
    );
    let first_of_other = runs
        .service
        .invoke_workflow_endpoint(
            &runs.session_id,
            other.workflow.id(),
            other.endpoint.id(),
            None,
        )
        .unwrap();
    assert_eq!(
        first_of_other.number(),
        1,
        "each workflow counts its own runs"
    );
}

#[test]
fn a_failed_run_needs_attention_until_it_is_cleared() {
    let mut runs = Runs::new("review");
    let run = runs.run();
    assert!(runs.get(&run).outcome().is_none() && !runs.get(&run).attention());

    runs.fail(&run, WorkflowFailureKind::StepNeedsHelp, "resend the diff");
    let failed = runs.get(&run);
    assert_eq!(
        failed.outcome(),
        Some(WorkflowRunOutcome::Failed {
            reason: WorkflowFailureKind::StepNeedsHelp,
            detail: "resend the diff".into(),
        })
    );
    assert!(failed.attention());
    let wire = serde_json::to_value(&failed).unwrap();
    assert_eq!(wire["attention"], true);
    assert_eq!(wire["number"], 1);
    assert_eq!(wire["outcome"]["kind"], "failed");
    assert_eq!(wire["outcome"]["reason"], "step_needs_help");

    let cleared = runs
        .service
        .clear_workflow_run_attention(&runs.session_id, run.id())
        .unwrap();
    assert!(!cleared.attention());
    assert_eq!(
        serde_json::to_value(&cleared).unwrap()["attention"],
        false,
        "the outcome stays; only the attention closes"
    );
}

#[test]
fn a_newer_run_closes_the_attention_of_the_older_ones() {
    let mut runs = Runs::new("review");
    let first = runs.run();
    runs.fail(
        &first,
        WorkflowFailureKind::ProviderFailure,
        "provider down",
    );
    assert!(runs.get(&first).attention());
    let second = runs.run();
    assert!(!runs.get(&first).attention(), "the person moved on");
    assert_eq!(second.number(), 2);
    assert!(!runs.get(&second).attention());
}

#[test]
fn a_stopped_run_needs_no_one() {
    let mut runs = Runs::new("review");
    let run = runs.run();
    runs.service
        .record_workflow_failure_event(
            &runs.session_id,
            run.id(),
            WorkflowFailureEvent::new(
                WorkflowFailureKind::RunStopped,
                run.node_runs()[0].id(),
                Vec::new(),
                "stopped",
            ),
        )
        .unwrap();
    assert!(runs.get(&run).outcome().is_none());
}

#[test]
fn workspace_claim_wait_names_the_current_holder_until_dispatch() {
    let mut runs = Runs::new("review");
    let run = runs.run();
    let node = &run.node_runs()[0];
    let session_id = &runs.session_id;

    for holding_run_id in ["run-one", "run-two"] {
        assert!(runs
            .service
            .block_workflow_node_on_workspace_claim(
                session_id,
                run.id(),
                node.id(),
                Some(holding_run_id)
            )
            .unwrap()
            .is_some());
        assert!(runs
            .service
            .block_workflow_node_on_workspace_claim(
                session_id,
                run.id(),
                node.id(),
                Some(holding_run_id)
            )
            .unwrap()
            .is_none());
        let waiting = runs.get(&run);
        let detail = format!("waiting for the workspace, held by run {holding_run_id}");
        assert_eq!(waiting.status_notice_suffix(), detail);
        assert_eq!(
            serde_json::to_value(&waiting).unwrap()["outcome"]["detail"],
            detail
        );
    }

    runs.service
        .ready_workflow_node_after_workspace_claim(session_id, run.id(), node.id())
        .unwrap();
    assert!(runs.get(&run).outcome().is_none());
}

#[test]
fn a_run_holding_a_node_on_a_named_thing_needs_attention_only_while_it_holds() {
    let mut runs = Runs::new("review");
    let run = runs.run();
    let node_run = run.node_runs()[0].id().to_owned();
    runs.service
        .hold_workflow_node_run(
            &runs.session_id,
            run.id(),
            &node_run,
            WorkflowNodeWaitingReason::ProviderProfileUsageExhausted,
        )
        .unwrap();
    let held = runs.get(&run);
    assert_eq!(
        held.outcome(),
        Some(WorkflowRunOutcome::Waiting {
            on: WorkflowWaitingOn::ProviderProfileUsageExhausted,
            node_id: run.node_runs()[0].node_id().into(),
            detail: None,
        })
    );
    assert!(held.attention());
    let wire = serde_json::to_value(&held).unwrap();
    assert_eq!(wire["outcome"]["kind"], "waiting");
    assert_eq!(wire["outcome"]["on"], "provider_profile_usage_exhausted");

    // The person opens it, then a different wait on the same node opens attention again.
    runs.service
        .clear_workflow_run_attention(&runs.session_id, run.id())
        .unwrap();
    assert!(!runs.get(&run).attention());
    runs.service
        .start_workflow_node_run(&runs.session_id, run.id(), &node_run)
        .unwrap();
    assert!(
        runs.get(&run).outcome().is_none(),
        "dispatched: no longer waiting"
    );
}

#[test]
fn a_terminal_run_that_needs_someone_stays_in_the_snapshot_until_cleared() {
    let mut runs = Runs::new("review");
    let ended = runs.run();
    runs.fail(
        &ended,
        WorkflowFailureKind::StepNeedsHelp,
        "needs the browser",
    );
    let archived = runs
        .service
        .archive_terminal_workflow_runs(&runs.session_id)
        .unwrap();
    assert!(archived.is_empty(), "attention keeps it in the snapshot");
    runs.service
        .clear_workflow_run_attention(&runs.session_id, ended.id())
        .unwrap();
    let archived = runs
        .service
        .archive_terminal_workflow_runs(&runs.session_id)
        .unwrap();
    assert_eq!(archived.len(), 1);
    assert_eq!(archived[0].id(), ended.id());
}

#[test]
fn a_run_round_trips_through_its_own_wire_form() {
    let mut runs = Runs::new("review");
    let run = runs.run();
    runs.fail(&run, WorkflowFailureKind::StepNeedsHelp, "reason");
    let failed = runs.get(&run);
    let text = serde_json::to_string(&failed).unwrap();
    let restored: WorkflowRun = serde_json::from_str(&text).unwrap();
    assert_eq!(restored, failed);
    assert_eq!(restored.number(), 1);
    assert!(restored.attention());
}
