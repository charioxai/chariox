use super::*;
use crate::session::WorkflowQueuedPromptStatus;

fn archived_service() -> (SessionService, String, TestWorkflowEndpoint) {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let workflow = workflow_with_endpoint(&mut service, session.id(), "dup", "agent-1");
    (service, session.id().to_string(), workflow)
}

#[test]
fn an_archived_workflow_refuses_runs_and_queueing_with_the_unarchive_command() {
    let (mut service, session_id, workflow) = archived_service();
    service
        .set_workflow_archived(&session_id, workflow.workflow.id(), true)
        .expect("workflow should archive");

    let run_error = service
        .invoke_workflow_endpoint(
            &session_id,
            workflow.workflow.id(),
            workflow.endpoint.id(),
            None,
        )
        .expect_err("an archived workflow must not run");
    assert!(
        run_error.to_string().contains("workflow unarchive dup"),
        "{run_error}"
    );
    let queue_error = service
        .enqueue_workflow_prompt(
            &session_id,
            workflow.workflow.id(),
            workflow.endpoint.id(),
            None,
            Some("default"),
            WorkflowQueuedPromptSource::Manual,
            None,
        )
        .expect_err("an archived workflow must not queue");
    assert!(
        queue_error.to_string().contains("workflow unarchive dup"),
        "{queue_error}"
    );

    let resolved = service
        .resolve_workflow_ref(&session_id, "dup")
        .expect("an archived workflow stays resolvable");
    assert!(resolved.is_archived());
    assert!(resolved.archived_at_ms().is_some());
}

#[test]
fn unarchive_restores_dispatch_at_the_same_revision() {
    let (mut service, session_id, workflow) = archived_service();
    let revision = service
        .resolve_workflow_ref(&session_id, "dup")
        .expect("workflow resolves")
        .revision();
    let archived = service
        .set_workflow_archived(&session_id, "dup", true)
        .expect("workflow should archive");
    let first_stamp = archived.archived_at_ms();
    let again = service
        .set_workflow_archived(&session_id, "dup", true)
        .expect("archiving twice is allowed");
    assert_eq!(again.archived_at_ms(), first_stamp);

    let restored = service
        .set_workflow_archived(&session_id, "dup", false)
        .expect("workflow should unarchive");
    assert!(!restored.is_archived());
    assert_eq!(restored.revision(), revision);
    service
        .invoke_workflow_endpoint(
            &session_id,
            workflow.workflow.id(),
            workflow.endpoint.id(),
            None,
        )
        .expect("an unarchived workflow runs again");
}

#[test]
fn a_prompt_queued_before_archiving_waits_until_unarchive() {
    let (mut service, session_id, workflow) = archived_service();
    let resolved = service
        .resolve_workflow_ref(&session_id, "dup")
        .expect("workflow resolves");
    super::endpoint_instances::register_primary_instance(
        &mut service,
        &session_id,
        &resolved,
        &workflow.endpoint,
        "instance-1",
    );
    service
        .enqueue_workflow_prompt(
            &session_id,
            workflow.workflow.id(),
            workflow.endpoint.id(),
            None,
            Some("default"),
            WorkflowQueuedPromptSource::Manual,
            None,
        )
        .expect("prompt queues");
    service
        .set_workflow_archived(&session_id, "dup", true)
        .expect("workflow should archive");
    assert!(service
        .dequeue_next_workflow_prompt_and_create_run(&session_id)
        .expect("queue stays readable")
        .is_none());
    service
        .set_workflow_archived(&session_id, "dup", false)
        .expect("workflow should unarchive");
    assert!(service
        .dequeue_next_workflow_prompt_and_create_run(&session_id)
        .expect("queue stays readable")
        .is_some());
}

#[test]
fn an_archived_workflows_idle_instance_does_not_hide_another_workflows_queued_prompt() {
    let (mut service, session_id, a) = archived_service();
    let resolved = service
        .resolve_workflow_ref(&session_id, "dup")
        .expect("workflow resolves");
    super::endpoint_instances::register_primary_instance(
        &mut service,
        &session_id,
        &resolved,
        &a.endpoint,
        "instance-a",
    );
    super::endpoint_instances::enqueue(
        &mut service,
        &session_id,
        a.workflow.id(),
        a.endpoint.id(),
        "queued for the archived workflow",
    );
    service
        .set_workflow_archived(&session_id, "dup", true)
        .expect("workflow should archive");
    // B has no instance yet: its prompt needs one provisioned.
    let b = workflow_with_endpoint(&mut service, &session_id, "other", "agent-1");
    super::endpoint_instances::enqueue(
        &mut service,
        &session_id,
        b.workflow.id(),
        b.endpoint.id(),
        "queued for the live workflow",
    );

    let candidate = service
        .workflow_runtime_instance_provision_candidate(&session_id)
        .expect("candidate lookup should succeed")
        .expect("the live workflow's prompt gets an instance, whatever A has idle");
    assert_eq!(candidate.workflow.id(), b.workflow.id());
    // The archived workflow's prompt waits where it was.
    let session = service.get_session(&session_id).expect("session exists");
    assert!(session.workflow_queued_prompts().iter().any(|item| {
        item.workflow_id() == a.workflow.id() && item.status() == WorkflowQueuedPromptStatus::Queued
    }));
}

#[test]
fn deleting_a_workflow_with_a_run_in_flight_names_the_run_and_changes_nothing() {
    let (mut service, session_id, workflow) = archived_service();
    let run = service
        .invoke_workflow_endpoint(
            &session_id,
            workflow.workflow.id(),
            workflow.endpoint.id(),
            None,
        )
        .expect("workflow runs");
    let remove = || crate::local::WorkflowDesignOp::WorkflowRemove {
        workflow_id: workflow.workflow.id().to_string(),
    };
    let error = service
        .apply_workflow_design_op_with_authority(&session_id, remove(), "user".into(), None, None)
        .expect_err("delete must refuse while a run is in flight");
    assert!(error.to_string().contains(run.id()), "{error}");
    assert_eq!(
        service
            .list_workflows(&session_id)
            .expect("list workflows")
            .len(),
        1
    );
}
