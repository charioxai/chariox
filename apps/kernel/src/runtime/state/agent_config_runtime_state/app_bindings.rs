use super::*;
use crate::extension::{ExtensionGrant, ExtensionKind};

impl KernelRuntimeState {
    pub(super) async fn grant_agent_app(
        &self,
        agent_ref: &str,
        grant: ExtensionGrant,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        grant.validate_app_binding()?;
        self.owned.ensure_agent_extension_authority(
            agent_ref,
            caller_user_id,
            "grant agent App",
        )?;
        let agent = self
            .owned
            .agent_store
            .get_agent(agent_ref)
            .or_else(|_| self.owned.agent_store.get_agent_by_ref(agent_ref))?;
        let store = self.owned.durable_state_store.clone();
        let permit = self
            .app_control()
            .try_admit()
            .map_err(|_| app_binding_error("App control is busy"))?;
        let owner = caller_user_id.to_string();
        let installation_id = grant.name.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            store.check_app_binding(&owner, &installation_id)
        })
        .await
        .map_err(|_| app_binding_error("App binding check did not complete"))?
        .map_err(|_| {
            app_binding_error(
                "App installation is unavailable or its publisher verification is no longer valid",
            )
        })?;
        // Ownership is checked again after the writer wait. A binding does not
        // retain invocation authority if the App is subsequently retired/revoked.
        let agent = self
            .owned
            .grant_agent_extension(agent.id(), grant.clone(), caller_user_id)?;
        self.append_agent_durable_event(
            "agent.extension_granted",
            &agent,
            Some(&format!("app:{}", grant.name)),
        )
        .await?;
        self.append_home_extension_grant_audit_event(
            "home_extension.grant.created",
            &agent,
            caller_user_id,
            &grant,
        )?;
        self.sync_remote_extension_manifest_for_agent(&agent, Some(caller_user_id), Some(false))
            .await?;
        self.invalidate_workflow_copies_after_source_agent_change(agent.session_id(), agent.id())?;
        Ok(agent)
    }

    pub(super) async fn revoke_agent_app(
        &self,
        agent_ref: &str,
        name: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let grant = ExtensionGrant::app(name);
        grant.validate_app_binding()?;
        // Revocation must work even after uninstall or publisher revocation.
        let agent = self.owned.revoke_agent_extension(
            agent_ref,
            ExtensionKind::App,
            name,
            caller_user_id,
        )?;
        self.append_agent_durable_event(
            "agent.extension_revoked",
            &agent,
            Some(&format!("app:{name}")),
        )
        .await?;
        self.append_home_extension_named_grant_audit_event(
            "home_extension.grant.revoked",
            &agent,
            caller_user_id,
            ExtensionKind::App,
            name,
        )?;
        self.sync_remote_extension_manifest_for_agent(&agent, Some(caller_user_id), Some(true))
            .await?;
        self.invalidate_workflow_copies_after_source_agent_change(agent.session_id(), agent.id())?;
        Ok(agent)
    }

    /// Agent-initiated binding uses the existing session/agent permission mode
    /// and RuntimeInteraction. Direct authenticated user grants skip this helper.
    pub(crate) async fn authorize_agent_app_binding(
        &self,
        session_id: &str,
        initiating_agent_id: &str,
        target_agent_id: &str,
        installation_id: &str,
    ) -> Result<bool, DaemonError> {
        ExtensionGrant::app(installation_id).validate_app_binding()?;
        let session = self.owned.session_store.get_session(session_id)?;
        let agent = self.owned.agent_store.get_agent(initiating_agent_id)?;
        let target = self.owned.agent_store.get_agent(target_agent_id)?;
        if agent.session_id() != session.id()
            || target.session_id() != session.id()
            || agent.owner_user_id() != target.owner_user_id()
        {
            return Err(app_binding_error(
                "App binding target is outside this agent's authority",
            ));
        }
        self.owned.ensure_agent_extension_authority(
            target.id(),
            agent.owner_user_id(),
            "request agent App binding",
        )?;
        if target.has_extension_grant(ExtensionKind::App, installation_id) {
            return Ok(true);
        }
        if !crate::session::effective_agent_user_authority(&session, Some(&agent))
            .requires_approval()
        {
            return Ok(true);
        }
        let interaction = crate::session::RuntimeInteraction::new(
            format!(
                "app-binding-permission-{}-{:016x}",
                agent.id(),
                rand::random::<u64>()
            ),
            agent.id(),
            crate::session::RuntimeInteractionKind::Permission,
            crate::session::RuntimeInteractionLevel::Warning,
            Some("Chariox App binding approval".into()),
            format!(
                "Allow agent `{}` to bind App installation `{installation_id}` to agent `{}`?",
                agent.agent_ref(),
                target.agent_ref()
            ),
            vec![
                crate::session::RuntimeInteractionChoice::new(
                    "allow",
                    "Allow",
                    "allow",
                    Some(crate::session::RuntimeInteractionChoiceStyle::Primary),
                ),
                crate::session::RuntimeInteractionChoice::new(
                    "deny",
                    "Deny",
                    "deny",
                    Some(crate::session::RuntimeInteractionChoiceStyle::Danger),
                ),
            ],
            None,
            None,
            None,
        );
        let resolution = self
            .create_runtime_interaction(session.id(), interaction)
            .await?
            .await
            .map_err(|_| app_binding_error("App binding approval closed without a decision"))?;
        // A deleted/reassigned initiating agent must not retain a pending grant.
        let current = self.owned.agent_store.get_agent(agent.id())?;
        if current.owner_user_id() != agent.owner_user_id() || current.session_id() != session.id()
        {
            return Err(app_binding_error(
                "App binding caller changed while approval was pending",
            ));
        }
        Ok(resolution.choice_id.as_deref() == Some("allow"))
    }
}

fn app_binding_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "agent.extension.app",
        message: message.into(),
    }
}
