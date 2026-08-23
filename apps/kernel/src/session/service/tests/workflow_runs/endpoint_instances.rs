use std::collections::BTreeMap;

use super::*;

fn register_primary_instance(
    service: &mut SessionService,
    session_id: &str,
    workflow: &crate::session::WorkflowDefinition,
    endpoint: &crate::session::WorkflowEndpointDefinition,
    instance_id: &str,
) {
    let node_agent_ids = workflow
        .nodes()
        .iter()
        .map(|node| (node.id().to_string(), node.agent_id().to_string()))
        .collect::<BTreeMap<_, _>>();
    service
        .register_workflow_runtime_instance(
            session_id,
            crate::session::WorkflowEndpointRuntimeInstance::new(
                instance_id,
                workflow.id(),
                endpoint.id(),
                workflow.revision(),
                1,
                true,
                node_agent_ids,
                "worktree-1",
            ),
        )
        .expect("primary instance should register");
}

fn enqueue(
    service: &mut SessionService,
    session_id: &str,
    workflow_id: &str,
    endpoint_id: &str,
    prompt: &str,
) -> crate::session::WorkflowQueuedPrompt {
    service
        .enqueue_workflow_prompt(
            session_id,
            workflow_id,
            endpoint_id,
            Some(prompt.to_string()),
            None,
            WorkflowQueuedPromptSource::Manual,
            None,
        )
        .expect("workflow prompt should enqueue")
}

#[test]
fn endpoint_instance_stays_exclusive_and_reuses_after_terminal_run() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let TestWorkflowEndpoint { workflow, endpoint } =
        workflow_with_endpoint(&mut service, session.id(), "review", "agent-1");
    let workflow = service
        .resolve_workflow_ref(session.id(), workflow.id())
        .expect("workflow should resolve");
    register_primary_instance(
        &mut service,
        session.id(),
        &workflow,
        &endpoint,
        "instance-1",
    );

    let first_item = enqueue(
        &mut service,
        session.id(),
        workflow.id(),
        endpoint.id(),
        "first",
    );
    let second_item = enqueue(
        &mut service,
        session.id(),
        workflow.id(),
        endpoint.id(),
        "second",
    );

    let (claimed_first, first_run, _, _) = service
        .dequeue_next_workflow_prompt_and_create_run(session.id())
        .expect("queue should advance")
        .expect("first item should start");
    assert_eq!(claimed_first.id(), first_item.id());
    assert_eq!(first_run.runtime_instance_id(), Some("instance-1"));
    assert_eq!(first_run.queue_item_id(), Some(first_item.id()));
    assert_eq!(
        first_run.invocation_source(),
        WorkflowQueuedPromptSource::Manual
    );
    assert!(service
        .dequeue_next_workflow_prompt_and_create_run(session.id())
        .expect("queue check should succeed")
        .is_none());

    service
        .cancel_workflow_run(session.id(), first_run.id())
        .expect("first run should cancel");
    service
        .store
        .get_mut(session.id())
        .expect("session should exist")
        .reconcile_workflow_runtime_instances();

    let (claimed_second, second_run, _, _) = service
        .dequeue_next_workflow_prompt_and_create_run(session.id())
        .expect("queue should advance after release")
        .expect("second item should start");
    assert_eq!(claimed_second.id(), second_item.id());
    assert_eq!(second_run.runtime_instance_id(), Some("instance-1"));
}

#[test]
fn saturated_endpoint_does_not_block_an_unrelated_endpoint() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1", "agent-2"]);
    let first = workflow_with_endpoint(&mut service, session.id(), "first", "agent-1");
    let second = workflow_with_endpoint(&mut service, session.id(), "second", "agent-2");
    let first_workflow = service
        .resolve_workflow_ref(session.id(), first.workflow.id())
        .expect("first workflow should resolve");
    let second_workflow = service
        .resolve_workflow_ref(session.id(), second.workflow.id())
        .expect("second workflow should resolve");
    register_primary_instance(
        &mut service,
        session.id(),
        &first_workflow,
        &first.endpoint,
        "first-instance",
    );
    register_primary_instance(
        &mut service,
        session.id(),
        &second_workflow,
        &second.endpoint,
        "second-instance",
    );

    enqueue(
        &mut service,
        session.id(),
        first_workflow.id(),
        first.endpoint.id(),
        "occupy first",
    );
    service
        .dequeue_next_workflow_prompt_and_create_run(session.id())
        .expect("first queue should advance")
        .expect("first endpoint should start");
    let blocked = enqueue(
        &mut service,
        session.id(),
        first_workflow.id(),
        first.endpoint.id(),
        "blocked first",
    );
    let available = enqueue(
        &mut service,
        session.id(),
        second_workflow.id(),
        second.endpoint.id(),
        "available second",
    );

    let (claimed, _, workflow, _) = service
        .dequeue_next_workflow_prompt_and_create_run(session.id())
        .expect("queue should advance")
        .expect("unrelated endpoint should start");
    assert_eq!(claimed.id(), available.id());
    assert_eq!(workflow.id(), second_workflow.id());
    assert!(service
        .get_session(session.id())
        .expect("session should exist")
        .workflow_queued_prompts()
        .iter()
        .any(|item| item.id() == blocked.id()));
}

