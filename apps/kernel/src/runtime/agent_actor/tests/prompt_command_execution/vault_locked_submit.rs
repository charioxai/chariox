//! A prompt that needs the locked Vault never holds its submit open for a person: it is
//! accepted into the Room's queue at once and waits there on the Room's one unlock card,
//! for as long as the person needs; the unlock releases it to its provider.
use super::*;

const PASSPHRASE: &str = "submit passphrase";

struct LockedVaultRoom {
    _state_root: TestWorktree,
    _worktree: TestWorktree,
    _app: Arc<Mutex<DaemonApp>>,
    runtime: AgentRuntime,
    state: KernelRuntimeState,
    sessions: crate::session::SessionStateStore,
    prompts: PromptStateOwner,
    vault: std::path::PathBuf,
    session_id: String,
    agent_id: String,
    attachment_id: String,
}

/// A Room whose Project gained a secret after its agent's provider started, so the agent's next
/// prompt needs the Vault to bring the Project environment in, while the Vault is locked (as after
/// a kernel restart).
async fn room_behind_a_locked_vault(label: &str) -> LockedVaultRoom {
    let state_root = TestWorktree::new(&format!("{label}-state"));
    let vault = state_root.path().join("vault.json");
    crate::secret::create_chariox_encrypted_vault_for_test(&vault, PASSPHRASE).unwrap();
    crate::secret::lock_chariox_encrypted_vault(&vault).unwrap();
    let mut config = DaemonConfig::for_tests();
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::CharioxEncrypted;
    config.user_config.credential_vault.unlock_policy =
        crate::config::CredentialVaultUnlockPolicy::Ttl;
    config.user_config.credential_vault.path = vault.display().to_string();
    let worktree = TestWorktree::new(label);
    let environments = crate::project_environment::ProjectEnvironmentStore::new(
        &config.private_runtime_state_root(),
    );
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-vault-submit",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    launch_dev_stub_provider(&mut app, session.id(), agent.id(), "sonnet");
    let session_snapshot = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(session.id())
        .expect("session snapshot should be available");
    let session_projection = app.session_state_projection_store();
    session_projection.update(session_snapshot.clone());
    let agent_runtime_projection = app.agent_runtime_projection_store();
    agent_runtime_projection.update_session(&session_snapshot);
    let prompt_state_owner = app.prompt_state_owner();
    let sessions = app.session_state_store();
    let app = Arc::new(Mutex::new(app));
    let state = owned_runtime_state(&app).await;
    environments
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
    let runtime = AgentRuntime::new(
        state.clone(),
        ProviderRunOperationLanes::default(),
        FocusedAgentProjection::default(),
        session_projection,
        agent_runtime_projection,
        prompt_state_owner.clone(),
        crate::session::PromptIdAllocator::default(),
    );
    LockedVaultRoom {
        _state_root: state_root,
        _worktree: worktree,
        _app: app,
        runtime,
        state,
        sessions,
        prompts: prompt_state_owner,
        vault,
        session_id: session.id().to_string(),
        agent_id: agent.id().to_string(),
        attachment_id: attachment.id().to_string(),
    }
}

impl LockedVaultRoom {
    /// Submits through the SubmitPrompt request a client sends; a person is never awaited here.
    async fn submit(&self, text: &str) -> PromptSubmissionOutcome<crate::local::PromptReceipt> {
        let request = SubmitPromptRequest {
            session_id: self.session_id.clone(),
            attachment_id: self.attachment_id.clone(),
            target_agent_id: Some(self.agent_id.clone()),
            prompt: text.to_string(),
            attachments: Vec::new(),
        };
        let command = crate::runtime::command::KernelCommand::from_local_request(
            format!("vault-locked-submit-{text}"),
            None,
            None,
            &LocalDaemonRequest::SubmitPrompt(request.clone()),
        );
        let response = timeout(
            Duration::from_secs(5),
            self.runtime.dispatch_prompt_submit(&command, request),
        )
        .await
        .expect("a submit must never wait for a person to unlock the Vault")
        .expect("a submit that needs the locked Vault is accepted");
        let LocalDaemonResponse::PromptSubmitted { outcome, .. } = response else {
            panic!("unexpected response");
        };
        outcome
    }

    fn unlock_cards(&self) -> Vec<crate::session::RuntimeInteraction> {
        self.sessions
            .get_session(&self.session_id)
            .unwrap()
            .active_interactions()
            .iter()
            .filter(|card| card.id().starts_with("vault-unlock-"))
            .cloned()
            .collect()
    }

