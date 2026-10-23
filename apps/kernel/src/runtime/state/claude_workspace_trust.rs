//! MP-08 / MP-10 / MP-11: the kernel trusts a Claude launch folder for the
//! person, and asks first only when the folder ships its own Claude hooks.
use super::*;
use crate::session::{
    RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionChoiceStyle,
    RuntimeInteractionKind, RuntimeInteractionLevel,
};

const HOOK_CARD_TIMEOUT_SEC: u64 = 600;
const HOOK_LINES_SHOWN: usize = 8;
const HOOK_ACTION_CHARS: usize = 160;

impl KernelRuntimeState {
    /// Runs before a Claude launch plans its process. A folder that ships hooks
    /// needs the person's approval unless the agent runs in yolo; every newly
    /// trusted folder is announced once in the agent's pane.
    pub(super) async fn admit_claude_workspace_trust(
        &self,
        request: &crate::provider::LaunchProviderRequest,
    ) -> Result<(), DaemonError> {
        let Some(folder) = crate::provider::untrusted_claude_workspace(request)? else {
            return Ok(());
        };
        if folder.awaits_hook_approval(request) {
            self.confirm_claude_project_hooks(request, &folder).await?;
        }
        if let Some(trusted) = crate::provider::trust_claude_workspace(request)? {
            let recipients = self
                .owned
                .attachment_store
                .list_session_attachment_ids(&request.session_id);
            self.owned.record_notice_for_agent(
                &request.session_id,
                None,
                request.agent_id.as_deref(),
                recipients,
                format!(
                    "Chariox trusted {} in Claude Code for you.",
                    trusted.display()
                ),
            );
        }
        Ok(())
    }

    async fn confirm_claude_project_hooks(
        &self,
        request: &crate::provider::LaunchProviderRequest,
        folder: &crate::provider::UntrustedClaudeWorkspace,
    ) -> Result<(), DaemonError> {
        let refuse = |message: String| DaemonError::LocalTransport {
            operation: "launch provider run",
            message,
        };
        let agent_id = request.agent_id.as_deref().ok_or_else(|| {
            refuse(format!(
                "{} ships Claude hooks and needs your approval; start the agent from a session so the approval card can be shown",
                folder.path.display()
            ))
        })?;
        let interaction = RuntimeInteraction::new(
            format!(
                "claude-workspace-hooks-{agent_id}-{}",
                crate::session::unix_epoch_ms()
            ),
            agent_id,
            RuntimeInteractionKind::Permission,
            RuntimeInteractionLevel::Warning,
            Some("Trust this folder and run its Claude hooks?".to_string()),
            hooks_card_message(folder),
            vec![
                RuntimeInteractionChoice::new(
                    "trust",
                    "Trust folder",
                    "trust",
                    Some(RuntimeInteractionChoiceStyle::Primary),
                ),
                RuntimeInteractionChoice::new(
                    "cancel",
                    "Cancel",
                    "cancel",
                    Some(RuntimeInteractionChoiceStyle::Secondary),
                ),
            ],
            None,
            Some(HOOK_CARD_TIMEOUT_SEC),
            Some("cancel".to_string()),
        );
        let resolution =
            crate::runtime::native_interaction_bridge::request_runtime_interaction_with_timeout(
                self,
                &request.session_id,
                interaction,
                "launch provider run",
                None,
            )
            .await?;
        if resolution.status == "answered" && resolution.choice_id.as_deref() == Some("trust") {
            return Ok(());
        }
        Err(refuse(format!(
            "{} was not trusted, so the agent did not start; start it again and choose Trust folder to run its Claude hooks",
            folder.path.display()
        )))
    }
}

