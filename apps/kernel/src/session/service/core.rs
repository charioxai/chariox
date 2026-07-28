use super::*;
use crate::session::WorkflowPublicationSnapshot;
use std::path::Path;

mod event_publication;
mod publication;

impl SessionService {
    pub fn new(config: &DaemonConfig) -> Self {
        Self {
            store: SessionStore::new(),
            ephemeral_session_ids: BTreeSet::new(),
            host_machine_id: config.host_machine_id.clone(),
            host_daemon_id: config.daemon_id.clone(),
            event_environment_id: config.event_delivery_environment_id.clone(),
            prompt_id_allocator: PromptIdAllocator::default(),
            next_workflow_number: 0,
            next_workflow_schema_number: 0,
            next_workflow_endpoint_number: 0,
            next_workflow_node_number: 0,
            next_workflow_edge_number: 0,
            next_workflow_run_number: 0,
            next_workflow_node_run_number: 0,
            next_workflow_message_number: 0,
            next_workflow_watchdog_number: 0,
            next_workflow_publication_number: 0,
            next_workflow_event_binding_number: 0,
            next_workflow_prompt_queue_number: 0,
            next_workflow_queued_prompt_number: 0,
            next_agent_prompt_schedule_number: 0,
            max_workflow_queues_per_workflow: config.max_workflow_queues_per_workflow(),
            session_default_max_agents: config.session_default_max_agents(),
            workflow_default_max_concurrent: config.workflow_code_limits().max_concurrent.max(1),
            next_workspace_link_number: 0,
        }
    }

    pub fn create_session(
        &mut self,
        request: CreateSessionRequest,
    ) -> Result<RuntimeSession, DaemonError> {
        if request.metaagent {
            return Err(DaemonError::LocalTransport {
                operation: "create session",
                message:
                    "creating separate metaagent sessions is deprecated; create a regular session and send `/meta <task>` to enter meta mode"
                        .to_string(),
            });
        }
        let alias = match request.alias {
            Some(alias) if !alias.trim().is_empty() => normalize_session_alias(Some(alias))?,
            _ if !request.hidden => Some(self.default_session_alias(&request.workspace_id)),
            _ => None,
        };
        if let Some(alias) = alias.as_deref() {
            self.ensure_alias_available(&request.workspace_id, alias)?;
        }
        let mut session = RuntimeSession::new(
            self.store.next_session_id(),
            alias,
            request.workspace_id,
            request.worktree_id,
            self.host_machine_id.clone(),
            self.host_daemon_id.clone(),
        );
        session.set_max_agents(self.session_default_max_agents);
        session.set_owner_user_id(request.owner_user_id);
        session.set_hidden(request.hidden);
        if let Some(agent_defaults) = request.agent_defaults {
            session.set_agent_defaults(agent_defaults);
        }
        if let Some(mode) = request.workspace_live_sync_mode {
            session.set_workspace_live_sync_mode(Some(mode));
        }

        Ok(self.store.insert(session))
    }

    pub(crate) fn create_ephemeral_session(
        &mut self,
        request: CreateSessionRequest,
    ) -> Result<RuntimeSession, DaemonError> {
        let session = self.create_session(request)?;
        self.ephemeral_session_ids.insert(session.id().to_string());
        Ok(session)
    }

    pub(crate) fn is_ephemeral_session(&self, session_id: &str) -> bool {
        self.ephemeral_session_ids.contains(session_id)
    }

    pub(crate) fn has_session(&self, session_id: &str) -> bool {
        self.store.get(session_id).is_some()
    }

    pub(crate) fn durable_sessions(&self) -> Vec<RuntimeSession> {
        self.store
            .list()
            .into_iter()
            .filter(|session| !self.is_ephemeral_session(session.id()))
            .collect()
    }

    fn default_session_alias(&self, workspace_id: &str) -> String {
        let base = default_session_alias_base(workspace_id);
        let mut number = self
            .store
            .list()
            .iter()
            .filter(|session| !session.is_hidden() && session.workspace_id() == workspace_id)
            .count()
            + 1;
        loop {
            let alias = format!("{base}-{number}");
            if self.store.visible_non_ended_sessions().all(|session| {
                session.workspace_id() != workspace_id || session.alias() != Some(alias.as_str())
            }) {
                return alias;
            }
            number += 1;
        }
    }