    async fn unlock_card(&self) -> crate::session::RuntimeInteraction {
        timeout(Duration::from_secs(10), async {
            loop {
                if let Some(card) = self.unlock_cards().into_iter().next() {
                    return card;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the waiting prompt raises the unlock card in its Room")
    }

    fn queue(&self) -> (Option<String>, Vec<String>) {
        let session = self.sessions.get_session(&self.session_id).unwrap();
        let (active, queued) = self.prompts.state_parts(&session, &self.agent_id);
        (
            active.map(|prompt| prompt.id().to_string()),
            queued
                .iter()
                .map(|prompt| prompt.id().to_string())
                .collect(),
        )
    }

    /// After the unlock the card closes and the prompt is still the agent's, waiting for its
    /// provider (the real start of a provider run is proven by the live drill).
    async fn released(&self, prompt_id: &str) {
        let released = timeout(Duration::from_secs(20), async {
            while !self.unlock_cards().is_empty()
                || !crate::secret::chariox_encrypted_vault_status(&self.vault)
                    .unwrap()
                    .unlocked
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        assert!(released.is_ok(), "the unlock closes the Room's card");
        let (active, queued) = self.queue();
        assert!(
            active.as_deref() == Some(prompt_id) || queued.iter().any(|id| id == prompt_id),
            "the prompt is not lost: {active:?} {queued:?}"
        );
    }

    async fn answer(&self, card: &crate::session::RuntimeInteraction, choice: &str) {
        let reply = (choice != "cancel").then_some(PASSPHRASE);
        self.state
            .resolve_terminal_runtime_interaction(
                &self.session_id,
                card.id(),
                choice,
                reply,
                Some(crate::session::DEFAULT_LOCAL_USER_ID),
            )
            .await
            .unwrap();
    }
}

fn queued_id(outcome: PromptSubmissionOutcome<crate::local::PromptReceipt>) -> String {
    match outcome {
        PromptSubmissionOutcome::Queued { prompt } => prompt.id().to_string(),
        other => panic!("a prompt that needs the locked Vault waits in the queue: {other:?}"),
    }
}

#[tokio::test]
async fn a_prompt_needing_the_locked_vault_queues_at_once_on_one_card_until_the_unlock() {
    let room = room_behind_a_locked_vault("vault-locked-submit-unlock").await;
    let first = queued_id(room.submit("first request").await);
    let card = room.unlock_card().await;
    assert_eq!(card.title(), Some("Unlock Chariox Vault"));
    assert_eq!(
        card.timeout_sec(),
        None,
        "the card waits for as long as the person needs"
    );
    assert!(!format!("{card:?}").contains(PASSPHRASE));

    // A second use while the card is open joins it: still one card in the Room.
    let second = queued_id(room.submit("second request").await);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(room.unlock_cards().len(), 1, "one unlock card per Room");
    assert_eq!(room.queue(), (None, vec![first.clone(), second.clone()]));

    room.answer(&card, "passphrase").await;
    room.released(&first).await;
    crate::secret::lock_chariox_encrypted_vault(&room.vault).unwrap();
}

#[tokio::test]
async fn an_unlock_elsewhere_releases_the_waiting_prompt_and_withdraws_its_card() {
    let room = room_behind_a_locked_vault("vault-locked-submit-elsewhere").await;
    let first = queued_id(room.submit("first request").await);
    room.unlock_card().await;
    room.state
        .unlock_credential_vault_for_owner(
            crate::session::DEFAULT_LOCAL_USER_ID,
            crate::local::UnlockCredentialVaultRequest {
                passphrase: crate::local::VaultPassphrase::new(PASSPHRASE),
                duration: crate::local::VaultUnlockDuration::UntilKernelStops,
                surface: crate::local::VaultUnlockSurface::VaultWindow,
            },
        )
        .await
        .expect("the owner unlocks in the Vault window");
    room.released(&first).await;
    crate::secret::lock_chariox_encrypted_vault(&room.vault).unwrap();
}

#[tokio::test]
async fn a_cancelled_unlock_ends_the_waiting_prompt_with_how_to_unlock() {
    let room = room_behind_a_locked_vault("vault-locked-submit-cancel").await;
    queued_id(room.submit("first request").await);
    let card = room.unlock_card().await;
    room.answer(&card, "cancel").await;
    timeout(Duration::from_secs(20), async {
        while room.queue() != (None, Vec::new()) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("a cancelled unlock ends the prompt that waited on it");
    assert!(room.unlock_cards().is_empty());
    assert!(
        !crate::secret::chariox_encrypted_vault_status(&room.vault)
            .unwrap()
            .unlocked
    );
}
