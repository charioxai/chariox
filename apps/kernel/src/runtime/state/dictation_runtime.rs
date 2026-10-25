//! Kernel dictation requests; the service owns the model, uploads and transcription.
use super::*;
use crate::local::{DictationCommand, DictationResult};

impl KernelRuntimeState {
    pub(crate) async fn dictation_request(
        &self,
        command_context: &crate::runtime::command::KernelCommand,
        owner: &str,
        command: DictationCommand,
    ) -> Result<DictationResult, DaemonError> {
        let provider = self.owned.dictation.selected_service();
        let remote = provider != crate::local::DictationProvider::Local;
        let _vault_unlock = if let DictationCommand::Transcribe { context, .. } = &command {
            if remote {
                if let Some(context) = context {
                    let session = self.owned.session_store.get_session(&context.session_id)?;
                    if session.owner_user_id() != owner {
                        return Err(DaemonError::RelayTransport {
                            operation: "dictation",
                            code: "dictation_context_invalid".into(),
                            message: "Dictation can only open a Vault prompt in your own Room."
                                .into(),
                            retryable: false,
                        });
                    }
                }
                Some(
                    self.ensure_vault_unlocked_for_command_context(
                        command_context,
                        context.as_ref().map(|context| context.session_id.as_str()),
                        context.as_ref().map(|context| context.agent_id.as_str()),
                        "dictation",
                    )
                    .await
                    .map_err(|_| DaemonError::RelayTransport {
                        operation: "dictation",
                        code: "dictation_vault_locked".into(),
                        message: "Unlock your kernel's Vault, then try dictation again.".into(),
                        retryable: false,
                    })?,
                )
            } else {
                None
            }
        } else {
            None
        };
        // Unlock can wait for human input; resolve the Vault configuration only after that wait.
        let config = self
            .owned
            .config_projection
            .snapshot()
            .user_config
            .credential_vault;
        self.owned
            .dictation
            .handle_with_provider(owner, command, Some(config), provider)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::{DictationContext, DictationProvider, DictationRequest, LocalDaemonRequest};

    #[test]
    fn dictation_service_change_during_vault_unlock_does_not_consume_the_recording() {
        let root = crate::test_support::TestWorktree::new("dictation-unlock-service");
        let path = root.path().join("vault.json");
        crate::secret::create_chariox_encrypted_vault_for_test(&path, "synthetic-passphrase")
            .unwrap();
        let mut config = crate::config::DaemonConfig::for_tests();
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.path = path.display().to_string();
        config.user_config.credential_vault.unlock_policy =
            crate::config::CredentialVaultUnlockPolicy::KernelInit;
        let harness = crate::local::test_support::LocalRouterTestHarness::with_config(config);
        let request = harness
            .fixture_session_request("dictation-unlock-service")
            .with_owner_user_id("alice");
        let (session, agent) = harness.with_app_mut(|app| {
            crate::app::KernelSessionService::new(app)
                .create_session(request)
                .unwrap()
        });
        let state = harness.runtime_state();
        harness
            .block_on_test_task(state.owned.dictation.handle(
                "alice",
                DictationCommand::SetService {
                    service: DictationProvider::Openai,
                },
            ))
            .unwrap();
        use base64::Engine as _;
        let chunk = base64::engine::general_purpose::STANDARD.encode(vec![1_u8; 16_000]);
        harness
            .block_on_test_task(state.owned.dictation.handle(
                "alice",
                DictationCommand::AppendAudio {
                    upload_id: "pending".into(),
                    offset: 0,
                    pcm_base64: chunk,
                },
            ))
            .unwrap();
        let command = DictationCommand::Transcribe {
            upload_id: "pending".into(),
            context: Some(DictationContext {
                session_id: session.id().into(),
                agent_id: agent.id().into(),
            }),
        };
        let request = LocalDaemonRequest::Dictation(DictationRequest {
            command: command.clone(),
        });
        let caller = crate::runtime::command::KernelCommand::from_local_request(
            "dictation-wait",
            None,
            None,
            &request,
        );
        let reply = harness.block_on_test_task(async {
            let pending = state.dictation_request(&caller, "alice", command);
            tokio::pin!(pending);
            let interaction = tokio::select! {
                result = &mut pending => panic!("must wait for Vault: {result:?}"),
                result = tokio::time::timeout(std::time::Duration::from_secs(2), async {
                    loop {
                        let current = state.owned.session_store.get_session(session.id()).unwrap();
                        if let Some(interaction) = current.active_interaction_for_agent(agent.id()) { break interaction.clone() }
                        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                    }
                }) => result.expect("Vault interaction"),
            };
            state.owned.dictation.handle("alice", DictationCommand::SetService { service: DictationProvider::Local }).await.unwrap();
            state.owned.resolve_runtime_interaction(session.id(), interaction.id(), "passphrase", Some("synthetic-passphrase"), Some("alice"), false).unwrap();
            pending.await
        });
        assert!(
            matches!(reply, Err(DaemonError::RelayTransport { code, .. }) if code == "dictation_service_changed")
        );
        // Appending at the previous offset proves the pending request did not consume the upload.
        harness
            .block_on_test_task(state.owned.dictation.handle(
                "alice",
                DictationCommand::AppendAudio {
                    upload_id: "pending".into(),
                    offset: 16_000,
                    pcm_base64: base64::engine::general_purpose::STANDARD.encode([1_u8; 2]),
                },
            ))
            .unwrap();
    }
}