fn register_clone_instance(
    service: &mut SessionService,
    session_id: &str,
    workflow: &crate::session::WorkflowDefinition,
    endpoint: &crate::session::WorkflowEndpointDefinition,
    instance_id: &str,
    worktree_path: &str,
) {
    let node_agent_ids = workflow
        .nodes()
        .iter()
        .map(|node| (node.id().to_string(), node.agent_id().to_string()))
        .collect::<BTreeMap<_, _>>();
    service
        .register_workflow_runtime_instance(
            session_id,
            crate::session::WorkflowEndpointRuntimeInstance::new(
                instance_id,
                workflow.id(),
                endpoint.id(),
                workflow.revision(),
                2,
                false,
                node_agent_ids,
                worktree_path,
            ),
        )
        .expect("clone instance should register");
}

fn plain_temp_directory(label: &str) -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "chariox-{label}-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&directory).expect("temporary directory should exist");
    directory
}

#[test]
fn missing_clone_worktree_never_dispatches_as_healthy_idle_and_sweeps_clean() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let TestWorkflowEndpoint { workflow, endpoint } =
        workflow_with_endpoint(&mut service, session.id(), "review", "agent-1");
    {
        let session = service
            .store
            .get_mut(session.id())
            .expect("session should exist");
        let workflow_mut = session
            .workflow_mut(workflow.id())
            .expect("workflow should exist");
        workflow_mut
            .endpoint_mut(endpoint.id())
            .expect("endpoint should exist")
            .set_max_instances(2);
    }
    let workflow = service
        .resolve_workflow_ref(session.id(), workflow.id())
        .expect("workflow should resolve");
    let endpoint = workflow.endpoint(endpoint.id()).expect("endpoint").clone();
    register_primary_instance(
        &mut service,
        session.id(),
        &workflow,
        &endpoint,
        "instance-primary",
    );
    let clone_worktree = plain_temp_directory("clone-worktree");
    register_clone_instance(
        &mut service,
        session.id(),
        &workflow,
        &endpoint,
        "instance-clone",
        &clone_worktree.display().to_string(),
    );

    // A live direct clone stays healthy idle and needs no cleanup.
    let status_of = |service: &SessionService, instance_id: &str| {
        service
            .get_session(session.id())
            .expect("session should exist")
            .workflow_runtime_instance(instance_id)
            .expect("instance should exist")
            .status()
    };
    assert_eq!(
        status_of(&service, "instance-clone"),
        crate::session::WorkflowEndpointRuntimeInstanceStatus::Idle
    );
    assert_eq!(
        service
            .cleanup_ready_workflow_runtime_instances(session.id())
            .expect("cleanup should succeed")
            .len(),
        0
    );

    // Once the clone directory vanishes, the instance must go stale and be
    // returned by the cleanup sweep instead of dispatching as healthy idle.
    std::fs::remove_dir(&clone_worktree).expect("clone directory should be removable");
    let ready = service
        .cleanup_ready_workflow_runtime_instances(session.id())
        .expect("cleanup should succeed");
    assert_eq!(
        ready
            .iter()
            .map(|instance| instance.id())
            .collect::<Vec<_>>(),
        vec!["instance-clone"]
    );
    assert_eq!(
        status_of(&service, "instance-clone"),
        crate::session::WorkflowEndpointRuntimeInstanceStatus::Stale
    );
    assert_eq!(
        status_of(&service, "instance-primary"),
        crate::session::WorkflowEndpointRuntimeInstanceStatus::Idle
    );
    std::fs::remove_dir(&clone_worktree).ok();

    // Dispatch provisioning must plan a fresh clone rather than reuse the ghost.
    // Occupy the primary first so the queued prompt needs a second instance.
    enqueue(
        &mut service,
        session.id(),
        workflow.id(),
        endpoint.id(),
        "occupy primary",
    );
    service
        .dequeue_next_workflow_prompt_and_create_run(session.id())
        .expect("queue should advance")
        .expect("primary should start");
    enqueue(
        &mut service,
        session.id(),
        workflow.id(),
        endpoint.id(),
        "after ghost",
    );
    let candidate = service
        .workflow_runtime_instance_provision_candidate(session.id())
        .expect("candidate lookup should succeed")
        .expect("a replacement clone should be provisionable");
    assert!(!candidate.primary);
}

