use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn grant_agent_extension(
        &self,
        agent_ref: &str,
        grant: crate::extension::ExtensionGrant,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.ensure_agent_ref_owner(agent_ref, caller_user_id, "grant agent extension")?;
        let agent = self.agent_store.grant_extension(agent_ref, grant)?;
        let _ = self.session_snapshot(agent.session_id())?;
        Ok(agent)
    }

    pub(super) fn revoke_agent_extension(
        &self,
        agent_ref: &str,
        kind: crate::extension::ExtensionKind,
        name: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.ensure_agent_ref_owner(agent_ref, caller_user_id, "revoke agent extension")?;
        let agent = self.agent_store.revoke_extension(agent_ref, kind, name)?;
        let _ = self.session_snapshot(agent.session_id())?;
        Ok(agent)
    }

    pub(super) fn grant_agent_mcp(
        &self,
        agent_ref: &str,
        name: String,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.ensure_agent_ref_owner(agent_ref, caller_user_id, "grant agent capability")?;
        let agent = self.agent_store.grant_mcp(agent_ref, name)?;
        let _ = self.session_snapshot(agent.session_id())?;
        Ok(agent)
    }

    pub(super) fn revoke_agent_mcp(
        &self,
        agent_ref: &str,
        name: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.ensure_agent_ref_owner(agent_ref, caller_user_id, "revoke agent capability")?;
        let agent = self.agent_store.revoke_mcp(agent_ref, name)?;
        let _ = self.session_snapshot(agent.session_id())?;
        Ok(agent)
    }

    pub(super) fn grant_agent_skill(
        &self,
        agent_ref: &str,
        name: String,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.ensure_agent_ref_owner(agent_ref, caller_user_id, "grant agent capability")?;
        let agent = self.agent_store.grant_skill(agent_ref, name)?;
        let _ = self.session_snapshot(agent.session_id())?;
        Ok(agent)
    }

    pub(super) fn revoke_agent_skill(
        &self,
        agent_ref: &str,
        name: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.ensure_agent_ref_owner(agent_ref, caller_user_id, "revoke agent capability")?;
        let agent = self.agent_store.revoke_skill(agent_ref, name)?;
        let _ = self.session_snapshot(agent.session_id())?;
        Ok(agent)
    }

    pub(super) fn capability_context(
        &self,
        session_id: &str,
        attachment_id: &str,
        capability: &'static str,
    ) -> Result<CapabilityRuntimeSnapshot, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let attachment = self.ensure_attachment_in_session(session_id, attachment_id)?;
        if !matches!(
            attachment.capability_level(),
            crate::attachment::ClientCapabilityLevel::FullTerminal
                | crate::attachment::ClientCapabilityLevel::InteractiveStructured
        ) {
            return Err(DaemonError::AttachmentCapabilityDenied {
                session_id: session_id.to_string(),
                attachment_id: attachment.id().to_string(),
                capability,
            });
        }
        Ok(CapabilityRuntimeSnapshot {
            workspace_id: session.workspace_id().to_string(),
            worktree_root: std::path::PathBuf::from(session.worktree_id()),
            workspace_coordinator: self.workspace_coordinator.clone(),
            operational_history_store: self.operational_history_store.clone(),
            operational_artifact_root: self
                .config_projection
                .snapshot()
                .operational_artifact_root(),
            operational_artifact_index_path: self
                .config_projection
                .snapshot()
                .operational_artifact_index_path(),
            history_archive_enabled: self
                .config_projection
                .snapshot()
                .user_config
                .history
                .archive
                .mode
                == crate::config::HistoryArchiveMode::External,
        })
    }

    pub(super) fn managed_io_domain_from_arg(
        domain: Option<&str>,
    ) -> Result<crate::io::ArtifactDomainKind, DaemonError> {
        match domain.unwrap_or("text") {
            "text" => Ok(crate::io::ArtifactDomainKind::TextDocument),
            "structured" => Ok(crate::io::ArtifactDomainKind::StructuredDocument),
            "opaque" => Ok(crate::io::ArtifactDomainKind::OpaqueBlob),
            other => Err(DaemonError::LocalTransport {
                operation: "runtime_tool_managed_io",
                message: format!("unsupported artifact domain `{other}`"),
            }),
        }
    }
}
