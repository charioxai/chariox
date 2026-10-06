//! MP-08/MP-11: native source confirmation gates credential-free owner copies.
use super::*;
use crate::managed_context::owner_managed::admission_error;
use crate::managed_context::package::ManagedContextDevelopmentSelection;
use crate::session::{
    RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionChoiceStyle,
    RuntimeInteractionKind, RuntimeInteractionLevel,
};

impl KernelRuntimeState {
    pub(crate) async fn review_credential_free_owner_context(
        &self,
        development: &ManagedContextDevelopmentSelection,
        context_id: &str,
        target: &str,
    ) -> Result<(), DaemonError> {
        if let ManagedContextDevelopmentSelection::SourceProject {
            project_id,
            repositories,
        } = development
        {
            let selections = repositories
                .iter()
                .map(crate::managed_context::outbound_service::resolve_repository_selection)
                .collect::<Result<Vec<_>, _>>()?;
            return self
                .review_credential_free_project_context(project_id, &selections, target)
                .await;
        }
        // A kernel-only copy has no selected Project. Use an existing owner
        // session so the normal TUI/Web interaction projection remains the sole
        // approval path; never manufacture a provider run or approve unattended.
        let config = self.owned.config_projection.snapshot();
        let owner = config
            .cloud_relay
            .as_ref()
            .map(|profile| profile.user_id.as_str())
            .ok_or_else(|| admission_error("Source kernel has no enrolled owner"))?;
        let (session, agent) = self
            .owned
            .session_store
            .list_sessions()
            .into_iter()
            .filter(|session| {
                [owner, crate::session::DEFAULT_LOCAL_USER_ID].contains(&session.owner_user_id())
            })
            .find_map(|session| {
                self.owned
                    .agent_store
                    .get_session_agents(session.id())
                    .into_iter()
                    .find(|agent| {
                        agent.remote_execution().is_none()
                            && !session
                                .active_interactions()
                                .iter()
                                .any(|interaction| interaction.agent_id() == Some(agent.id()))
                            && self
                                .owned
                                .prompt_state_owner
                                .active_prompt_for_agent(&session, agent.id())
                                .is_none()
                    })
                    .map(|agent| (session, agent))
            })
            .ok_or_else(|| {
                admission_error(
                    "Open an idle source-kernel session to review kernel context, then retry",
                )
            })?;
        let id = format!("owner-context:{context_id}");
        let interaction = RuntimeInteraction::new(
            &id, agent.id(), RuntimeInteractionKind::Choice, RuntimeInteractionLevel::Info,
            Some(format!("Ready to copy kernel context to {target}")),
            "Copy kernel extensions, personal instructions and their packages. No Project or credentials are selected. Review the source content before continuing: automatic credential checks cannot identify every secret in free-form files.",
            vec![
                RuntimeInteractionChoice::new("continue", "Looks good, continue", "continue", Some(RuntimeInteractionChoiceStyle::Primary)),
                RuntimeInteractionChoice::new("cancel", "Cancel", "cancel", Some(RuntimeInteractionChoiceStyle::Danger)),
            ], None, Some(900), Some("cancel".into()),
        );
        let receiver = self
            .create_runtime_interaction(session.id(), interaction)
            .await?;
        match tokio::time::timeout(Duration::from_secs(900), receiver).await {
            Ok(Ok(resolution))
                if resolution.status == "answered"
                    && resolution.choice_id.as_deref() == Some("continue") =>
            {
                Ok(())
            }
            result => {
                if !matches!(result, Ok(Ok(_))) {
                    self.timeout_runtime_interaction(session.id(), &id).await?;
                }
                Err(DaemonError::ManagedContext {
                    code: "managed_context_review_cancelled",
                    operation: "owner-managed context review",
                    message: "Owner-managed context review was cancelled or expired".into(),
                    retryable: false,
                })
            }
        }
    }
}
