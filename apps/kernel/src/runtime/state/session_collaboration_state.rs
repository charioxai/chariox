use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::*;

impl KernelRuntimeState {
    pub(crate) fn list_session_members(
        &self,
        session_id: &str,
    ) -> Result<
        (
            Vec<crate::session::SessionMember>,
            Vec<crate::session::SessionInvite>,
        ),
        DaemonError,
    > {
        self.owned
            .session_store
            .read()
            .list_session_members(session_id)
    }

    pub(crate) fn create_session_invite(
        &self,
        session_id: &str,
        invite_id: String,
        created_by_user_id: String,
        expires_at_ms: Option<u64>,
        max_uses: Option<u32>,
        collaboration_level: crate::session::CollaborationLevel,
    ) -> Result<
        (
            crate::session::RuntimeSession,
            crate::session::SessionInvite,
        ),
        DaemonError,
    > {
        let (session, invite) = self.owned.session_store.write().create_session_invite(
            session_id,
            invite_id,
            created_by_user_id,
            expires_at_ms,
            max_uses,
            collaboration_level,
        )?;
        let session = self.owned.session_snapshot(session.id())?;
        Ok((session, invite))
    }

    pub(crate) fn join_session_invite(
        &self,
        session_id: &str,
        invite_id: &str,
        user_id: String,
        now_ms: u64,
    ) -> Result<
        (
            crate::session::RuntimeSession,
            crate::session::SessionMember,
        ),
        DaemonError,
    > {
        let (session, member) = self
            .owned
            .session_store
            .write()
            .join_session_invite(session_id, invite_id, user_id, now_ms)?;
        let session = self.owned.session_snapshot(session.id())?;
        Ok((session, member))
    }

    pub(crate) fn revoke_session_invite(
        &self,
        session_id: &str,
        invite_ref: &str,
    ) -> Result<
        (
            crate::session::RuntimeSession,
            crate::session::SessionInvite,
        ),
        DaemonError,
    > {
        let (session, invite) = self
            .owned
            .session_store
            .write()
            .revoke_session_invite(session_id, invite_ref)?;
        let session = self.owned.session_snapshot(session.id())?;
        Ok((session, invite))
    }

    pub(crate) fn create_workspace_link(
        &self,
        session_id: &str,
        name: String,
        created_by_user_id: String,
    ) -> Result<
        (
            crate::session::RuntimeSession,
            crate::session::WorkspaceLinkDefinition,
        ),
        DaemonError,
    > {
        let (session, link) = self.owned.session_store.write().create_workspace_link(
            session_id,
            name,
            created_by_user_id,
        )?;
        let session = self.owned.session_snapshot(session.id())?;
        Ok((session, link))
    }

    pub(crate) fn list_workspace_links(
        &self,
        session_id: &str,
    ) -> Result<Vec<crate::session::WorkspaceLinkDefinition>, DaemonError> {
        self.owned
            .session_store
            .read()
            .list_workspace_links(session_id)
    }

    pub(crate) fn set_workspace_live_sync_mode(
        &self,
        session_id: &str,
        mode: crate::config::WorkspaceLiveSyncMode,
        caller_user_id: &str,
        command: Option<&crate::runtime::command::KernelCommand>,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        let previous_mode = self
            .owned
            .session_store
            .get_session(session_id)?
            .workspace_live_sync_mode();
        let session = self
            .owned
            .session_store
            .write()
            .set_workspace_live_sync_mode(session_id, mode)?;
        let session = self.owned.session_snapshot(session.id())?;
        self.record_workspace_live_sync_mode_changed_event(
            &session,
            previous_mode,
            mode,
            caller_user_id,
            command,
        );
        Ok(session)
    }

    fn record_workspace_live_sync_mode_changed_event(
        &self,
        session: &crate::session::RuntimeSession,
        previous_mode: Option<crate::config::WorkspaceLiveSyncMode>,
        mode: crate::config::WorkspaceLiveSyncMode,
        caller_user_id: &str,
        command: Option<&crate::runtime::command::KernelCommand>,
    ) {
        let mut metadata = BTreeMap::new();
        metadata.insert(
            "caller_user_id".to_string(),
            serde_json::json!(caller_user_id),
        );
        metadata.insert(
            "previous_mode".to_string(),
            previous_mode
                .map(|mode| serde_json::json!(mode.as_config_str()))
                .unwrap_or(serde_json::Value::Null),
        );
        metadata.insert("mode".to_string(), serde_json::json!(mode.as_config_str()));
        metadata.insert(
            "scope".to_string(),
            serde_json::json!("selected_workspace_worktree"),
        );
        metadata.insert(
            "other_repositories".to_string(),
            serde_json::json!("unrestricted"),
        );
        if let Some(command) = command {
            metadata.insert(
                "command_id".to_string(),
                serde_json::json!(command.command_id),
            );
            metadata.insert(
                "correlation_id".to_string(),
                serde_json::json!(command.correlation_id),
            );
            metadata.insert(
                "command_source".to_string(),
                serde_json::json!(format!("{:?}", command.source)),
            );
            metadata.insert(
                "caller_kind".to_string(),
                serde_json::json!(format!("{:?}", command.caller.caller_kind)),
            );
            if let Some(client_id) = command.caller.client_id.as_deref() {
                metadata.insert("client_id".to_string(), serde_json::json!(client_id));
            }
            if let Some(machine_id) = command.caller.machine_id.as_deref() {
                metadata.insert("machine_id".to_string(), serde_json::json!(machine_id));
            }
        }

        if let Err(error) = self.owned.operational_history_store.append_operational_event(
            crate::history::HistoryEventKind::WorkspaceLiveSyncModeChanged,
            Some(crate::history::HistoryEventRole::System),
            Some(format!(
                "Workspace live sync mode changed to {} for selected workspace/worktree; other repositories remain unrestricted.",
                mode.as_config_str(),
            )),
            metadata,
            crate::history::HistoryEventTurnContext {
                workspace_id: Some(session.workspace_id().to_string()),
                session_id: Some(session.id().to_string()),
                worktree_path: Some(session.worktree_id().to_string()),
                ..Default::default()
            },
        ) {
            crate::logging::warn_with_fields(
                "daemon.history",
                "failed to append workspace live sync mode audit event",
                serde_json::json!({
                    "session_id": session.id(),
                    "mode": mode.as_config_str(),
                    "error": error.to_string(),
                }),
            );
        }
    }

