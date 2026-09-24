use super::*;

fn blocked_entry() -> (
    KernelRuntimeState,
    String,
    String,
    crate::runtime::workspace_coordinator::WorkspaceClaimGuard,
    TestRoot,
) {
    let (runtime, session, workflow, endpoint, root) = runtime_with_idle_workflow();
    let snapshot = runtime.owned.session_store.get_session(&session).unwrap();
    let blocker = runtime
        .owned
        .workspace_coordinator
        .acquire_worktree_write_claim(
            snapshot.workspace_id(),
            snapshot.worktree_id(),
            &session,
            None,
            "fixture_contention",
        )
        .unwrap();
    let (outcome, _) = runtime
        .owned
        .workflow_enqueue_prompt_and_maybe_start(
            &session,
            &workflow,
            &endpoint,
            Some("blocked entry".into()),
            None,
            None,
        )
        .unwrap();
    let crate::app::workflow_runtime::WorkflowLaunchOutcome::Started { workflow_run, .. } = outcome
    else {
        panic!("ready run")
    };
    assert_eq!(
        workflow_run.node_runs()[0].status(),
        crate::session::WorkflowNodeRunStatus::BlockedOnWorkspaceClaim
    );
    let run = workflow_run.id().to_owned();
    (runtime, session, run, blocker, root)
}

fn prompt_count(runtime: &KernelRuntimeState, session: &str) -> usize {
    let snapshot = runtime.owned.session_store.get_session(session).unwrap();
    runtime
        .owned
        .agent_store
        .get_session_agents(session)
        .iter()
        .map(|agent| {
            let (active, queued) = runtime
                .owned
                .prompt_state_owner
                .state_parts(&snapshot, agent.id());
            usize::from(active.is_some()) + queued.len()
        })
        .sum()
}

