//! MP-08/MP-10/MP-11: account authority follows worker lease bindings.
use super::*;

impl KernelRuntimeState {
    // MP-08/MP-10/MP-11: metadata reads use the same caller-owned exact
    // reference namespace as launch. The home picker and mutations stay scoped.
    pub(crate) fn provider_account_owner_for_profile_read(
        &self,
        runtime_owner: &str,
        provider: &str,
        reference: &str,
    ) -> Result<String, DaemonError> {
        crate::account_profile::provider_account_authority_owner_for_profile(
            &self.owned.config_projection.snapshot(),
            &self.owned.provider_account_profiles,
            runtime_owner,
            provider,
            reference,
        )
    }

    pub(super) async fn resolve_provider_launch_account_authority(
        &self,
        mut request: crate::provider::LaunchProviderRequest,
    ) -> Result<crate::provider::LaunchProviderRequest, DaemonError> {
        let session = self.owned.session_store.get_session(&request.session_id)?;
        if request.agent_id.is_none() {
            request.agent_id = session.focused_agent_id().map(str::to_string).or_else(|| {
                self.owned
                    .agent_store
                    .get_focused_agent(&request.session_id)
                    .map(|agent| agent.id().to_string())
            });
        }
        let execution_owner = self
            .with_app_side_effect(|app| {
                app.provider_account_owner_for_execution(
                    &request.session_id,
                    request.agent_id.as_deref(),
                )
            })
            .await?;
        request.owner_user_id = match request.agent_id.as_deref() {
            Some(id) => self
                .owned
                .agent_store
                .get_agent(id)?
                .owner_user_id()
                .to_string(),
            None => session.owner_user_id().to_string(),
        };
        let config = self.owned.config_projection.snapshot();
        request.provider_account_owner_user_id = Some(
            if execution_owner
                != crate::account_profile::provider_account_authority_owner_user_id(
                    &config,
                    &request.owner_user_id,
                )
            {
                execution_owner
            } else {
                crate::account_profile::provider_account_authority_for_launch(
                    &config,
                    &self.owned.provider_account_profiles,
                    &request,
                )?
            },
        );
        Ok(request)
    }

    pub(super) async fn provider_account_owner_for_run(
        &self,
        run: &crate::provider::RuntimeProviderRun,
    ) -> Result<String, DaemonError> {
        self.with_app_side_effect(|app| {
            app.provider_account_owner_for_execution(run.session_id(), run.agent_instance_id())
        })
        .await
    }
}

// MP-08/MP-10/MP-11: owned admission/queue paths cannot take the app lock.
// The kernel marks lease runs before admission; only that validated execution
// binding keeps its owner namespace. Home remote projections keep home aliases.
impl KernelRuntimeOwnedState {
    pub(super) fn provider_account_authority_owner_for_agent(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Result<String, DaemonError> {
        let session = self.session_store.get_session(agent.session_id())?;
        let leased_run = if agent.remote_execution().is_none() {
            self.provider_store
                .get_run_for_agent(agent.session_id(), agent.id())
                .filter(|run| {
                    self.provider_run_projection
                        .is_leased_provider_run(run.id())
                })
        } else {
            None
        };
        if let Some(run) = leased_run {
            if run.session_id() != agent.session_id()
                || run.agent_instance_id() != Some(agent.id())
                || run.owner_user_id() != agent.owner_user_id()
                || session.owner_user_id() != agent.owner_user_id()
            {
                return Err(DaemonError::LocalTransport {
                    operation: "resolve leased admission account authority",
                    message:
                        "lease provider run does not match its backing agent and session owner"
                            .into(),
                });
            }
            return Ok(agent.owner_user_id().to_string());
        }
        let Some(provider) = crate::provider::canonical_provider_family(agent.provider()) else {
            return Ok(
                crate::account_profile::provider_account_authority_owner_user_id(
                    &self.config_projection.snapshot(),
                    agent.owner_user_id(),
                ),
            );
        };
        crate::account_profile::provider_account_authority_owner_for_profile(
            &self.config_projection.snapshot(),
            &self.provider_account_profiles,
            agent.owner_user_id(),
            provider,
            agent.provider_account_profile(),
        )
    }
}
