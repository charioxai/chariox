//! An expired or unanswered Vault unlock ends the queued prompt that asked for it;
//! it never leaves the agent idle behind a prompt that nobody can start.
use super::*;

#[tokio::test(start_paused = true)]
async fn a_timed_out_vault_unlock_ends_the_queued_prompt_and_the_queue_moves_on() {
    let root = crate::test_support::TestWorktree::new("queue-vault-unlock-timeout");
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
    for (id, text) in [("head-a", "first request"), ("head-b", "second request")] {
        runtime
            .owned
            .prompt_state_owner
            .submit_prepared_prompt(
                &session,
                crate::session::PromptQueueItem::new(
                    id,
                    "attachment-1",
                    &agent_id,
                    text,
                    crate::session::PromptStatus::Queued,
                ),
                true,
            )
            .unwrap();
    }
    let (active, queued) = runtime
        .owned
        .prompt_state_owner
        .state_parts(&session, &agent_id);
    runtime
        .owned
        .mirror_prompt_owner_agent_state(&session_id, &agent_id, active, queued)
        .unwrap();

    // Nobody answers either unlock card: each prompt asks once and times out.
    let started = runtime
        .advance_project_queued_prompt_after_settlement(&session_id, &agent_id)
        .await;

    let session = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .unwrap();
    let (active, queued) = runtime
        .owned
        .prompt_state_owner
        .state_parts(&session, &agent_id);
    assert!(started.is_none() && active.is_none());
    assert!(
        queued.is_empty(),
        "an unanswered Vault unlock left {} prompts queued behind an idle agent",
        queued.len()
    );
    drop(app);
}
