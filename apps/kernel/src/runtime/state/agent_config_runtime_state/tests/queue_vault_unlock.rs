//! A queued prompt that needs the locked Vault waits on the Room's unlock card for as long as the
//! person needs; a cancelled unlock ends it, so the agent never sits idle behind a prompt that
//! nobody can start.
use super::*;

/// An agent whose launch needs a locked Vault: its Project holds a secret and the Vault is
/// encrypted behind a passphrase nobody gives.
async fn agent_behind_a_locked_vault(
    label: &str,
) -> (
    crate::test_support::TestWorktree,
    Arc<Mutex<DaemonApp>>,
    KernelRuntimeState,
    String,
    String,
) {
    let root = crate::test_support::TestWorktree::new(label);
    let vault = root.path().join("vault.json");
    crate::secret::create_chariox_encrypted_vault_for_test(&vault, "queue passphrase").unwrap();
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::CharioxEncrypted;
    config.user_config.credential_vault.unlock_policy =
        crate::config::CredentialVaultUnlockPolicy::Ttl;
    config.user_config.credential_vault.path = vault.display().to_string();
    let (app, runtime, session_id, agent_id) = agent_config_runtime_with_config(config).await;
    let session = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .unwrap();
    crate::project_environment::ProjectEnvironmentStore::new(
        &runtime
            .owned
            .config_projection
            .snapshot()
            .private_runtime_state_root(),
    )
    .save(
        session.project_id(),
        0,
        crate::project_environment::ProjectEnvironmentRecord {
            schema: crate::project_environment::PROJECT_ENVIRONMENT_SCHEMA,
            secrets: vec!["SYNTHETIC_TOKEN".into()],
            ..Default::default()
        },
    )
    .unwrap();
    (root, app, runtime, session_id, agent_id)
}

fn queue_prompts(
    runtime: &KernelRuntimeState,
    session_id: &str,
    agent_id: &str,
    prompts: Vec<crate::session::PromptQueueItem>,
) {
    let session = runtime.owned.session_store.get_session(session_id).unwrap();
    for prompt in prompts {
        runtime
            .owned
            .prompt_state_owner
            .submit_prepared_prompt(&session, prompt, true)
            .unwrap();
    }
    let (active, queued) = runtime
        .owned
        .prompt_state_owner
        .state_parts(&session, agent_id);
    runtime
        .owned
        .mirror_prompt_owner_agent_state(session_id, agent_id, active, queued)
        .unwrap();
}