#[test]
fn workspace_release_submits_pending_entry_once_and_blocked_poll_does_not_rewrite() {
    let (runtime, session, run, blocker, _root) = blocked_entry();
    let db = rusqlite::Connection::open(
        runtime
            .owned
            .config_projection
            .snapshot()
            .durable_state_path(),
    )
    .unwrap();
    let before: i64 = db
        .query_row("SELECT max(sequence) FROM durable_state_events", [], |r| {
            r.get(0)
        })
        .unwrap();
    runtime
        .owned
        .workflow_start_next_queued_prompt_for_response(&session)
        .unwrap();
    assert_eq!(
        db.query_row("SELECT max(sequence) FROM durable_state_events", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap(),
        before
    );
    assert_eq!(prompt_count(&runtime, &session), 0);
    drop(blocker);
    let _dispatches = runtime.owned.workflow_retry_blocked_claims();
    assert!(
        runtime
            .owned
            .workflow_entry_intent(&session, &run)
            .unwrap()
            .unwrap()
            .submitted
    );
    assert_eq!(prompt_count(&runtime, &session), 1);
    runtime
        .owned
        .workflow_start_next_queued_prompt_for_response(&session)
        .unwrap();
    assert_eq!(prompt_count(&runtime, &session), 1);
}

#[test]
fn user_resume_of_unsubmitted_entry_keeps_one_original_operation() {
    let (runtime, session, run, blocker, _root) = blocked_entry();
    let original = runtime
        .owned
        .workflow_entry_intent(&session, &run)
        .unwrap()
        .unwrap();
    // User pause keeps a resumable run. Cancel is terminal and persistence
    // correctly archives it before this resume path can operate on it.
    runtime
        .owned
        .session_store
        .write()
        .pause_workflow_run(&session, &run)
        .unwrap();
    runtime
        .owned
        .persist_workflow_runtime_session(&session, "fixture_paused")
        .unwrap();
    drop(blocker);
    runtime.owned.workflow_resume_run(&session, &run).unwrap();
    let submitted = runtime
        .owned
        .workflow_entry_intent(&session, &run)
        .unwrap()
        .unwrap();
    assert!(submitted.submitted);
    assert_eq!(submitted.operation_id, original.operation_id);
    assert_eq!(prompt_count(&runtime, &session), 1);
    runtime
        .owned
        .workflow_start_next_queued_prompt_for_response(&session)
        .unwrap();
    assert_eq!(prompt_count(&runtime, &session), 1);
}

#[test]
fn actual_legacy_queue_and_retry_fallback_defer_owned_entries_without_failure() {
    let (runtime, session, run, blocker, _root) = blocked_entry();
    let before = runtime
        .owned
        .workflow_entry_intent(&session, &run)
        .unwrap()
        .unwrap();
    {
        let mut app = runtime.app.blocking_lock();
        assert!(app
            .start_next_queued_workflow_prompt(&session)
            .unwrap()
            .is_none());
        let workflow = app
            .sessions()
            .resolve_workflow_run_ref(&session, &run)
            .unwrap();
        let result = app
            .enqueue_workflow_prompt_and_maybe_start(
                &session,
                workflow.workflow_id(),
                workflow.endpoint_id(),
                Some("legacy enqueue still accepted".into()),
                None,
                None,
            )
            .unwrap();
        assert!(matches!(
            result,
            crate::app::workflow_runtime::WorkflowLaunchOutcome::Enqueued { .. }
        ));
        crate::scheduler::runtime::retry_blocked_workflow_claims(&mut app);
        crate::scheduler::runtime::schedule_workflow_run_entry_node(&mut app, &session, &workflow)
            .unwrap();
        assert_eq!(
            app.sessions()
                .resolve_workflow_run_ref(&session, &run)
                .unwrap()
                .node_runs()[0]
                .status(),
            crate::session::WorkflowNodeRunStatus::BlockedOnWorkspaceClaim
        );
    }
    assert_eq!(prompt_count(&runtime, &session), 0);
    assert!(
        !runtime
            .owned
            .workflow_entry_intent(&session, &run)
            .unwrap()
            .unwrap()
            .submitted
    );
    drop(blocker);
    let _dispatches = runtime.owned.workflow_retry_blocked_claims();
    assert_eq!(prompt_count(&runtime, &session), 1);
    assert_eq!(
        runtime
            .owned
            .workflow_entry_intent(&session, &run)
            .unwrap()
            .unwrap()
            .operation_id,
        before.operation_id
    );
}

#[test]
fn legacy_dequeue_cannot_claim_app_event_before_owned_ready_intent_exists() {
    let (runtime, session, workflow, endpoint, _root) = runtime_with_idle_workflow();
    let mut app = runtime.app.blocking_lock();
    let invocation = crate::session::WorkflowPublicationInvocationEnvelope {
        publication_id: "fixture-publication".into(),
        hook_id: None,
        invocation_id: "fixture-receipt".into(),
        transport: "app_event".into(),
        endpoint_id: endpoint.clone(),
        queue_ref: None,
        input: serde_json::json!({"prompt":"defer App event"}),
        artifacts: vec![],
        mode: None,
        caller: serde_json::json!({"type":"app"}),
    };
    let queued = app
        .sessions_mut()
        .enqueue_workflow_prompt_with_publication_invocation(
            &session,
            &workflow,
            &endpoint,
            Some("defer App event".into()),
            None,
            crate::session::WorkflowQueuedPromptSource::Event,
            None,
            Some(invocation),
        )
        .unwrap();
    assert!(app
        .start_next_queued_workflow_prompt(&session)
        .unwrap()
        .is_none());
    let snapshot = app.sessions().get_session(&session).unwrap();
    assert!(snapshot.workflow_runs().is_empty());
    assert_eq!(
        snapshot.workflow_queued_prompts().front().unwrap().id(),
        queued.id()
    );
}

#[test]
fn entry_preparation_failure_retries_the_same_durable_ready_run() {
    let (runtime, session, workflow, endpoint, _root) = runtime_with_idle_workflow();
    let db = rusqlite::Connection::open(
        runtime
            .owned
            .config_projection
            .snapshot()
            .durable_state_path(),
    )
    .unwrap();
    db.execute_batch("CREATE TRIGGER reject_entry_prepare BEFORE INSERT ON durable_state_events WHEN NEW.kind='workflow.runtime.updated' AND json_extract(NEW.payload_json,'$.reason')='workflow_entry_prepared' BEGIN SELECT RAISE(ABORT,'prepare fault'); END;").unwrap();
    assert!(runtime
        .owned
        .workflow_enqueue_prompt_and_maybe_start(
            &session,
            &workflow,
            &endpoint,
            Some("retry exact entry".into()),
            None,
            None
        )
        .is_err());
    let before = runtime
        .owned
        .workflow_pending_entry(&session)
        .unwrap()
        .unwrap();
    assert!(!before.submitted);
    assert!(runtime
        .owned
        .durable_state_store
        .require_writer_healthy()
        .is_ok());
    assert_eq!(
        runtime
            .owned
            .session_store
            .get_session(&session)
            .unwrap()
            .workflow_runs()
            .len(),
        1
    );
    assert!(runtime
        .owned
        .session_store
        .get_session(&session)
        .unwrap()
        .workflow_queued_prompts()
        .is_empty());
    db.execute_batch("DROP TRIGGER reject_entry_prepare")
        .unwrap();
    let (outcome, _dispatches) = runtime
        .owned
        .workflow_start_next_queued_prompt_for_response(&session)
        .unwrap();
    let Some(crate::app::workflow_runtime::WorkflowLaunchOutcome::Started { workflow_run, .. }) =
        outcome
    else {
        panic!("same entry should start")
    };
    assert_eq!(workflow_run.id(), before.run_id);
    assert!(
        runtime
            .owned
            .workflow_entry_intent(&session, &before.run_id)
            .unwrap()
            .unwrap()
            .submitted
    );
    assert_eq!(
        runtime
            .owned
            .session_store
            .get_session(&session)
            .unwrap()
            .workflow_runs()
            .len(),
        1
    );
}

#[test]
fn failure_after_prompt_admission_fences_writes_and_retains_existing_prompt_recovery() {
    let (runtime, session, workflow, endpoint, _root) = runtime_with_idle_workflow();
    let db = rusqlite::Connection::open(
        runtime
            .owned
            .config_projection
            .snapshot()
            .durable_state_path(),
    )
    .unwrap();
    db.execute_batch("CREATE TRIGGER reject_scheduled BEFORE INSERT ON durable_state_events WHEN NEW.kind='workflow.runtime.updated' AND json_extract(NEW.payload_json,'$.reason')='workflow_entry_scheduled' BEGIN SELECT RAISE(ABORT,'schedule fault'); END;").unwrap();
    assert!(runtime
        .owned
        .workflow_enqueue_prompt_and_maybe_start(
            &session,
            &workflow,
            &endpoint,
            Some("accepted before acknowledgement".into()),
            None,
            None
        )
        .is_err());
    assert!(runtime
        .owned
        .durable_state_store
        .require_writer_healthy()
        .is_err());
    let (operation,run,prompt):(String,String,String)=db.query_row("SELECT operation_id,run_id,prompt_id FROM durable_workflow_dispatch_intents WHERE submitted=1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    let payload:String=db.query_row("SELECT payload_json FROM durable_state_events WHERE kind='session.prompt_state.updated' ORDER BY sequence DESC LIMIT 1",[],|r|r.get(0)).unwrap();
    let mut event: crate::durable_prompt_state::DurablePromptStateEventPayload =
        serde_json::from_str(&payload).unwrap();
    event.restore_private_states();
    let durable = event
        .active_prompt
        .iter()
        .chain(event.queued_prompts.iter())
        .find(|p| p.id() == prompt)
        .unwrap();
    assert_eq!(durable.durable_operation_id(), Some(operation.as_str()));
    assert_eq!(durable.workflow_run_id(), Some(run.as_str()));
    // A new ordinary persistence of the half-scheduled live projection cannot
    // turn it into another durable transition after the fence completes.
    let before: i64 = db
        .query_row("SELECT max(sequence) FROM durable_state_events", [], |r| {
            r.get(0)
        })
        .unwrap();
    let snapshot = runtime.owned.session_store.get_session(&session).unwrap();
    assert!(runtime
        .owned
        .durable_state_store
        .persist_workflow_runtime_transition(&snapshot, "must_not_commit")
        .is_err());
    assert!(runtime.owned.session_snapshot(&session).is_err());
    assert_eq!(
        db.query_row("SELECT max(sequence) FROM durable_state_events", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap(),
        before
    );
}