#[test]
fn primary_session_worktree_is_never_treated_as_a_disposable_clone() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let TestWorkflowEndpoint { workflow, endpoint } =
        workflow_with_endpoint(&mut service, session.id(), "review", "agent-1");
    let workflow = service
        .resolve_workflow_ref(session.id(), workflow.id())
        .expect("workflow should resolve");
    register_primary_instance(
        &mut service,
        session.id(),
        &workflow,
        &endpoint,
        "instance-1",
    );

    let ready = service
        .cleanup_ready_workflow_runtime_instances(session.id())
        .expect("cleanup should succeed");
    assert_eq!(ready.len(), 0);
    let restored = service
        .get_session(session.id())
        .expect("session should exist");
    let instance = restored
        .workflow_runtime_instance("instance-1")
        .expect("primary instance should remain");
    assert_eq!(
        instance.status(),
        crate::session::WorkflowEndpointRuntimeInstanceStatus::Idle
    );
}

#[test]
fn provision_candidate_respects_endpoint_cap_and_ordinal() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let TestWorkflowEndpoint { workflow, endpoint } =
        workflow_with_endpoint(&mut service, session.id(), "review", "agent-1");
    {
        let session = service
            .store
            .get_mut(session.id())
            .expect("session should exist");
        let workflow = session
            .workflow_mut(workflow.id())
            .expect("workflow should exist");
        workflow
            .endpoint_mut(endpoint.id())
            .expect("endpoint should exist")
            .set_max_instances(2);
    }
    let workflow = service
        .resolve_workflow_ref(session.id(), workflow.id())
        .expect("workflow should resolve");
    let endpoint = workflow
        .endpoint(endpoint.id())
        .expect("endpoint should resolve")
        .clone();
    enqueue(
        &mut service,
        session.id(),
        workflow.id(),
        endpoint.id(),
        "first",
    );
    let primary = service
        .workflow_runtime_instance_provision_candidate(session.id())
        .expect("candidate lookup should succeed")
        .expect("primary candidate should exist");
    assert!(primary.primary);
    assert_eq!(primary.ordinal, 1);
    register_primary_instance(
        &mut service,
        session.id(),
        &workflow,
        &endpoint,
        "instance-1",
    );
    service
        .dequeue_next_workflow_prompt_and_create_run(session.id())
        .expect("queue should advance")
        .expect("primary should start");
    enqueue(
        &mut service,
        session.id(),
        workflow.id(),
        endpoint.id(),
        "second",
    );
    let clone = service
        .workflow_runtime_instance_provision_candidate(session.id())
        .expect("candidate lookup should succeed")
        .expect("clone candidate should exist");
    assert!(!clone.primary);
    assert_eq!(clone.ordinal, 2);
}

#[test]
fn workflow_removal_retains_instances_as_stale_until_external_cleanup() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let TestWorkflowEndpoint { workflow, endpoint } =
        workflow_with_endpoint(&mut service, session.id(), "review", "agent-1");
    let workflow = service
        .resolve_workflow_ref(session.id(), workflow.id())
        .expect("workflow should resolve");
    register_primary_instance(
        &mut service,
        session.id(),
        &workflow,
        &endpoint,
        "instance-1",
    );

    let session = service
        .store
        .get_mut(session.id())
        .expect("session should exist");
    session
        .remove_workflow(workflow.id())
        .expect("workflow should be removed");

    let instance = session
        .workflow_runtime_instance("instance-1")
        .expect("instance record must remain available for cleanup");
    assert_eq!(
        instance.status(),
        crate::session::WorkflowEndpointRuntimeInstanceStatus::Stale
    );
    assert_eq!(session.cleanup_ready_workflow_runtime_instances().len(), 1);
}