#[tokio::test]
async fn held_provider_queues_resume_independently_of_a_vault_unlock() {
    use crate::session::{PromptQueueItem, PromptStatus, SessionAgentDefaults};

    let (_root, app, runtime, blocked_session, blocked_agent) =
        agent_behind_a_locked_vault("held-queue-vault-independence").await;
    let worktree = crate::test_support::TestWorktree::new("held-queue-without-vault");
    let (ready_session, ready_agent) =
        crate::app::KernelSessionService::new(&mut *app.lock().await)
            .create_session(
                worktree
                    .session_request()
                    .with_agent_defaults(SessionAgentDefaults::new("dev-stub")),
            )
            .unwrap();
    for (session, agent, source) in [
        (
            blocked_session.as_str(),
            blocked_agent.as_str(),
            "terminal:owner",
        ),
        (ready_session.id(), ready_agent.id(), "agent:fixture"),
    ] {
        let mut prompt = PromptQueueItem::new(
            "held",
            "attachment-1",
            agent,
            "queued turn",
            PromptStatus::Queued,
        )
        .with_source_attribution(source, crate::session::DEFAULT_LOCAL_USER_ID);
        prompt.set_launch_wait(Some("Waiting for memory on builder".into()));
        queue_prompts(&runtime, session, agent, vec![prompt]);
    }
    let state = runtime.clone();
    let scheduler = tokio::spawn(async move { state.run_provider_turn_admission().await });
    let result = tokio::time::timeout(std::time::Duration::from_secs(6), async {
        loop {
            let locked = runtime
                .owned
                .session_store
                .get_session(&blocked_session)
                .unwrap();
            let ready = runtime
                .owned
                .session_store
                .get_session(ready_session.id())
                .unwrap();
            if locked
                .active_interactions()
                .iter()
                .any(|card| card.id().starts_with("vault-unlock-"))
                && runtime
                    .owned
                    .provider_store
                    .get_run_for_agent(ready.id(), ready_agent.id())
                    .is_some()
                && runtime
                    .owned
                    .prompt_state_owner
                    .peek_next_queued_prompt(&ready, ready_agent.id())
                    .is_none()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await;
    scheduler.abort();
    result.expect(
        "the independent queue must start while the first Room is still waiting on its Vault card",
    );
}

/// An agent with two queued prompts whose launch needs a locked Vault.
async fn two_queued_prompts_behind_a_locked_vault(
    label: &str,
) -> (
    crate::test_support::TestWorktree,
    Arc<Mutex<DaemonApp>>,
    KernelRuntimeState,
    String,
    String,
) {
    let (root, app, runtime, session_id, agent_id) = agent_behind_a_locked_vault(label).await;
    let prompts = [("head-a", "first request"), ("head-b", "second request")]
        .map(|(id, text)| {
            crate::session::PromptQueueItem::new(
                id,
                "attachment-1",
                &agent_id,
                text,
                crate::session::PromptStatus::Queued,
            )
        })
        .to_vec();
    queue_prompts(&runtime, &session_id, &agent_id, prompts);
    (root, app, runtime, session_id, agent_id)
}

fn assert_queue_ended_idle(
    runtime: &KernelRuntimeState,
    session_id: &str,
    agent_id: &str,
    started: Option<crate::session::PromptQueueItem>,
) {
    let session = runtime.owned.session_store.get_session(session_id).unwrap();
    let (active, queued) = runtime
        .owned
        .prompt_state_owner
        .state_parts(&session, agent_id);
    assert!(started.is_none() && active.is_none());
    assert!(
        queued.is_empty(),
        "a refused Vault unlock left {} prompts queued behind an idle agent",
        queued.len()
    );
}

#[tokio::test(start_paused = true)]
async fn an_unanswered_vault_unlock_keeps_the_queued_prompts_waiting_on_its_card() {
    let (_root, app, runtime, session_id, agent_id) =
        two_queued_prompts_behind_a_locked_vault("queue-vault-unlock-waits").await;
    let promotion = runtime.advance_project_queued_prompt_after_settlement(&session_id, &agent_id);
    tokio::pin!(promotion);
    tokio::select! {
        started = &mut promotion => panic!("the queue must wait for the person: {started:?}"),
        () = tokio::time::sleep(std::time::Duration::from_secs(24 * 60 * 60)) => {}
    }
    let session = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .unwrap();
    let cards = session
        .active_interactions()
        .iter()
        .filter(|card| card.id().starts_with("vault-unlock-"))
        .collect::<Vec<_>>();
    assert_eq!(cards.len(), 1, "one unlock card waits in the Room");
    assert_eq!(cards[0].timeout_sec(), None);
    let (active, queued) = runtime
        .owned
        .prompt_state_owner
        .state_parts(&session, &agent_id);
    assert!(active.is_none());
    assert_eq!(
        queued
            .iter()
            .map(|prompt| prompt.prompt())
            .collect::<Vec<_>>(),
        vec!["first request", "second request"]
    );
    drop(app);
}

/// The owner cancels each unlock card as it appears, `count` times.
async fn cancel_unlock_cards(
    runtime: &KernelRuntimeState,
    session_id: &str,
    agent_id: &str,
    count: usize,
) {
    let owner = runtime
        .owned
        .session_store
        .get_session(session_id)
        .unwrap()
        .owner_user_id()
        .to_string();
    let mut cancelled = 0;
    while cancelled < count {
        let card = runtime
            .owned
            .session_store
            .get_session(session_id)
            .unwrap()
            .active_interaction_for_agent(agent_id)
            .map(|card| card.id().to_string());
        match card {
            Some(card) => {
                runtime
                    .resolve_terminal_runtime_interaction(
                        session_id,
                        &card,
                        "cancel",
                        None,
                        Some(&owner),
                    )
                    .await
                    .unwrap();
                cancelled += 1;
            }
            None => tokio::time::sleep(std::time::Duration::from_millis(1)).await,
        }
    }
}

#[tokio::test]
async fn a_cancelled_vault_unlock_ends_the_queued_prompt_and_the_queue_moves_on() {
    let (_root, app, runtime, session_id, agent_id) =
        two_queued_prompts_behind_a_locked_vault("queue-vault-unlock-cancel").await;
    // The owner cancels each unlock card as it appears.
    let cancel_cards = cancel_unlock_cards(&runtime, &session_id, &agent_id, 2);
    let (started, ()) = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        tokio::join!(
            runtime.advance_project_queued_prompt_after_settlement(&session_id, &agent_id),
            cancel_cards
        )
    })
    .await
    .expect("a cancelled unlock must end the queue, not wait");
    assert_queue_ended_idle(&runtime, &session_id, &agent_id, started);
    drop(app);
}

#[tokio::test]
async fn a_cancelled_vault_unlock_fails_the_workflow_run_whose_step_was_queued() {
    let (_root, app, runtime, session_id, agent_id) =
        agent_behind_a_locked_vault("queue-vault-unlock-workflow").await;
    let (run_id, node_run_id) = {
        let mut sessions = runtime.owned.session_store.write();
        let workflow = sessions
            .create_workflow(&session_id, Some("vault-step".to_string()))
            .unwrap();
        let node = sessions
            .add_workflow_node(&session_id, workflow.id(), &agent_id)
            .unwrap();
        let endpoint = sessions
            .create_workflow_endpoint(&session_id, workflow.id(), node.id(), Some("entry".into()))
            .unwrap();
        let run = sessions
            .invoke_workflow_endpoint(
                &session_id,
                workflow.id(),
                endpoint.id(),
                Some("needs the vault".into()),
            )
            .unwrap();
        let node_run_id = run.node_runs()[0].id().to_string();
        sessions
            .prepare_workflow_turn(
                &session_id,
                run.id(),
                &node_run_id,
                format!("workflow-ack:{node_run_id}"),
                "needs the vault".into(),
                None,
                None,
            )
            .unwrap();
        (run.id().to_string(), node_run_id)
    };
    queue_prompts(
        &runtime,
        &session_id,
        &agent_id,
        vec![crate::session::PromptQueueItem::new(
            "step-prompt",
            "attachment-1",
            &agent_id,
            "needs the vault",
            crate::session::PromptStatus::Queued,
        )
        .with_workflow_context(&run_id, &node_run_id)],
    );

    // The owner cancels the unlock card: the step's prompt ends, and the run it served with it.
    let (started, ()) = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        tokio::join!(
            runtime.advance_project_queued_prompt_after_settlement(&session_id, &agent_id),
            cancel_unlock_cards(&runtime, &session_id, &agent_id, 1)
        )
    })
    .await
    .expect("a cancelled unlock ends the step, not waits");

    assert_queue_ended_idle(&runtime, &session_id, &agent_id, started);
    let session = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .unwrap();
    let run = session
        .workflow_runs()
        .iter()
        .find(|run| run.id() == run_id)
        .expect("the run is still recorded");
    assert_eq!(
        run.status(),
        crate::session::WorkflowRunStatus::Failed,
        "a workflow step whose unlock was cancelled fails its run, not hangs"
    );
    drop(app);
}