fn hooks_card_message(folder: &crate::provider::UntrustedClaudeWorkspace) -> String {
    let mut lines = vec![format!(
        "{} ships Claude hooks. Trusting the folder lets Claude Code run these commands on this machine:",
        folder.path.display()
    )];
    for hook in folder.hooks.iter().take(HOOK_LINES_SHOWN) {
        lines.push(format!(
            "• {}: {}",
            hook.event,
            crate::session::bounded_display_text(&hook.action, HOOK_ACTION_CHARS, false)
        ));
    }
    if folder.hooks.len() > HOOK_LINES_SHOWN {
        lines.push(format!(
            "… and {} more",
            folder.hooks.len() - HOOK_LINES_SHOWN
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::provider::{AgentPermissionLevel, LaunchProviderRequest, ProviderClientInterface};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    struct Gate {
        _folder: crate::test_support::TestWorktree,
        _profile: crate::test_support::TestWorktree,
        state: KernelRuntimeState,
        request: LaunchProviderRequest,
        folder: std::path::PathBuf,
        profile_state: std::path::PathBuf,
        session_id: String,
        agent_id: String,
        attachment_id: String,
    }

    fn gate(permission_level: AgentPermissionLevel, hooks: Option<&str>) -> Gate {
        let folder = crate::test_support::TestWorktree::new("trust-folder");
        let profile = crate::test_support::TestWorktree::new("trust-profile");
        if let Some(hooks) = hooks {
            std::fs::create_dir_all(folder.path().join(".claude")).unwrap();
            std::fs::write(folder.path().join(".claude/settings.json"), hooks).unwrap();
        }
        let config = crate::config::DaemonConfig::for_tests();
        let mut app = crate::test_support::bootstrap_authenticated_app(config).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(folder.session_request())
            .unwrap();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "trust-terminal",
                ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        let app = Arc::new(Mutex::new(app));
        let router =
            crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(app, 32);
        let request =
            LaunchProviderRequest::new(session.id(), "claude", "claude", "default", "haiku")
                .with_agent_id(agent.id())
                .with_client_interface(ProviderClientInterface::NativeTui)
                .with_working_directory(folder.path().to_owned())
                .with_permission_level(permission_level)
                .with_provider_account_env(std::collections::BTreeMap::from([(
                    "CLAUDE_CONFIG_DIR".to_string(),
                    profile.path().display().to_string(),
                )]));
        Gate {
            folder: folder.path().to_owned(),
            profile_state: profile.path().join(".claude.json"),
            _folder: folder,
            _profile: profile,
            state: router.runtime_state(),
            request,
            session_id: session.id().into(),
            agent_id: agent.id().into(),
            attachment_id: attachment.id().into(),
        }
    }

    impl Gate {
        fn trusted(&self) -> bool {
            std::fs::read(&self.profile_state)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                .is_some_and(|state| {
                    state["projects"][self.folder.to_str().unwrap()]["hasTrustDialogAccepted"]
                        == true
                })
        }

        fn notices(&self) -> Vec<String> {
            self.state
                .owned
                .terminal_stream
                .drain_notice_records(&self.session_id, &self.attachment_id)
                .into_iter()
                .map(|notice| notice.message)
                .collect()
        }

        fn card(&self) -> Option<crate::session::RuntimeInteraction> {
            self.state
                .owned
                .session_store
                .get_session(&self.session_id)
                .unwrap()
                .active_interaction_for_agent(&self.agent_id)
                .cloned()
        }
    }

    const HOOKS: &str = r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"./audit.sh"}]}]}}"#;

    #[tokio::test]
    async fn folder_without_hooks_in_ask_mode_is_trusted_with_a_notice_and_no_card() {
        let g = gate(AgentPermissionLevel::Required, None);
        g.state
            .admit_claude_workspace_trust(&g.request)
            .await
            .unwrap();
        assert!(g.card().is_none());
        assert!(g.trusted());
        let notices = g.notices();
        assert_eq!(notices.len(), 1);
        assert!(notices[0].contains(g.folder.to_str().unwrap()));
        // Trusting again is silent: the notice is for the first time only.
        g.state
            .admit_claude_workspace_trust(&g.request)
            .await
            .unwrap();
        assert!(g.notices().is_empty());
    }

    #[tokio::test]
    async fn folder_with_hooks_in_ask_mode_waits_for_the_card_and_trusts_on_approval() {
        let g = gate(AgentPermissionLevel::Required, Some(HOOKS));
        let mut admit = Box::pin(g.state.admit_claude_workspace_trust(&g.request));
        tokio::select! {
            result = &mut admit => panic!("trust was admitted before approval: {result:?}"),
            _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {}
        }
        let card = g.card().expect("hooks card should be shown");
        assert!(card.message().contains("./audit.sh"));
        assert!(!g.trusted());
        g.state
            .resolve_terminal_runtime_interaction(
                &g.session_id,
                card.id(),
                "trust",
                None,
                Some(crate::session::DEFAULT_LOCAL_USER_ID),
            )
            .await
            .unwrap();
        admit.await.unwrap();
        assert!(g.trusted());
        assert_eq!(g.notices().len(), 1);
    }

    #[tokio::test]
    async fn cancelling_the_hooks_card_stops_the_start_and_trusts_nothing() {
        let g = gate(AgentPermissionLevel::Required, Some(HOOKS));
        let mut admit = Box::pin(g.state.admit_claude_workspace_trust(&g.request));
        tokio::select! {
            result = &mut admit => panic!("trust was admitted before approval: {result:?}"),
            _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {}
        }
        let card = g.card().expect("hooks card should be shown");
        g.state
            .resolve_terminal_runtime_interaction(
                &g.session_id,
                card.id(),
                "cancel",
                None,
                Some(crate::session::DEFAULT_LOCAL_USER_ID),
            )
            .await
            .unwrap();
        let error = admit.await.unwrap_err();
        assert!(error.to_string().contains("was not trusted"));
        assert!(!g.trusted());
        assert!(g.notices().is_empty());
    }

    #[tokio::test]
    async fn folder_with_hooks_in_yolo_is_trusted_with_only_the_notice() {
        let g = gate(AgentPermissionLevel::Yolo, Some(HOOKS));
        g.state
            .admit_claude_workspace_trust(&g.request)
            .await
            .unwrap();
        assert!(g.card().is_none());
        assert!(g.trusted());
        assert_eq!(g.notices().len(), 1);
    }
}