    pub(crate) fn restore_session(&mut self, session: RuntimeSession) -> RuntimeSession {
        self.store.insert(session)
    }

    pub(crate) fn remove_restored_session(&mut self, session_id: &str) -> Option<RuntimeSession> {
        self.ephemeral_session_ids.remove(session_id);
        self.store.remove(session_id)
    }

    pub(crate) fn replace_publication_runtime_workflows(
        &mut self,
        session_id: &str,
        workflows: Vec<WorkflowDefinition>,
        workflow_prompt_queues: Vec<WorkflowPromptQueueDefinition>,
        workflow_watchdogs: Vec<WorkflowWatchdogDefinition>,
    ) -> Result<RuntimeSession, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        session.replace_publication_runtime_workflows(
            workflows,
            workflow_prompt_queues,
            workflow_watchdogs,
        );
        let first_agent_id = session
            .workflows()
            .first()
            .and_then(|workflow| workflow.nodes().first())
            .map(|node| node.agent_id().to_string());
        session.set_focused_agent(first_agent_id);
        Ok(session.clone())
    }

    pub(crate) fn restore_workflow_publication(
        &mut self,
        session_id: &str,
        publication: WorkflowPublicationDefinition,
        source_snapshot: Option<WorkflowPublicationSnapshot>,
    ) -> Result<WorkflowPublicationDefinition, DaemonError> {
        if publication.session_id() != session_id {
            return Err(DaemonError::LocalTransport {
                operation: "restore workflow publication",
                message: format!(
                    "publication `{}` belongs to session `{}` instead of `{session_id}`",
                    publication.id(),
                    publication.session_id()
                ),
            });
        }
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        Ok(session.create_workflow_publication(publication, source_snapshot))
    }

    pub fn get_session(&self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        self.store
            .get(session_id)
            .cloned()
            .ok_or_else(|| DaemonError::SessionNotFound {
                session_id: session_id.to_string(),
            })
    }

    pub fn list_session_members(
        &self,
        session_id: &str,
    ) -> Result<(Vec<SessionMember>, Vec<SessionInvite>), DaemonError> {
        let session = self.get_session(session_id)?;
        Ok((session.members().to_vec(), session.invites().to_vec()))
    }

    pub fn create_workspace_link(
        &mut self,
        session_id: &str,
        name: String,
        created_by_user_id: String,
    ) -> Result<(RuntimeSession, WorkspaceLinkDefinition), DaemonError> {
        let normalized_name = normalize_workspace_link_name(&name)?;
        let link_id = self.next_workspace_link_id();
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        ensure_workspace_link_name_available(session, &normalized_name)?;
        let link = WorkspaceLinkDefinition::new(
            link_id,
            session_id.to_string(),
            normalized_name,
            created_by_user_id,
        );
        let link = session.create_workspace_link(link);
        session.touch();
        Ok((session.clone(), link))
    }

    pub fn list_workspace_links(
        &self,
        session_id: &str,
    ) -> Result<Vec<WorkspaceLinkDefinition>, DaemonError> {
        Ok(self.get_session(session_id)?.workspace_links().to_vec())
    }

    pub fn set_workspace_live_sync_mode(
        &mut self,
        session_id: &str,
        mode: crate::config::WorkspaceLiveSyncMode,
    ) -> Result<RuntimeSession, DaemonError> {
        let session =
            self.get_session_mut_for_operation(session_id, "set workspace live sync mode")?;
        session.set_workspace_live_sync_mode(Some(mode));
        Ok(session.clone())
    }

    pub fn resolve_workspace_link_ref(
        &self,
        session_id: &str,
        link_ref: &str,
    ) -> Result<WorkspaceLinkDefinition, DaemonError> {
        let session = self.get_session(session_id)?;
        resolve_workspace_link_ref_in_session(&session, link_ref).cloned()
    }

    pub fn attach_workspace_link(
        &mut self,
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
            RuntimeSession,
            WorkspaceLinkDefinition,
            WorkspaceLinkAttachment,
        ),
        DaemonError,
    > {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let link_id = resolve_workspace_link_ref_in_session(session, link_ref)?
            .link_id()
            .to_string();
        let attachment = WorkspaceLinkAttachment::new(
            link_id.clone(),
            user_id,
            machine_id,
            kernel_id,
            repo_root,
            branch,
            repo_fingerprint,
        );
        let link =
            session
                .workspace_link_mut(&link_id)
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "attach workspace link",
                    message: format!("workspace link `{link_ref}` was not found"),
                })?;
        let attachment = link.attach(attachment);
        let link = link.clone();
        session.touch();
        Ok((session.clone(), link, attachment))
    }

    pub fn detach_workspace_link(
        &mut self,
        session_id: &str,
        link_ref: &str,
        user_id: String,
        repo_root: Option<&Path>,
    ) -> Result<
        (
            RuntimeSession,
            WorkspaceLinkDefinition,
            Vec<WorkspaceLinkAttachment>,
        ),
        DaemonError,
    > {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let link_id = resolve_workspace_link_ref_in_session(session, link_ref)?
            .link_id()
            .to_string();
        let link =
            session
                .workspace_link_mut(&link_id)
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "detach workspace link",
                    message: format!("workspace link `{link_ref}` was not found"),
                })?;
        let detached = link.detach(&user_id, repo_root);
        let link = link.clone();
        session.touch();
        Ok((session.clone(), link, detached))
    }

    fn next_workspace_link_id(&mut self) -> String {
        self.next_workspace_link_number += 1;
        format!("workspace-link-{}", self.next_workspace_link_number)
    }

    pub fn create_session_invite(
        &mut self,
        session_id: &str,
        invite_id: String,
        created_by_user_id: String,
        expires_at_ms: Option<u64>,
        max_uses: Option<u32>,
        collaboration_level: CollaborationLevel,
    ) -> Result<(RuntimeSession, SessionInvite), DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        if !session.has_member(&created_by_user_id) {
            return Err(DaemonError::LocalTransport {
                operation: "create session invite",
                message: format!(
                    "user `{created_by_user_id}` is not a member of session `{session_id}`"
                ),
            });
        }
        let invite = SessionInvite::new(
            invite_id,
            session_id,
            created_by_user_id,
            unix_epoch_ms(),
            expires_at_ms,
            max_uses,
            collaboration_level,
        );
        let invite = session.add_invite(invite);
        session.touch();
        Ok((session.clone(), invite))
    }

    pub fn join_session_invite(
        &mut self,
        session_id: &str,
        invite_id: &str,
        user_id: String,
        now_ms: u64,
    ) -> Result<(RuntimeSession, SessionMember), DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let (invited_by_user_id, collaboration_level) = {
            let invite =
                session
                    .invite_mut(invite_id)
                    .ok_or_else(|| DaemonError::LocalTransport {
                        operation: "join session invite",
                        message: format!("session invite `{invite_id}` was not found"),
                    })?;
            if invite.session_id() != session_id {
                return Err(DaemonError::LocalTransport {
                    operation: "join session invite",
                    message: "session invite target does not match the local session".to_string(),
                });
            }
            if invite.is_revoked() {
                return Err(DaemonError::LocalTransport {
                    operation: "join session invite",
                    message: "session invite is revoked".to_string(),
                });
            }
            if invite.is_expired(now_ms) {
                return Err(DaemonError::LocalTransport {
                    operation: "join session invite",
                    message: "session invite is expired".to_string(),
                });
            }
            if invite.is_exhausted() {
                return Err(DaemonError::LocalTransport {
                    operation: "join session invite",
                    message: "session invite has no uses remaining".to_string(),
                });
            }
            invite.mark_used();
            (
                Some(invite.created_by_user_id().to_string()),
                invite.collaboration_level(),
            )
        };
        let member = session.add_member(user_id, invited_by_user_id, collaboration_level);
        session.touch();
        Ok((session.clone(), member))
    }

    pub fn revoke_session_invite(
        &mut self,
        session_id: &str,
        invite_id: &str,
    ) -> Result<(RuntimeSession, SessionInvite), DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let invite = session
            .invite_mut(invite_id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "revoke session invite",
                message: format!("session invite `{invite_id}` was not found"),
            })?;
        invite.revoke(unix_epoch_ms());
        let invite = invite.clone();
        session.touch();
        Ok((session.clone(), invite))
    }

    pub fn list_sessions(&self) -> Vec<RuntimeSession> {
        self.store.visible_non_ended_sessions().cloned().collect()
    }

    pub fn list_non_ended_sessions_including_hidden(&self) -> Vec<RuntimeSession> {
        self.store.non_ended_sessions().cloned().collect()
    }

    pub fn list_all_sessions(&self) -> Vec<RuntimeSession> {
        self.store.list()
    }

    pub fn list_workflows(&self, session_id: &str) -> Result<Vec<WorkflowDefinition>, DaemonError> {
        Ok(self.get_session(session_id)?.workflows().to_vec())
    }

    pub fn resolve_workflow_ref(
        &self,
        session_id: &str,
        workflow_ref: &str,
    ) -> Result<WorkflowDefinition, DaemonError> {
        let normalized_ref = workflow_ref.trim().to_lowercase();
        let session = self.get_session(session_id)?;
        let workflows = session.workflows();
        if let Some(workflow) = workflows
            .iter()
            .find(|workflow| workflow.id() == normalized_ref)
        {
            return Ok(workflow.clone());
        }
        if let Some(workflow) = workflows
            .iter()
            .find(|workflow| workflow.alias() == Some(normalized_ref.as_str()))
        {
            return Ok(workflow.clone());
        }
        let id_matches = workflows
            .iter()
            .filter(|workflow| workflow.id().starts_with(&normalized_ref))
            .cloned()
            .collect::<Vec<_>>();
        if id_matches.len() == 1 {
            return Ok(id_matches[0].clone());
        }
        let alias_matches = workflows
            .iter()
            .filter(|workflow| {
                workflow
                    .alias()
                    .is_some_and(|alias| alias.starts_with(normalized_ref.as_str()))
            })
            .cloned()
            .collect::<Vec<_>>();
        if alias_matches.len() == 1 {
            return Ok(alias_matches[0].clone());
        }
        Err(DaemonError::WorkflowNotFound {
            session_id: session_id.to_string(),
            workflow_id: workflow_ref.to_string(),
        })
    }

    pub fn create_workflow(
        &mut self,
        session_id: &str,
        alias: Option<String>,
    ) -> Result<WorkflowDefinition, DaemonError> {
        self.create_workflow_controlled_by_metaagent(session_id, alias, None)
    }

    pub fn create_workflow_controlled_by_metaagent(
        &mut self,
        session_id: &str,
        alias: Option<String>,
        controlled_by_metaagent_id: Option<String>,
    ) -> Result<WorkflowDefinition, DaemonError> {
        self.create_workflow_controlled_by_metaagent_with_alias_base(
            session_id,
            alias,
            "workflow",
            controlled_by_metaagent_id,
        )
    }

    pub(super) fn create_workflow_controlled_by_metaagent_with_alias_base(
        &mut self,
        session_id: &str,
        alias: Option<String>,
        default_base: &str,
        controlled_by_metaagent_id: Option<String>,
    ) -> Result<WorkflowDefinition, DaemonError> {
        let alias = self.workflow_alias_for_create(session_id, alias, default_base)?;
        let workflow = match controlled_by_metaagent_id {
            Some(metaagent_id) => WorkflowDefinition::new_controlled_by_metaagent(
                self.next_workflow_id(),
                alias,
                metaagent_id,
            ),
            None => WorkflowDefinition::new(self.next_workflow_id(), alias),
        };
        let workflow = workflow.with_max_concurrent(self.workflow_default_max_concurrent);
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        Ok(session.create_workflow(workflow))
    }

    pub(super) fn workflow_alias_for_create(
        &self,
        session_id: &str,
        alias: Option<String>,
        default_base: &str,
    ) -> Result<Option<String>, DaemonError> {
        let alias = match alias {
            Some(alias) if !alias.trim().is_empty() => normalize_workflow_alias(Some(alias))?,
            _ => Some(self.default_workflow_alias(session_id, default_base)?),
        };
        if let Some(alias) = alias.as_deref() {
            self.ensure_workflow_alias_available(session_id, alias)?;
        }
        Ok(alias)
    }

    fn default_workflow_alias(&self, session_id: &str, base: &str) -> Result<String, DaemonError> {
        let base = default_workflow_alias_base(base);
        let session = self
            .store
            .get(session_id)
            .ok_or_else(|| DaemonError::SessionNotFound {
                session_id: session_id.to_string(),
            })?;
        let mut number = session
            .workflows()
            .iter()
            .filter(|workflow| {
                workflow
                    .alias()
                    .is_some_and(|alias| workflow_alias_uses_base(alias, &base))
            })
            .count()
            + 1;
        loop {
            let alias = format!("{base}-{number}");
            if session
                .workflows()
                .iter()
                .all(|workflow| workflow.alias() != Some(alias.as_str()))
            {
                return Ok(alias);
            }
            number += 1;
        }
    }

    pub fn assign_workflow_alias(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        alias: String,
    ) -> Result<WorkflowDefinition, DaemonError> {
        let workflow_id = self
            .resolve_workflow_ref(session_id, workflow_ref)?
            .id()
            .to_string();
        let alias = normalize_workflow_alias(Some(alias))?.ok_or_else(|| {
            DaemonError::InvalidWorkflowAlias {
                alias: String::new(),
                message: "alias cannot be empty",
            }
        })?;
        self.ensure_workflow_alias_available_for_update(session_id, &workflow_id, &alias)?;
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow =
            session
                .workflow_mut(&workflow_id)
                .ok_or_else(|| DaemonError::WorkflowNotFound {
                    session_id: session_id.to_string(),
                    workflow_id: workflow_id.to_string(),
                })?;
        workflow.set_alias(Some(alias));
        Ok(workflow.clone())
    }

    pub fn set_workflow_flush_agent_context_before_run(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        value: bool,
    ) -> Result<WorkflowDefinition, DaemonError> {
        let workflow_id = self
            .resolve_workflow_ref(session_id, workflow_ref)?
            .id()
            .to_string();
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow =
            session
                .workflow_mut(&workflow_id)
                .ok_or_else(|| DaemonError::WorkflowNotFound {
                    session_id: session_id.to_string(),
                    workflow_id: workflow_id.clone(),
                })?;
        workflow.set_flush_agent_context_before_run(value);
        Ok(workflow.clone())
    }

    pub fn set_workflow_run_output_schema_ref(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        value: Option<String>,
    ) -> Result<WorkflowDefinition, DaemonError> {
        let workflow_id = self
            .resolve_workflow_ref(session_id, workflow_ref)?
            .id()
            .to_string();
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow =
            session
                .workflow_mut(&workflow_id)
                .ok_or_else(|| DaemonError::WorkflowNotFound {
                    session_id: session_id.to_string(),
                    workflow_id: workflow_id.clone(),
                })?;
        workflow.set_run_output_schema_ref(value);
        Ok(workflow.clone())
    }

    pub fn assign_session_alias(
        &mut self,
        session_id: &str,
        alias: String,
    ) -> Result<RuntimeSession, DaemonError> {
        let alias = normalize_session_alias(Some(alias))?.ok_or_else(|| {
            DaemonError::InvalidSessionAlias {
                alias: String::new(),
                message: "alias cannot be empty",
            }
        })?;

        let session = self.get_session(session_id)?;
        self.ensure_session_alias_available_for_update(
            session.workspace_id(),
            session.id(),
            &alias,
        )?;

        let session = self.get_session_mut_for_operation(session_id, "assign alias")?;
        session.set_alias(Some(alias));
        Ok(session.clone())
    }

    pub fn resolve_workflow_endpoint_ref(
        &self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
    ) -> Result<WorkflowEndpointDefinition, DaemonError> {
        let workflow = self.resolve_workflow_ref(session_id, workflow_ref)?;
        let normalized_ref = endpoint_ref.trim().to_lowercase();
        if let Some(endpoint) = workflow
            .endpoints()
            .iter()
            .find(|endpoint| endpoint.id() == normalized_ref)
        {
            return Ok(endpoint.clone());
        }
        if let Some(endpoint) = workflow
            .endpoints()
            .iter()
            .find(|endpoint| endpoint.alias() == Some(normalized_ref.as_str()))
        {
            return Ok(endpoint.clone());
        }
        let id_matches = workflow
            .endpoints()
            .iter()
            .filter(|endpoint| endpoint.id().starts_with(&normalized_ref))
            .cloned()
            .collect::<Vec<_>>();
        if id_matches.len() == 1 {
            return Ok(id_matches[0].clone());
        }
        let alias_matches = workflow
            .endpoints()
            .iter()
            .filter(|endpoint| {
                endpoint
                    .alias()
                    .is_some_and(|alias| alias.starts_with(normalized_ref.as_str()))
            })
            .cloned()
            .collect::<Vec<_>>();
        if alias_matches.len() == 1 {
            return Ok(alias_matches[0].clone());
        }
        Err(DaemonError::WorkflowEndpointNotFound {
            session_id: session_id.to_string(),
            workflow_id: workflow.id().to_string(),
            endpoint_id: endpoint_ref.to_string(),
        })
    }

    pub fn list_workflow_runs(
        &self,
        session_id: &str,
        workflow_ref: Option<&str>,
    ) -> Result<Vec<WorkflowRun>, DaemonError> {
        let workflow_id = workflow_ref
            .map(|reference| self.resolve_workflow_ref(session_id, reference))
            .transpose()?
            .map(|workflow| workflow.id().to_string());
        let session = self.get_session(session_id)?;
        Ok(session
            .workflow_runs()
            .iter()
            .filter(|workflow_run| {
                workflow_id
                    .as_deref()
                    .is_none_or(|id| workflow_run.workflow_id() == id)
            })
            .cloned()
            .collect())
    }

    pub fn resolve_workflow_run_ref(
        &self,
        session_id: &str,
        workflow_run_ref: &str,
    ) -> Result<WorkflowRun, DaemonError> {
        let normalized_ref = workflow_run_ref.trim().to_lowercase();
        let session = self.get_session(session_id)?;
        let workflow_runs = session.workflow_runs();
        if let Some(workflow_run) = workflow_runs
            .iter()
            .find(|workflow_run| workflow_run.id() == normalized_ref)
        {
            return Ok(workflow_run.clone());
        }
        let id_matches = workflow_runs
            .iter()
            .filter(|workflow_run| workflow_run.id().starts_with(&normalized_ref))
            .cloned()
            .collect::<Vec<_>>();
        if id_matches.len() == 1 {
            return Ok(id_matches[0].clone());
        }
        Err(DaemonError::WorkflowRunNotFound {
            session_id: session_id.to_string(),
            workflow_run_id: workflow_run_ref.to_string(),
        })
    }

    pub fn resolve_workflow_prompt_queue_ref(
        &self,
        session_id: &str,
        workflow_id: &str,
        queue_ref: &str,
    ) -> Result<String, DaemonError> {
        let normalized_ref = queue_ref.trim().to_lowercase();
        let session = self.get_session(session_id)?;
        if let Some(queue) = session.workflow_prompt_queues().iter().find(|queue| {
            queue.workflow_id() == workflow_id
                && (queue.id() == normalized_ref || queue.alias() == normalized_ref)
        }) {
            return Ok(queue.id().to_string());
        }
        let matches = session
            .workflow_prompt_queues()
            .iter()
            .filter(|queue| {
                queue.workflow_id() == workflow_id
                    && (queue.id().starts_with(&normalized_ref)
                        || queue.alias().starts_with(&normalized_ref))
            })
            .map(|queue| queue.id().to_string())
            .collect::<Vec<_>>();
        if matches.len() == 1 {
            return Ok(matches[0].clone());
        }
        Err(DaemonError::InvalidWorkflowGraphReference {
            session_id: session_id.to_string(),
            workflow_id: workflow_id.to_string(),
            reference: queue_ref.to_string(),
            message: "workflow prompt queue was not found",
        })
    }

    pub fn resolve_queued_workflow_prompt_ref(
        &self,
        session_id: &str,
        queue_item_ref: &str,
    ) -> Result<String, DaemonError> {
        let normalized_ref = queue_item_ref.trim().to_lowercase();
        let session = self.get_session(session_id)?;
        if let Some(queued_prompt) = session
            .workflow_queued_prompts()
            .iter()
            .find(|queued_prompt| queued_prompt.id() == normalized_ref)
        {
            return Ok(queued_prompt.id().to_string());
        }
        let id_matches = session
            .workflow_queued_prompts()
            .iter()
            .filter(|queued_prompt| queued_prompt.id().starts_with(&normalized_ref))
            .map(|queued_prompt| queued_prompt.id().to_string())
            .collect::<Vec<_>>();
        if id_matches.len() == 1 {
            return Ok(id_matches[0].clone());
        }
        Err(DaemonError::InvalidWorkflowGraphReference {
            session_id: session_id.to_string(),
            workflow_id: normalized_ref.clone(),
            reference: queue_item_ref.to_string(),
            message: "queued workflow prompt was not found",
        })
    }
}