#[test]
fn runtime_instance_and_attribution_reconcile_after_session_restart() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let TestWorkflowEndpoint { workflow, endpoint } =
        workflow_with_endpoint(&mut service, session.id(), "review", "agent-1");
    let workflow = service
        .resolve_workflow_ref(session.id(), workflow.id())
        .expect("workflow should resolve");
    register_primary_instance(
        &mut service,
        session.id(),
        &workflow,
        &endpoint,
        "instance-1",
    );
    let queued = enqueue(
        &mut service,
        session.id(),
        workflow.id(),
        endpoint.id(),
        "restart-safe",
    );
    let (_, run, _, _) = service
        .dequeue_next_workflow_prompt_and_create_run(session.id())
        .expect("queue should advance")
        .expect("run should start");

    let encoded = serde_json::to_value(
        service
            .get_session(session.id())
            .expect("session should exist"),
    )
    .expect("session should serialize");
    let mut restored: crate::session::RuntimeSession =
        serde_json::from_value(encoded).expect("session should restore");

    let restored_run = restored
        .workflow_run(run.id())
        .expect("run should survive restart");
    assert_eq!(restored_run.runtime_instance_id(), Some("instance-1"));
    assert_eq!(restored_run.queue_item_id(), Some(queued.id()));
    assert_eq!(
        restored_run.runtime_agent_id_for_node(endpoint.entry_node_id()),
        Some("agent-1")
    );
    assert_eq!(
        restored
            .workflow_runtime_instance("instance-1")
            .expect("instance should survive restart")
            .active_run_id(),
        Some(run.id())
    );

    restored
        .workflow_run_mut(run.id())
        .expect("run should remain mutable")
        .set_status(WorkflowRunStatus::Completed);
    restored.reconcile_workflow_runtime_instances();
    assert_eq!(
        restored
            .workflow_runtime_instance("instance-1")
            .expect("instance should remain reusable")
            .status(),
        crate::session::WorkflowEndpointRuntimeInstanceStatus::Idle
    );
}

#[test]
fn restored_hot_state_sweeps_missing_clone_worktrees_after_restart() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let TestWorkflowEndpoint { workflow, endpoint } =
        workflow_with_endpoint(&mut service, session.id(), "review", "agent-1");
    let workflow = service
        .resolve_workflow_ref(session.id(), workflow.id())
        .expect("workflow should resolve");
    register_primary_instance(
        &mut service,
        session.id(),
        &workflow,
        &endpoint,
        "instance-primary",
    );
    let clone_worktree = plain_temp_directory("restart-clone-worktree");
    register_clone_instance(
        &mut service,
        session.id(),
        &workflow,
        &endpoint,
        "instance-clone",
        &clone_worktree.display().to_string(),
    );

    let encoded = serde_json::to_value(
        service
            .get_session(session.id())
            .expect("session should exist"),
    )
    .expect("session should serialize");
    std::fs::remove_dir(&clone_worktree).expect("clone directory should be removable");
    let restored: crate::session::RuntimeSession =
        serde_json::from_value(encoded).expect("session should restore");

    assert_eq!(
        restored
            .workflow_runtime_instance("instance-clone")
            .expect("clone instance should survive restart")
            .status(),
        crate::session::WorkflowEndpointRuntimeInstanceStatus::Idle
    );
    let restored_session_id = restored.id().to_string();
    let mut restarted_service = SessionService::new(&test_config());
    restarted_service.restore_session(restored);
    let ready = restarted_service
        .cleanup_ready_workflow_runtime_instances(&restored_session_id)
        .expect("restored cleanup should succeed");
    assert_eq!(
        ready
            .iter()
            .map(|instance| instance.id())
            .collect::<Vec<_>>(),
        vec!["instance-clone"]
    );
    assert_eq!(
        restarted_service
            .get_session(&restored_session_id)
            .expect("restored session should exist")
            .workflow_runtime_instance("instance-clone")
            .expect("instance record should remain for cleanup")
            .status(),
        crate::session::WorkflowEndpointRuntimeInstanceStatus::Stale
    );
    assert_eq!(
        restarted_service
            .get_session(&restored_session_id)
            .expect("restored session should exist")
            .workflow_runtime_instance("instance-primary")
            .expect("primary instance should survive restart")
            .status(),
        crate::session::WorkflowEndpointRuntimeInstanceStatus::Idle
    );
}

#[test]
fn cap_only_edit_keeps_existing_instances_on_the_new_revision() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    seed_agents(&mut service, session.id(), &["agent-1"]);
    let TestWorkflowEndpoint { workflow, endpoint } =
        workflow_with_endpoint(&mut service, session.id(), "review", "agent-1");
    let workflow = service
        .resolve_workflow_ref(session.id(), workflow.id())
        .expect("workflow should resolve");
    register_primary_instance(
        &mut service,
        session.id(),
        &workflow,
        &endpoint,
        "instance-1",
    );

    service
        .set_workflow_endpoint_max_instances(session.id(), workflow.id(), endpoint.id(), 4)
        .expect("cap should update");
    let next_revision = service
        .resolve_workflow_ref(session.id(), workflow.id())
        .expect("workflow should resolve")
        .revision();
    let session_snapshot = service
        .get_session(session.id())
        .expect("session should exist");
    let instance = session_snapshot
        .workflow_runtime_instance("instance-1")
        .expect("instance should remain");
    assert_eq!(instance.workflow_revision(), next_revision);
    assert_eq!(
        instance.status(),
        crate::session::WorkflowEndpointRuntimeInstanceStatus::Idle
    );
}
