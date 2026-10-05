//! MP-08 / MP-11: restored Meta work must not occupy the retired task lane.
use super::*;
use crate::session::{CreateSessionRequest, MetaagentTaskStatus, WorkflowQueuedPromptSource};

#[tokio::test]
async fn retired_meta_restart_removes_real_paused_task_edit_notifications() {
    let config = DaemonConfig::for_tests();
    let mut app = DaemonApp::bootstrap(config.clone()).unwrap();
    let (session, agent) = KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("meta-edits", "meta-edits"))
        .unwrap();
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    app.agents_mut()
        .activate_agent_meta_mode(&agent_id, None)
        .unwrap();
    app.sessions_mut()
        .start_or_update_metaagent_task(&session_id, &agent_id, "Old task")
        .unwrap();
    app.sessions_mut()
        .set_metaagent_task_status(&session_id, &agent_id, MetaagentTaskStatus::Paused)
        .unwrap();
    app.prompt_state_owner
        .submit_prepared_prompt(
            &session,
            crate::session::PromptQueueItem::new(
                "old-turn",
                "terminal",
                &agent_id,
                "Old Meta turn",
                crate::session::PromptStatus::Queued,
            ),
            false,
        )
        .unwrap();
    // Preserve the paused task while projecting the prompt fixture.
    let mut session = app.sessions().get_session(&session_id).unwrap();
    app.prompt_state_owner.project_into_session(&mut session);
    app.sessions_mut().restore_session(session);
    let app = std::sync::Arc::new(tokio::sync::Mutex::new(app));
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
        std::sync::Arc::clone(&app),
        1,
    );
    let runtime = router.runtime_state();
    // Exercise the legacy update handler, including its ordinary automation
    // attachment, hidden continuation context, and forced paused-task queue.
    runtime
        .execute_metaagent_task_request(crate::local::LocalDaemonRequest::UpdateMetaagentTask(
            crate::local::UpdateMetaagentTaskRequest {
                session_id: session_id.clone(),
                metaagent_id: agent_id.clone(),
                task_markdown: Some("Updated retired task".into()),
                plan_markdown: Some("Updated retired plan".into()),
            },
        ))
        .await
        .unwrap();
    let (notification_id, user_ids) = {
        let app = app.lock().await;
        let mut session = app.sessions().get_session(&session_id).unwrap();
        let (_, queue) = app.prompt_state_owner.state_parts(&session, &agent_id);
        assert_eq!(queue.len(), 1);
        let notification = queue.front().unwrap();
        assert_eq!(notification.prompt(), "<metaagent-event/>");
        assert_eq!(
            notification.source_client_id(),
            Some(format!("metaagent:{agent_id}:task").as_str())
        );
        assert!(!crate::scheduler::runtime::is_workflow_prompt_attachment(
            notification.source_attachment_id()
        ));
        assert!(notification
            .hidden_system_context()
            .contains("Updated retired task"));
        assert!(notification
            .hidden_system_context()
            .contains("Updated retired plan"));
        let notification_id = notification.id().to_string();
        let mut user_ids = Vec::new();
        for text in ["Ordinary follow-up", "<metaagent-event/>"] {
            let outcome = app
                .prompt_state_owner
                .submit_prepared_prompt(
                    &session,
                    crate::session::PromptQueueItem::new(
                        "user-followup",
                        "terminal",
                        &agent_id,
                        text,
                        crate::session::PromptStatus::Queued,
                    )
                    .with_source_attribution("owner-terminal", agent.owner_user_id())
                    .with_hidden_system_context("Ordinary kernel-supplied context"),
                    true,
                )
                .unwrap();
            let crate::session::PromptSubmissionOutcome::Queued { prompt } = outcome else {
                panic!("user follow-up must queue");
            };
            user_ids.push(prompt.id().to_string());
        }
        app.prompt_state_owner.project_into_session(&mut session);
        app.sessions_mut().restore_session(session);
        app.save_durable_state_snapshot().unwrap();
        (notification_id, user_ids)
    };
    drop(runtime);
    drop(router);
    drop(app);
    for _ in 0..2 {
        let app = DaemonApp::bootstrap(config.clone()).unwrap();
        let session = app.sessions().get_session(&session_id).unwrap();
        assert_eq!(
            session.metaagent_task(&agent_id).unwrap().status(),
            MetaagentTaskStatus::Aborted
        );
        assert!(!app.agents().get_agent(&agent_id).unwrap().is_metaagent());
        assert!(session.active_prompt_for_agent(&agent_id).is_none());
        let queued = session.queued_prompts_for_agent(&agent_id).unwrap();
        assert_eq!(
            queued
                .iter()
                .map(|prompt| prompt.id().to_string())
                .collect::<Vec<_>>(),
            user_ids,
            "restart must remove task-notifier {notification_id} and preserve user prompts"
        );
        assert!(queued
            .iter()
            .all(|prompt| prompt.hidden_system_context() == "Ordinary kernel-supplied context"));
        let receipts = app
            .durable_state_store()
            .load_events_by_kind("session.updated")
            .unwrap();
        assert!(receipts
            .iter()
            .any(|event| event.payload["reason"] == "meta_retired"
                && event.payload["retired_prompt_ids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|id| id == &notification_id)));
        let next = app
            .prompt_state_owner
            .activate_next_queued_prompt(&session, &agent_id, None)
            .unwrap()
            .unwrap();
        assert_eq!(
            next.id(),
            user_ids[0],
            "ordinary work must advance instead of the retired task"
        );
        assert!(app.providers().list_runs().is_empty());
    }
}

#[test]
fn retired_meta_restart_aborts_legacy_tasks_and_advances_workflow_queue() {
    for status in [
        MetaagentTaskStatus::Active,
        MetaagentTaskStatus::Paused,
        MetaagentTaskStatus::Blocked,
    ] {
        let config = DaemonConfig::for_tests();
        let (session_id, agent_id, child_id, queued_id, workflow_prompt_id) = {
            let mut app = DaemonApp::bootstrap(config.clone()).unwrap();
            let (session, agent) = KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new(
                    "meta-retirement",
                    "meta-retirement",
                ))
                .unwrap();
            app.agents_mut()
                .activate_agent_meta_mode(agent.id(), None)
                .unwrap();
            app.sessions_mut()
                .start_or_update_metaagent_task(session.id(), agent.id(), "Persisted Meta task")
                .unwrap();
            app.sessions_mut()
                .set_metaagent_task_status(session.id(), agent.id(), status)
                .unwrap();
            let child = KernelSessionService::new(&mut app)
                .spawn_agent(crate::agent::CreateAgentRequest::new(
                    session.id(),
                    "dev-stub",
                ))
                .unwrap();
            app.agents_mut()
                .set_controlled_by_metaagent_id(child.id(), Some(agent.id().into()))
                .unwrap();
            let queued = app
                .sessions_mut()
                .enqueue_metaagent_task(
                    session.id(),
                    agent.id(),
                    "old-terminal",
                    "Queued Meta task",
                    Vec::new(),
                )
                .unwrap();
            let workflow = app
                .sessions_mut()
                .create_workflow(session.id(), Some("after-meta".into()))
                .unwrap();
            let node = app
                .sessions_mut()
                .add_workflow_node(session.id(), workflow.id(), child.id())
                .unwrap();
            let endpoint = app
                .sessions_mut()
                .create_workflow_endpoint(
                    session.id(),
                    workflow.id(),
                    node.id(),
                    Some("main".into()),
                )
                .unwrap();
            let prompt = app
                .sessions_mut()
                .enqueue_workflow_prompt(
                    session.id(),
                    workflow.id(),
                    endpoint.id(),
                    Some("Ordinary queued work".into()),
                    Some("default"),
                    WorkflowQueuedPromptSource::Manual,
                    None,
                )
                .unwrap();
            app.save_durable_state_snapshot().unwrap();
            (
                session.id().to_string(),
                agent.id().to_string(),
                child.id().to_string(),
                queued.id().to_string(),
                prompt.id().to_string(),
            )
        };
        let app = DaemonApp::bootstrap(config.clone()).unwrap();
        let session = app.sessions().get_session(&session_id).unwrap();
        let task = session.metaagent_task(&agent_id).unwrap();
        assert_eq!(
            task.status(),
            MetaagentTaskStatus::Aborted,
            "legacy task must settle at restart"
        );
        assert!(task.aborted_reason().unwrap().contains("/sudo"));
        assert!(
            session.queued_metaagent_tasks().is_empty(),
            "{queued_id} must not restart"
        );
        assert!(!session.has_active_metaagent_task());
        assert!(!app.agents().get_agent(&agent_id).unwrap().is_metaagent());
        assert!(app
            .agents()
            .get_agent(&child_id)
            .unwrap()
            .controlled_by_metaagent_id()
            .is_none());
        let receipts = app
            .durable_state_store()
            .load_events_by_kind("session.updated")
            .unwrap();
        assert!(receipts
            .iter()
            .any(|event| event.payload["reason"] == "meta_retired"
                && event.payload["retired_task_ids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|id| id == &queued_id)));
        let revision = task.revision();
        drop(app);
        // A second boot must retain the settlement without re-aborting the task.
        let app = DaemonApp::bootstrap(config).unwrap();
        let session = app.sessions().get_session(&session_id).unwrap();
        assert_eq!(
            session.metaagent_task(&agent_id).unwrap().revision(),
            revision
        );
        assert!(session.queued_metaagent_tasks().is_empty());
        let next = app
            .sessions_mut()
            .dequeue_next_workflow_prompt(&session_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            next.id(),
            workflow_prompt_id,
            "ordinary workflow queue must advance"
        );
        assert!(
            app.providers().list_runs().is_empty(),
            "retirement must not launch an agent"
        );
    }
}

#[test]
fn retired_meta_restart_cancels_legacy_turn_but_preserves_ordinary_work() {
    let config = DaemonConfig::for_tests();
    let (session_id, meta_id, child_id, ordinary_ids, child_prompt_id) = {
        let mut app = DaemonApp::bootstrap(config.clone()).unwrap();
        let (mut session, agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("meta-prompts", "meta-prompts"))
            .unwrap();
        app.agents_mut()
            .activate_agent_meta_mode(agent.id(), None)
            .unwrap();
        let child = KernelSessionService::new(&mut app)
            .spawn_agent(crate::agent::CreateAgentRequest::new(
                session.id(),
                "dev-stub",
            ))
            .unwrap();
        app.agents_mut()
            .set_controlled_by_metaagent_id(child.id(), Some(agent.id().into()))
            .unwrap();
        let mut ordinary_ids = Vec::new();
        let mut child_prompt_id = String::new();
        for (id, source, target, text, force_queue) in [
            (
                "legacy-turn",
                "terminal",
                agent.id(),
                "old Meta task",
                false,
            ),
            (
                "legacy-event",
                "workflow-run:metaagent-task-orphaned",
                agent.id(),
                "<metaagent-event/>",
                true,
            ),
            (
                "ordinary-followup",
                "terminal",
                agent.id(),
                "do ordinary work",
                true,
            ),
            (
                "literal-followup",
                "terminal",
                agent.id(),
                "<metaagent-event/>",
                true,
            ),
            (
                "controlled-turn",
                "terminal",
                child.id(),
                "ongoing ordinary work",
                false,
            ),
        ] {
            let outcome = app
                .prompt_state_owner
                .submit_prepared_prompt(
                    &session,
                    crate::session::PromptQueueItem::new(
                        id,
                        source,
                        target,
                        text,
                        crate::session::PromptStatus::Queued,
                    ),
                    force_queue,
                )
                .unwrap();
            let prompt = match outcome {
                crate::session::PromptSubmissionOutcome::Started { prompt }
                | crate::session::PromptSubmissionOutcome::Queued { prompt } => prompt,
            };
            if source == "terminal" && force_queue {
                ordinary_ids.push(prompt.id().to_string());
            }
            if target == child.id() {
                child_prompt_id = prompt.id().to_string();
            }
        }
        app.prompt_state_owner.project_into_session(&mut session);
        session.set_agents(app.agents.get_session_agents(session.id()));
        app.sessions_mut().restore_session(session.clone());
        app.save_durable_state_snapshot().unwrap();
        (
            session.id().to_string(),
            agent.id().to_string(),
            child.id().to_string(),
            ordinary_ids,
            child_prompt_id,
        )
    };
    for _ in 0..2 {
        let app = DaemonApp::bootstrap(config.clone()).unwrap();
        let session = app.sessions().get_session(&session_id).unwrap();
        assert!(session.active_prompt_for_agent(&meta_id).is_none());
        assert_eq!(
            session
                .queued_prompts_for_agent(&meta_id)
                .unwrap()
                .iter()
                .map(|p| p.id().to_string())
                .collect::<Vec<_>>(),
            ordinary_ids
        );
        assert_eq!(
            session.active_prompt_for_agent(&child_id).unwrap().id(),
            child_prompt_id
        );
        assert!(!app.agents().get_agent(&meta_id).unwrap().is_metaagent());
        assert!(app
            .agents()
            .get_agent(&child_id)
            .unwrap()
            .controlled_by_metaagent_id()
            .is_none());
    }
}

#[test]
fn retired_meta_restart_preserves_completed_and_aborted_task_history() {
    for status in [MetaagentTaskStatus::Completed, MetaagentTaskStatus::Aborted] {
        let config = DaemonConfig::for_tests();
        let (session_id, agent_id, original) = {
            let mut app = DaemonApp::bootstrap(config.clone()).unwrap();
            let (session, agent) = KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new("meta-history", "meta-history"))
                .unwrap();
            app.agents_mut()
                .activate_agent_meta_mode(agent.id(), None)
                .unwrap();
            app.sessions_mut()
                .start_or_update_metaagent_task(session.id(), agent.id(), "Old task")
                .unwrap();
            app.sessions_mut()
                .set_metaagent_task_status(session.id(), agent.id(), status)
                .unwrap();
            let task = app
                .sessions()
                .get_session(session.id())
                .unwrap()
                .metaagent_task(agent.id())
                .unwrap()
                .clone();
            app.save_durable_state_snapshot().unwrap();
            (session.id().to_string(), agent.id().to_string(), task)
        };
        let app = DaemonApp::bootstrap(config).unwrap();
        let session = app.sessions().get_session(&session_id).unwrap();
        assert_eq!(session.metaagent_task(&agent_id).unwrap(), &original);
        assert!(!app.agents().get_agent(&agent_id).unwrap().is_metaagent());
    }
}

#[test]
fn retired_meta_restart_fails_closed_on_checkpoint_error_then_retries() {
    let config = DaemonConfig::for_tests();
    let (session_id, agent_id) = {
        let mut app = DaemonApp::bootstrap(config.clone()).unwrap();
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "meta-checkpoint",
                "meta-checkpoint",
            ))
            .unwrap();
        app.agents_mut()
            .activate_agent_meta_mode(agent.id(), None)
            .unwrap();
        app.sessions_mut()
            .start_or_update_metaagent_task(session.id(), agent.id(), "Old task")
            .unwrap();
        app.save_durable_state_snapshot().unwrap();
        (session.id().to_string(), agent.id().to_string())
    };
    let connection = rusqlite::Connection::open(config.durable_state_path()).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER fail_meta_checkpoint BEFORE INSERT ON durable_state_snapshots
        BEGIN SELECT RAISE(FAIL, 'injected meta retirement checkpoint failure'); END;",
        )
        .unwrap();
    let error = match DaemonApp::bootstrap(config.clone()) {
        Ok(_) => panic!("failed retirement checkpoint must prevent kernel admission"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("injected meta retirement checkpoint failure"),
        "{error}"
    );
    connection
        .execute_batch("DROP TRIGGER fail_meta_checkpoint;")
        .unwrap();
    drop(connection);
    let app = DaemonApp::bootstrap(config).unwrap();
    let session = app.sessions().get_session(&session_id).unwrap();
    assert_eq!(
        session.metaagent_task(&agent_id).unwrap().status(),
        MetaagentTaskStatus::Aborted
    );
    assert!(!app.agents().get_agent(&agent_id).unwrap().is_metaagent());
}