fn default_session_alias_base(workspace_id: &str) -> String {
    let repo_name = Path::new(workspace_id)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(workspace_id);
    default_alias_base(repo_name, "workspace")
}

fn default_workflow_alias_base(base: &str) -> String {
    default_alias_base(base, "workflow")
}

fn default_alias_base(input: &str, fallback: &str) -> String {
    let mut base = String::new();
    let mut previous_separator = false;
    for char in input.trim().to_lowercase().chars() {
        if char.is_ascii_lowercase() || char.is_ascii_digit() || char == '_' {
            base.push(char);
            previous_separator = false;
        } else if char == '-' || char.is_ascii_whitespace() {
            if !previous_separator && !base.is_empty() {
                base.push('-');
                previous_separator = true;
            }
        } else if !previous_separator && !base.is_empty() {
            base.push('-');
            previous_separator = true;
        }
    }
    while base.ends_with('-') {
        base.pop();
    }
    if base.is_empty() {
        fallback.to_string()
    } else {
        base
    }
}

fn workflow_alias_uses_base(alias: &str, base: &str) -> bool {
    let Some(suffix) = alias.strip_prefix(base) else {
        return false;
    };
    suffix.strip_prefix('-').is_some_and(|number| {
        !number.is_empty() && number.chars().all(|char| char.is_ascii_digit())
    })
}

