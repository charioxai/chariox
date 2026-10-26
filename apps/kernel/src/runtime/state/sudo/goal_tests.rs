//! MP-08 / MP-09 / MP-10 / MP-11 A12/G02: owner controls keep existing work.
use super::tests::{fixture_with_options, popup, Fixture};
use super::*;
use crate::durable_state::agent_lifecycle::{ExecutionState, Operation, Outcome};

pub(super) fn blocked_task(
    f: &Fixture,
    goal: bool,
) -> crate::durable_state::agent_lifecycle::AgentTaskExecution {
    let store = &f.state.owned.durable_state_store;
    let now = crate::session::unix_epoch_ms();
    store
        .agent_lifecycle(Operation::Begin {
            owner: "local".into(),
            room: f.request.session_id.clone(),
            agent: f.request.target_agent_id.clone().unwrap(),
            prompt: "existing-work".into(),
            run: Some(f.run.id().into()),
            now,
        })
        .unwrap();
    if goal {
        store
            .agent_lifecycle(Operation::StartGoal {
                task: "existing-work".into(),
                owner: "local".into(),
                objective: "List session count".into(),
                minutes: 10,
                now,
            })
            .unwrap();
    }
    let Outcome::Task(task) = store
        .agent_lifecycle(Operation::Block {
            task: "existing-work".into(),
            prompt: "existing-work".into(),
            reason: "Needs owner authority".into(),
        })
        .unwrap()
    else {
        panic!()
    };
    task
}

#[tokio::test]
async fn sudo_goal_resume_refuses_unsettled_goal_before_passkey() {
    let f = fixture_with_options(None, true);
    let task = blocked_task(&f, true);
    let session = f
        .state
        .owned
        .session_store
        .get_session(&f.request.session_id)
        .unwrap();
    f.state
        .owned
        .prompt_state_owner
        .sync_external_active_prompt(
            &session,
            &task.agent_id,
            Some(PromptQueueItem::new(
                &task.prompt_id,
                &f.request.attachment_id,
                &task.agent_id,
                "Blocked goal still finishing",
                PromptStatus::Running,
            )),
        );
    let mut request = f.request.clone();
    request.prompt = format!(
        "/sudo /goal resume {} {}",
        task.goal.as_ref().unwrap().id,
        task.revision
    );
    let error = tokio::time::timeout(
        Duration::from_secs(1),
        f.state
            .submit_sudo_prompt(request, "local", "sudo-terminal"),
    )
    .await
    .expect("unsettled goal must be refused before requesting a passkey")
    .unwrap_err();
    assert!(error.to_string().contains("settle"), "{error}");
    assert!(f.state.passkey_prompts_for("local").is_empty());
    assert!(f.state.list_sudo_turns("local").is_empty());
}

#[tokio::test]
async fn sudo_goal_resume_reauthorizes_existing_goal() {
    reauthorize_existing_goal(false).await;
}

#[tokio::test]
async fn sudo_goal_resume_never_starts_ordinary_work_when_busy_turn_settles() {
    reauthorize_existing_goal(true).await;
}

