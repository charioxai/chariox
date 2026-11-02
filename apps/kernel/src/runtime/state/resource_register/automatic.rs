use super::*;
use crate::config::UserCredentialConfig;

impl KernelRuntimeState {
    /// Shared by generated and captured credentials, including future capture entry points.
    /// It reads provenance only and never opens the Vault.
    pub(in crate::runtime) fn record_vault_resource(
        &self,
        credential: &UserCredentialConfig,
    ) -> Result<(), DaemonError> {
        let Some(metadata) = credential.metadata.as_ref() else {
            return Ok(());
        };
        let Some(kind) = metadata.created_by_kind.as_deref() else {
            return Ok(());
        };
        if !matches!(
            kind,
            "vault_generate"
                | "captured"
                | "capture_browser"
                | "capture_command"
                | "capture_file"
                | "capture_terminal"
                | "agent_auth"
        ) {
            return Ok(());
        }
        let (Some(room), Some(agent)) = (
            metadata.session_id.as_deref(),
            metadata.created_by_id.as_deref(),
        ) else {
            return Err(error(
                "created credential is missing Room or agent provenance",
            ));
        };
        let session = self.owned.session_store.get_session(room)?;
        let owner = session.owner_user_id();
        let id = resource_id(&["credential", &credential.id]);
        if let Some(existing) =
            self.owned
                .durable_state_store
                .created_resource(owner, session.project_id(), &id)?
        {
            if existing.credential_id.as_deref() != Some(credential.id.as_str()) {
                return Err(error(
                    "credential resource link conflicts with the existing register row",
                ));
            }
            // Updating a Vault handle cannot rewrite who originally created its resource.
            return Ok(());
        }
        let agent = self.owned.agent_store.get_agent(agent)?;
        if agent.session_id() != room {
            return Err(error("credential provenance does not match its Room"));
        }
        let row = CreatedResource {
            id, project_id: session.project_id().into(),
            service: credential.allowed_hosts.first().cloned().unwrap_or_else(|| "Vault".into()),
            kind: if kind == "vault_generate" { "password" } else { "credential" }.into(),
            name: credential.id.clone(), url: None, credential_id: Some(credential.id.clone()),
            created_by: agent.id().into(), created_at_ms: metadata.created_at_ms.unwrap_or_else(crate::session::unix_epoch_ms),
            revoke: ResourceRevokeMethod::Unavailable { reason: "Record the service's revoke request or account settings URL with chariox.mandate.set_resource_revoke before cleanup.".into() },
            state: CreatedResourceState::Active, revoked_at_ms: None, last_http_status: None,
        };
        self.owned
            .durable_state_store
            .record_created_resource(owner, &row)?;
        Ok(())
    }

    pub(in crate::runtime) fn record_ssh_resource(
        &self,
        turn: &KernelSudoTurn,
        host: &str,
        install_id: &str,
    ) -> Result<(), DaemonError> {
        // The privileged request already admitted this turn. Recording the result must survive
        // the turn ending during SSH installation: it records an existing effect, not new authority.
        let session = self.owned.session_store.get_session(&turn.session_id)?;
        let row = CreatedResource { id: resource_id(&["ssh",install_id]), project_id: session.project_id().into(),
            service: host.into(), kind: "ssh_machine".into(), name: install_id.into(), url: None,
            credential_id: None, created_by: turn.agent_id.clone(), created_at_ms: crate::session::unix_epoch_ms(),
            revoke: ResourceRevokeMethod::Unavailable { reason: format!("Use RemoveSshMachine with install_id {install_id} to remove this installation.") },
            state: CreatedResourceState::Active, revoked_at_ms: None, last_http_status: None };
        self.owned
            .durable_state_store
            .record_created_resource(&turn.owner_user_id, &row)?;
        Ok(())
    }
    pub(in crate::runtime) fn record_ssh_removal(
        &self,
        turn: &KernelSudoTurn,
        install_id: &str,
    ) -> Result<(), DaemonError> {
        let session = self.owned.session_store.get_session(&turn.session_id)?;
        let store = &self.owned.durable_state_store;
        let Some(mut row) = store.created_resource(
            &turn.owner_user_id,
            session.project_id(),
            &resource_id(&["ssh", install_id]),
        )?
        else {
            return Ok(());
        };
        if row.state == CreatedResourceState::Revoked {
            return Ok(());
        }
        let previous = row.clone();
        row.state = CreatedResourceState::Revoked;
        row.revoked_at_ms = Some(crate::session::unix_epoch_ms());
        store.change_created_resource(&turn.owner_user_id, &row, Some(&previous))
    }
}