fn normalize_workspace_link_name(name: &str) -> Result<String, DaemonError> {
    let normalized = name.trim().to_lowercase();
    if normalized.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "create workspace link",
            message: "workspace link name cannot be empty".to_string(),
        });
    }
    if !normalized
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.')
    {
        return Err(DaemonError::LocalTransport {
            operation: "create workspace link",
            message: "workspace link name may only contain letters, numbers, '-', '_' or '.'"
                .to_string(),
        });
    }
    Ok(normalized)
}

fn ensure_workspace_link_name_available(
    session: &RuntimeSession,
    name: &str,
) -> Result<(), DaemonError> {
    if session
        .workspace_links()
        .iter()
        .any(|link| link.name() == name)
    {
        Err(DaemonError::LocalTransport {
            operation: "create workspace link",
            message: format!("workspace link `{name}` already exists"),
        })
    } else {
        Ok(())
    }
}

fn resolve_workspace_link_ref_in_session<'a>(
    session: &'a RuntimeSession,
    link_ref: &str,
) -> Result<&'a WorkspaceLinkDefinition, DaemonError> {
    let normalized_ref = link_ref.trim().to_lowercase();
    if normalized_ref.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "resolve workspace link",
            message: "workspace link reference cannot be empty".to_string(),
        });
    }
    if let Some(link) = session
        .workspace_links()
        .iter()
        .find(|link| link.link_id() == normalized_ref || link.name() == normalized_ref)
    {
        return Ok(link);
    }
    let matches = session
        .workspace_links()
        .iter()
        .filter(|link| {
            link.link_id().starts_with(&normalized_ref) || link.name().starts_with(&normalized_ref)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [link] => Ok(*link),
        [] => Err(DaemonError::LocalTransport {
            operation: "resolve workspace link",
            message: format!("workspace link `{normalized_ref}` was not found"),
        }),
        _ => Err(DaemonError::LocalTransport {
            operation: "resolve workspace link",
            message: format!("workspace link `{normalized_ref}` is ambiguous"),
        }),
    }
}