async fn reauthorize_existing_goal(settle_after_busy: bool) {
    let f = fixture_with_options(None, true);
    let original = blocked_task(&f, true);
    let mut request = f.request.clone();
    request.prompt = "/sudo /goal resume".into();
    let state = f.state.clone();
    let pending = tokio::spawn(async move {
        state
            .submit_sudo_prompt(request, "local", "sudo-terminal")
            .await
    });
    let consent = tokio::time::timeout(Duration::from_secs(2), popup(&f.state))
        .await
        .unwrap();
    if settle_after_busy {
        let session = f
            .state
            .owned
            .session_store
            .get_session(&f.request.session_id)
            .unwrap();
        f.state
            .owned
            .prompt_state_owner
            .sync_external_active_prompt(
                &session,
                &original.agent_id,
                Some(PromptQueueItem::new(
                    "finishing-turn",
                    &f.request.attachment_id,
                    &original.agent_id,
                    "Previous work finishing",
                    PromptStatus::Running,
                )),
            );
        super::entry::SETTLE_AFTER_BUSY_FOR_TEST
            .lock()
            .unwrap()
            .insert(session.id().into());
    }
    super::window_tests::approve(&f, &consent, None)
        .await
        .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(5), pending)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let LocalDaemonResponse::AgentTaskUpdated { task, .. } = response else {
        panic!("expected goal control response")
    };
    assert_eq!(task.task_id, original.task_id);
    assert_eq!(
        task.goal.as_ref().unwrap().id,
        original.goal.as_ref().unwrap().id
    );
    let tasks = f
        .state
        .owned
        .durable_state_store
        .agent_tasks(Some(&f.request.session_id), None)
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_ne!(tasks[0].state, ExecutionState::Blocked);
    let turn = f.state.list_sudo_turns("local").pop().unwrap();
    assert_eq!(turn.task_id.as_deref(), Some(original.task_id.as_str()));
    assert_eq!(turn.provider_run_id.as_deref(), Some(f.run.id()));
    let session = f
        .state
        .owned
        .session_store
        .get_session(&f.request.session_id)
        .unwrap();
    let prompt = f
        .state
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, f.request.target_agent_id.as_deref().unwrap())
        .unwrap();
    assert!(prompt
        .hidden_system_context()
        .contains("receipt of that owner action"));
    assert!(f.state.sudo_for_provider_run(f.run.id()).is_ok());
    f.state
        .revoke_sudo(Some("local"), Some(&turn.entry_id), "test_cleanup")
        .unwrap();
}

#[tokio::test]
async fn sudo_goal_edit_and_cancel_use_owner_controls_without_passkey() {
    let f = fixture_with_options(None, true);
    let original = blocked_task(&f, true);
    for text in ["/sudo /goal edit Updated objective", "/sudo /goal cancel"] {
        let mut request = f.request.clone();
        request.prompt = text.into();
        let response = tokio::time::timeout(
            Duration::from_secs(2),
            f.state
                .submit_sudo_prompt(request, "local", "sudo-terminal"),
        )
        .await
        .unwrap()
        .unwrap();
        let LocalDaemonResponse::AgentTaskUpdated { task, .. } = response else {
            panic!()
        };
        assert_eq!(task.task_id, original.task_id);
        assert!(f.state.passkey_prompts_for("local").is_empty());
        assert!(f.state.list_sudo_turns("local").is_empty());
    }
}

#[tokio::test]
async fn sudo_goal_resume_without_goal_fails_before_passkey() {
    let f = fixture_with_options(None, true);
    let mut request = f.request.clone();
    request.prompt = "/sudo /goal resume".into();
    let error = tokio::time::timeout(
        Duration::from_secs(2),
        f.state
            .submit_sudo_prompt(request, "local", "sudo-terminal"),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert!(error.to_string().contains("no active goal"), "{error}");
    assert!(f.state.passkey_prompts_for("local").is_empty());
}

#[tokio::test]
async fn sudo_goal_resume_keeps_revision_fence_while_passkey_is_pending() {
    let f = fixture_with_options(None, true);
    let original = blocked_task(&f, true);
    let mut request = f.request.clone();
    request.prompt = "/sudo /goal resume".into();
    let state = f.state.clone();
    let pending = tokio::spawn(async move {
        state
            .submit_sudo_prompt(request, "local", "sudo-terminal")
            .await
    });
    let consent = tokio::time::timeout(Duration::from_secs(2), popup(&f.state))
        .await
        .unwrap();
    f.state
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::ControlGoal {
            task: original.task_id.clone(),
            owner: "local".into(),
            revision: original.revision,
            action: "edit".into(),
            value: Some("New owner objective".into()),
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    super::window_tests::approve(&f, &consent, None)
        .await
        .unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(5), pending)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    assert!(f.state.list_sudo_turns("local").is_empty());
    let tasks = f
        .state
        .owned
        .durable_state_store
        .agent_tasks(Some(&f.request.session_id), None)
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].goal.as_ref().unwrap().objective,
        "New owner objective"
    );
    assert!(tasks[0].pending_prompt_id.is_none());
}