    pub(crate) fn resolve_workspace_link_ref(
        &self,
        session_id: &str,
        link_ref: &str,
    ) -> Result<crate::session::WorkspaceLinkDefinition, DaemonError> {
        self.owned
            .session_store
            .read()
            .resolve_workspace_link_ref(session_id, link_ref)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn attach_workspace_link(
        &self,
        session_id: &str,
        link_ref: &str,
        user_id: String,
        machine_id: String,
        kernel_id: String,
        repo_root: String,
        branch: Option<String>,
        repo_fingerprint: Option<String>,
    ) -> Result<
        (
            crate::session::RuntimeSession,
            crate::session::WorkspaceLinkDefinition,
            crate::session::WorkspaceLinkAttachment,
        ),
        DaemonError,
    > {
        let result = self.owned.session_store.write().attach_workspace_link(
            session_id,
            link_ref,
            user_id,
            machine_id,
            kernel_id,
            repo_root,
            branch,
            repo_fingerprint,
        )?;
        let session = self.owned.session_snapshot(result.0.id())?;
        Ok((session, result.1, result.2))
    }

    pub(crate) fn record_workspace_live_sync_enrollment_notice(
        &self,
        session_id: &str,
        link_name: &str,
        repo_root: &str,
        mode: crate::config::WorkspaceLiveSyncMode,
    ) {
        let mode_label = match mode {
            crate::config::WorkspaceLiveSyncMode::Managed => "managed",
            crate::config::WorkspaceLiveSyncMode::Tracked => "tracked",
            crate::config::WorkspaceLiveSyncMode::Unrestricted => "off",
        };
        let next_action = match mode {
            crate::config::WorkspaceLiveSyncMode::Managed => {
                "managed mode is already active for this session"
            }
            crate::config::WorkspaceLiveSyncMode::Tracked => {
                "tracked mode is active; switch with `workspace sync managed` when provider write enforcement is supported"
            }
            crate::config::WorkspaceLiveSyncMode::Unrestricted => {
                "live sync mode is unchanged; choose `workspace sync managed` or `workspace sync tracked` to start syncing this session"
            }
        };
        self.owned.record_notice(
            session_id,
            None,
            Vec::new(),
            format!(
                "Workspace live sync link `{link_name}` attached worktree `{repo_root}`. Current mode: {mode_label}. Mode choice: managed requires provider write fencing; tracked syncs at turn end. Next action: {next_action}."
            ),
        );
    }

    pub(crate) fn detach_workspace_link(
        &self,
        session_id: &str,
        link_ref: &str,
        user_id: String,
        repo_root: Option<&Path>,
    ) -> Result<
        (
            crate::session::RuntimeSession,
            crate::session::WorkspaceLinkDefinition,
            Vec<crate::session::WorkspaceLinkAttachment>,
        ),
        DaemonError,
    > {
        let result = self
            .owned
            .session_store
            .write()
            .detach_workspace_link(session_id, link_ref, user_id, repo_root)?;
        let session = self.owned.session_snapshot(result.0.id())?;
        Ok((session, result.1, result.2))
    }

    pub(crate) fn workspace_live_sync_target_results(
        &self,
        session_id: &str,
    ) -> Vec<crate::git_observer::WorkspaceLiveSyncTargetResult> {
        let mut results = self
            .owned
            .workspace_live_sync_journal
            .target_results_for_session(session_id);
        match self.owned.durable_state_store.load_subject_events_by_kind(
            session_id,
            "workspace_live_sync.target_results_recorded",
            200,
        ) {
            Ok(events) => {
                for event in events {
                    match event.payload.get("target_results").cloned().map(
                        serde_json::from_value::<
                            Vec<crate::git_observer::WorkspaceLiveSyncTargetResult>,
                        >,
                    ) {
                        Some(Ok(mut persisted)) => results.append(&mut persisted),
                        Some(Err(error)) => crate::logging::warn_with_fields(
                            "daemon.workspace_live_sync",
                            "failed to decode persisted workspace live sync target results",
                            serde_json::json!({
                                "session_id": session_id,
                                "event_id": event.event_id,
                                "error": error.to_string(),
                            }),
                        ),
                        None => {}
                    }
                }
            }
            Err(error) => crate::logging::warn_with_fields(
                "daemon.workspace_live_sync",
                "failed to load persisted workspace live sync target results",
                serde_json::json!({
                    "session_id": session_id,
                    "error": error.to_string(),
                }),
            ),
        }
        let mut seen = BTreeSet::new();
        results
            .into_iter()
            .filter(|result| {
                let key = serde_json::to_string(result).unwrap_or_else(|_| {
                    format!(
                        "{}:{}:{}:{}",
                        result.session_id,
                        result.link_id,
                        result.source_agent_id,
                        result.target_repo_root
                    )
                });
                seen.insert(key)
            })
            .collect()
    }
}
