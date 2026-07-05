use super::*;
use crate::session::{WORKFLOW_PUBLICATION_KIND_INGRESS, WORKFLOW_PUBLICATION_KIND_SCHEDULE_ONLY};
use std::path::Path;

impl SessionService {
    pub fn new(config: &DaemonConfig) -> Self {
        Self {
            store: SessionStore::new(),
            host_machine_id: config.host_machine_id.clone(),
            host_daemon_id: config.daemon_id.clone(),
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
            next_workflow_prompt_queue_number: 0,
            next_workflow_queued_prompt_number: 0,
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
        Ok(session.create_workflow_publication(publication))
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

    #[allow(clippy::too_many_arguments)]
    pub fn create_workflow_publication(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        queue_ref: Option<String>,
        alias: Option<String>,
        kind: Option<String>,
        route: Option<String>,
        methods: Vec<String>,
        transport: Option<Value>,
        parser: Option<Value>,
        input_schema: Option<Value>,
        trace_exposure: Option<Value>,
        mode: Option<String>,
        sync_timeout_ms: Option<u64>,
        poll_ms: Option<u64>,
        created_by_user_id: String,
    ) -> Result<WorkflowPublicationDefinition, DaemonError> {
        let workflow = self.resolve_workflow_ref(session_id, workflow_ref)?;
        let endpoint =
            self.resolve_workflow_endpoint_ref(session_id, workflow.id(), endpoint_ref)?;
        validate_workflow_publication_trace_exposure(&trace_exposure, &workflow)?;
        let publication_kind = resolve_workflow_publication_kind(kind.as_deref(), &transport)?;
        validate_workflow_publication_options(
            &publication_kind,
            &transport,
            route.as_deref(),
            &methods,
            &parser,
            mode.as_deref(),
        )?;
        let normalized_queue_ref = normalize_workflow_publication_queue_ref(queue_ref);
        self.resolve_workflow_prompt_queue_ref(session_id, workflow.id(), &normalized_queue_ref)?;
        if publication_kind == WORKFLOW_PUBLICATION_KIND_SCHEDULE_ONLY {
            self.validate_schedule_only_workflow_publication(
                session_id,
                workflow.id(),
                endpoint.id(),
                &normalized_queue_ref,
            )?;
        }
        let alias = normalize_workflow_publication_alias(alias)?;
        if let Some(alias) = alias.as_deref() {
            self.ensure_workflow_publication_alias_available(session_id, alias)?;
        }
        let publication = WorkflowPublicationDefinition::new(
            self.next_workflow_publication_id(),
            session_id.to_string(),
            workflow.id().to_string(),
            endpoint.id().to_string(),
            Some(normalized_queue_ref),
            alias,
            publication_kind,
            route,
            methods,
            transport,
            parser,
            input_schema,
            trace_exposure,
            mode,
            sync_timeout_ms,
            poll_ms,
            created_by_user_id,
        );
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        Ok(session.create_workflow_publication(publication))
    }

    fn validate_schedule_only_workflow_publication(
        &self,
        session_id: &str,
        workflow_id: &str,
        endpoint_id: &str,
        queue_ref: &str,
    ) -> Result<(), DaemonError> {
        let session = self.get_session(session_id)?;
        let queue_id =
            self.resolve_workflow_prompt_queue_ref(session_id, workflow_id, queue_ref)?;
        let has_schedule = session.workflow_schedules().iter().any(|schedule| {
            let schedule_queue_matches = match schedule.queue_id() {
                Some(schedule_queue_id) => schedule_queue_id == queue_id.as_str(),
                None => queue_id == "default",
            };
            schedule.workflow_id() == workflow_id
                && schedule.endpoint_id() == endpoint_id
                && schedule.enabled()
                && schedule_queue_matches
        });
        if has_schedule {
            return Ok(());
        }
        invalid_workflow_publication_option(
            "schedule_only publications require an enabled schedule for the selected endpoint and queue",
        )
    }

    pub fn list_workflow_publications(
        &self,
        session_id: &str,
    ) -> Result<Vec<WorkflowPublicationDefinition>, DaemonError> {
        Ok(self
            .get_session(session_id)?
            .workflow_publications()
            .to_vec())
    }

    pub fn resolve_workflow_publication_ref(
        &self,
        session_id: &str,
        publication_ref: &str,
    ) -> Result<WorkflowPublicationDefinition, DaemonError> {
        let normalized_ref = publication_ref.trim().to_lowercase();
        let session = self.get_session(session_id)?;
        let publications = session.workflow_publications();
        if let Some(publication) = publications
            .iter()
            .find(|publication| publication.id() == normalized_ref)
        {
            return Ok(publication.clone());
        }
        if let Some(publication) = publications
            .iter()
            .find(|publication| publication.alias() == Some(normalized_ref.as_str()))
        {
            return Ok(publication.clone());
        }
        let id_matches = publications
            .iter()
            .filter(|publication| publication.id().starts_with(&normalized_ref))
            .cloned()
            .collect::<Vec<_>>();
        if id_matches.len() == 1 {
            return Ok(id_matches[0].clone());
        }
        let alias_matches = publications
            .iter()
            .filter(|publication| {
                publication
                    .alias()
                    .is_some_and(|alias| alias.starts_with(normalized_ref.as_str()))
            })
            .cloned()
            .collect::<Vec<_>>();
        if alias_matches.len() == 1 {
            return Ok(alias_matches[0].clone());
        }
        Err(DaemonError::LocalTransport {
            operation: "resolve workflow publication",
            message: format!("workflow publication `{publication_ref}` was not found"),
        })
    }

    pub fn disable_workflow_publication(
        &mut self,
        session_id: &str,
        publication_ref: &str,
    ) -> Result<WorkflowPublicationDefinition, DaemonError> {
        let publication_id = self
            .resolve_workflow_publication_ref(session_id, publication_ref)?
            .id()
            .to_string();
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let publication = session
            .workflow_publication_mut(&publication_id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "disable workflow publication",
                message: format!("workflow publication `{publication_ref}` was not found"),
            })?;
        publication.disable();
        Ok(publication.clone())
    }

    pub fn register_workflow_publication_endpoint(
        &mut self,
        session_id: &str,
        publication_ref: &str,
        status: impl Into<String>,
        open_url: impl Into<String>,
        deployment: Value,
    ) -> Result<WorkflowPublicationDefinition, DaemonError> {
        let publication_id = self
            .resolve_workflow_publication_ref(session_id, publication_ref)?
            .id()
            .to_string();
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let publication = session
            .workflow_publication_mut(&publication_id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "register workflow publication endpoint",
                message: format!("workflow publication `{publication_ref}` was not found"),
            })?;
        publication.mark_served(status, open_url, deployment);
        Ok(publication.clone())
    }

    pub fn mark_workflow_publication_runtime_status(
        &mut self,
        session_id: &str,
        publication_ref: &str,
        status: impl Into<String>,
        open_url: Option<Option<String>>,
        deployment: Option<Value>,
    ) -> Result<WorkflowPublicationDefinition, DaemonError> {
        let status = status.into();
        let publication_id = self
            .resolve_workflow_publication_ref(session_id, publication_ref)?
            .id()
            .to_string();
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let runtime_observability =
            session
                .workflow_publication(&publication_id)
                .map(|publication| {
                    workflow_publication_runtime_observability(
                        session,
                        publication,
                        runtime_reachability_for_status(&status),
                    )
                });
        let publication = session
            .workflow_publication_mut(&publication_id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "mark workflow publication runtime status",
                message: format!("workflow publication `{publication_ref}` was not found"),
            })?;
        publication.mark_runtime_status(status, open_url, deployment);
        if let Some(runtime_observability) = runtime_observability {
            publication.set_runtime_observability(
                runtime_observability.runtime,
                runtime_observability.schedules,
                runtime_observability.latest_run,
                runtime_observability.recent_runs,
                runtime_observability.latest_output,
            );
        }
        Ok(publication.clone())
    }

    pub fn mark_workflow_publication_runtime_error(
        &mut self,
        session_id: &str,
        publication_ref: &str,
        message: impl Into<String>,
    ) -> Result<WorkflowPublicationDefinition, DaemonError> {
        let message = message.into();
        let publication_id = self
            .resolve_workflow_publication_ref(session_id, publication_ref)?
            .id()
            .to_string();
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let runtime_observability =
            session
                .workflow_publication(&publication_id)
                .map(|publication| {
                    workflow_publication_runtime_observability(
                        session,
                        publication,
                        Some(serde_json::json!({
                            "reachable": false,
                            "error": message.clone(),
                        })),
                    )
                });
        let publication = session
            .workflow_publication_mut(&publication_id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "mark workflow publication runtime error",
                message: format!("workflow publication `{publication_ref}` was not found"),
            })?;
        publication.mark_runtime_error(message);
        if let Some(runtime_observability) = runtime_observability {
            publication.set_runtime_observability(
                runtime_observability.runtime,
                runtime_observability.schedules,
                runtime_observability.latest_run,
                runtime_observability.recent_runs,
                runtime_observability.latest_output,
            );
        }
        Ok(publication.clone())
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

fn validate_workflow_publication_trace_exposure(
    trace_exposure: &Option<Value>,
    workflow: &WorkflowDefinition,
) -> Result<(), DaemonError> {
    let Some(value) = trace_exposure else {
        return Ok(());
    };
    let Some(object) = value.as_object() else {
        return Err(DaemonError::InvalidWorkflowPublicationTraceExposure {
            message: "`trace_exposure` must be an object".to_string(),
        });
    };
    let Some(nodes_value) = object.get("nodes") else {
        return Ok(());
    };
    let Some(nodes_object) = nodes_value.as_object() else {
        return Err(DaemonError::InvalidWorkflowPublicationTraceExposure {
            message: "`trace_exposure.nodes` must be an object keyed by workflow node id"
                .to_string(),
        });
    };
    let known_node_ids = workflow
        .nodes()
        .iter()
        .map(|node| node.id())
        .collect::<std::collections::BTreeSet<_>>();
    for (node_id, levels_value) in nodes_object {
        if !known_node_ids.contains(node_id.as_str()) {
            return Err(DaemonError::InvalidWorkflowPublicationTraceExposure {
                message: format!("unknown workflow node id `{node_id}`"),
            });
        }
        let Some(levels) = levels_value.as_array() else {
            return Err(DaemonError::InvalidWorkflowPublicationTraceExposure {
                message: format!("trace levels for node `{node_id}` must be an array"),
            });
        };
        for level in levels {
            let Some(level) = level.as_str() else {
                return Err(DaemonError::InvalidWorkflowPublicationTraceExposure {
                    message: format!("trace level for node `{node_id}` must be a string"),
                });
            };
            if !matches!(
                level,
                "output_summary" | "assistant_messages" | "thinking" | "tool_use"
            ) {
                return Err(DaemonError::InvalidWorkflowPublicationTraceExposure {
                    message: format!("unknown trace exposure level `{level}` for node `{node_id}`"),
                });
            }
        }
    }
    Ok(())
}

struct WorkflowPublicationRuntimeObservability {
    runtime: Option<Value>,
    schedules: Vec<Value>,
    latest_run: Option<Value>,
    recent_runs: Vec<Value>,
    latest_output: Option<Value>,
}

fn workflow_publication_runtime_observability(
    session: &RuntimeSession,
    publication: &WorkflowPublicationDefinition,
    runtime: Option<Value>,
) -> WorkflowPublicationRuntimeObservability {
    let queue_refs = workflow_publication_queue_reference_set(session, publication);
    let mut schedules = session
        .workflow_schedules()
        .iter()
        .filter(|schedule| {
            if schedule.workflow_id() != publication.workflow_id()
                || schedule.endpoint_id() != publication.endpoint_id()
            {
                return false;
            }
            schedule
                .queue_id()
                .is_none_or(|queue_id| queue_refs.contains(queue_id))
        })
        .filter_map(|schedule| serde_json::to_value(schedule).ok())
        .collect::<Vec<_>>();
    schedules.sort_by_key(|schedule| {
        schedule
            .get("next_run_at_ms")
            .and_then(Value::as_u64)
            .unwrap_or(u64::MAX)
    });

    let mut runs = session
        .workflow_runs()
        .iter()
        .filter(|run| workflow_run_matches_publication(run, publication))
        .cloned()
        .collect::<Vec<_>>();
    runs.sort_by_key(|run| std::cmp::Reverse(workflow_run_sort_time(run)));

    let latest_run = runs.first().and_then(|run| serde_json::to_value(run).ok());
    let recent_runs = runs
        .iter()
        .take(5)
        .filter_map(|run| serde_json::to_value(run).ok())
        .collect::<Vec<_>>();
    let latest_output = runs.iter().find_map(workflow_run_latest_output_value);

    WorkflowPublicationRuntimeObservability {
        runtime,
        schedules,
        latest_run,
        recent_runs,
        latest_output,
    }
}

fn workflow_publication_queue_reference_set(
    session: &RuntimeSession,
    publication: &WorkflowPublicationDefinition,
) -> BTreeSet<String> {
    let mut refs = BTreeSet::new();
    let queue_ref = publication.queue_ref().unwrap_or("default").trim();
    let queue_ref = if queue_ref.is_empty() {
        "default"
    } else {
        queue_ref
    };
    refs.insert(queue_ref.to_string());
    if let Some(queue) = session.workflow_prompt_queues().iter().find(|candidate| {
        candidate.workflow_id() == publication.workflow_id()
            && (candidate.id() == queue_ref || candidate.alias() == queue_ref)
    }) {
        refs.insert(queue.id().to_string());
        refs.insert(queue.alias().to_string());
    }
    refs
}

fn workflow_run_matches_publication(
    run: &WorkflowRun,
    publication: &WorkflowPublicationDefinition,
) -> bool {
    if let Some(invocation) = run.publication_invocation() {
        return invocation.publication_id == publication.id();
    }
    if run.workflow_id() != publication.workflow_id()
        || run.endpoint_id() != publication.endpoint_id()
    {
        return false;
    }
    true
}

fn workflow_run_sort_time(run: &WorkflowRun) -> u64 {
    let Ok(value) = serde_json::to_value(run) else {
        return 0;
    };
    ["completed_at_ms", "started_at_ms", "created_at_ms"]
        .iter()
        .find_map(|field| value.get(*field).and_then(Value::as_u64))
        .unwrap_or(0)
}

fn workflow_run_latest_output_value(run: &WorkflowRun) -> Option<Value> {
    if let Some(output) = run.final_output() {
        return Some(serde_json::json!({
            "kind": "final",
            "message": serde_json::to_value(output).ok()?,
            "artifacts": [],
        }));
    }
    run.intermediate_outputs()
        .iter()
        .max_by_key(|output| {
            serde_json::to_value(output)
                .ok()
                .and_then(|value| value.get("timestamp_ms").and_then(Value::as_u64))
                .unwrap_or(0)
        })
        .and_then(|output| {
            let output_value = serde_json::to_value(output).ok()?;
            Some(serde_json::json!({
                "kind": "partial",
                "message": output_value.get("output").cloned().unwrap_or(Value::Null),
                "artifacts": [],
                "intermediate_output_id": output_value.get("id").cloned().unwrap_or(Value::Null),
            }))
        })
}

fn runtime_reachability_for_status(status: &str) -> Option<Value> {
    match status {
        "starting" | "ready" | "running" => Some(serde_json::json!({ "reachable": true })),
        "error" => Some(serde_json::json!({ "reachable": false })),
        _ => None,
    }
}

impl SessionService {
    fn ensure_workflow_publication_alias_available(
        &self,
        session_id: &str,
        alias: &str,
    ) -> Result<(), DaemonError> {
        if self
            .get_session(session_id)?
            .workflow_publications()
            .iter()
            .any(|publication| publication.alias() == Some(alias))
        {
            Err(DaemonError::LocalTransport {
                operation: "create workflow publication",
                message: format!("workflow publication alias `{alias}` is already in use"),
            })
        } else {
            Ok(())
        }
    }
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

fn normalize_workflow_publication_queue_ref(queue_ref: Option<String>) -> String {
    queue_ref
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("default")
        .to_string()
}

fn validate_workflow_publication_options(
    publication_kind: &str,
    transport: &Option<serde_json::Value>,
    route: Option<&str>,
    methods: &[String],
    parser: &Option<serde_json::Value>,
    mode: Option<&str>,
) -> Result<(), DaemonError> {
    validate_workflow_publication_mode(mode)?;
    validate_workflow_publication_route(route)?;
    if publication_kind == WORKFLOW_PUBLICATION_KIND_SCHEDULE_ONLY {
        if route.is_some_and(|route| !route.trim().is_empty()) {
            return invalid_workflow_publication_option(
                "schedule_only publications do not expose an ingress route",
            );
        }
        if !methods.is_empty() {
            return invalid_workflow_publication_option(
                "schedule_only publications do not support HTTP method overrides",
            );
        }
        if parser.is_some() {
            return invalid_workflow_publication_option(
                "schedule_only publications do not parse external request input",
            );
        }
        if mode.is_some() {
            return invalid_workflow_publication_option(
                "schedule_only publications do not support response mode overrides",
            );
        }
        let transport_kind = workflow_publication_transport_kind(transport)?;
        if transport.is_some() && transport_kind != WORKFLOW_PUBLICATION_KIND_SCHEDULE_ONLY {
            return invalid_workflow_publication_option(
                "schedule_only publications must not configure an ingress transport",
            );
        }
        return Ok(());
    }

    let kind = workflow_publication_transport_kind(transport)?;
    if kind == WORKFLOW_PUBLICATION_KIND_SCHEDULE_ONLY {
        return invalid_workflow_publication_option(
            "ingress publications must use an ingress transport",
        );
    }
    match kind.as_str() {
        "human_http" => {
            validate_workflow_publication_methods(&kind, methods, &["GET", "POST"])?;
            validate_human_http_publication_parser(parser)?;
        }
        "api_sse_json" => {
            validate_workflow_publication_methods(&kind, methods, &["POST"])?;
            validate_json_publication_parser(&kind, parser)?;
            if let Some(mode) = mode {
                if mode != "async" {
                    return invalid_workflow_publication_option(
                        "api_sse_json publications always use async response streaming",
                    );
                }
            }
        }
        "websocket_json" => {
            if !methods.is_empty() {
                return invalid_workflow_publication_option(
                    "websocket_json publications do not support HTTP method overrides",
                );
            }
            if parser.is_some() {
                return invalid_workflow_publication_option(
                    "websocket_json publications read input from WebSocket invoke messages",
                );
            }
            if let Some(mode) = mode {
                if mode != "async" {
                    return invalid_workflow_publication_option(
                        "websocket_json publications always use async event streaming",
                    );
                }
            }
        }
        "mcp" => {
            validate_workflow_publication_methods(&kind, methods, &["POST"])?;
            if parser.is_some() {
                return invalid_workflow_publication_option(
                    "mcp publications read input from MCP tool arguments",
                );
            }
            if let Some(mode) = mode {
                if mode != "sync" {
                    return invalid_workflow_publication_option(
                        "mcp publications always return a synchronous tool result",
                    );
                }
            }
        }
        _ => {
            return invalid_workflow_publication_option(&format!(
                "unsupported workflow publication transport `{kind}`"
            ));
        }
    }
    Ok(())
}

fn resolve_workflow_publication_kind(
    kind: Option<&str>,
    transport: &Option<serde_json::Value>,
) -> Result<String, DaemonError> {
    let inferred = || -> Result<String, DaemonError> {
        let transport_kind = workflow_publication_transport_kind(transport)?;
        if transport_kind == WORKFLOW_PUBLICATION_KIND_SCHEDULE_ONLY {
            Ok(WORKFLOW_PUBLICATION_KIND_SCHEDULE_ONLY.to_string())
        } else {
            Ok(WORKFLOW_PUBLICATION_KIND_INGRESS.to_string())
        }
    };
    let Some(kind) = kind.map(str::trim).filter(|value| !value.is_empty()) else {
        return inferred();
    };
    match kind {
        WORKFLOW_PUBLICATION_KIND_INGRESS | WORKFLOW_PUBLICATION_KIND_SCHEDULE_ONLY => {
            Ok(kind.to_string())
        }
        _ => invalid_workflow_publication_option(&format!(
            "unsupported workflow publication kind `{kind}`"
        )),
    }
}

fn validate_workflow_publication_route(route: Option<&str>) -> Result<(), DaemonError> {
    let Some(route) = route.map(str::trim).filter(|route| !route.is_empty()) else {
        return Ok(());
    };
    if route.starts_with('/') {
        return Ok(());
    }
    invalid_workflow_publication_option("workflow publication route must start with `/`")
}

fn workflow_publication_transport_kind(
    transport: &Option<serde_json::Value>,
) -> Result<String, DaemonError> {
    let Some(transport) = transport else {
        return Ok("human_http".to_string());
    };
    if let Some(kind) = transport.get("kind").and_then(|value| value.as_str()) {
        return Ok(kind.to_string());
    }
    if let Some(kind) = transport.as_str() {
        return Ok(kind.to_string());
    }
    invalid_workflow_publication_option(
        "workflow publication transport must be a string or { kind }",
    )
}

fn validate_workflow_publication_mode(mode: Option<&str>) -> Result<(), DaemonError> {
    match mode {
        Some("sync" | "async") | None => Ok(()),
        Some(mode) => invalid_workflow_publication_option(&format!(
            "unsupported workflow publication mode `{mode}`"
        )),
    }
}

fn validate_workflow_publication_methods(
    transport: &str,
    methods: &[String],
    allowed: &[&str],
) -> Result<(), DaemonError> {
    for method in methods {
        if !allowed.iter().any(|allowed| *allowed == method) {
            return invalid_workflow_publication_option(&format!(
                "{transport} publications do not support HTTP method `{method}`"
            ));
        }
    }
    Ok(())
}

fn validate_json_publication_parser(
    transport: &str,
    parser: &Option<serde_json::Value>,
) -> Result<(), DaemonError> {
    let Some(parser) = parser else {
        return Ok(());
    };
    if parser.get("kind").and_then(|value| value.as_str()) == Some("json") {
        return Ok(());
    }
    invalid_workflow_publication_option(&format!(
        "{transport} publications only support JSON body input"
    ))
}

fn validate_human_http_publication_parser(
    parser: &Option<serde_json::Value>,
) -> Result<(), DaemonError> {
    let Some(parser) = parser else {
        return Ok(());
    };
    let Some(kind) = parser.get("kind").and_then(|value| value.as_str()) else {
        return invalid_workflow_publication_option(
            "human_http publication parser must be an object with a supported kind",
        );
    };
    if matches!(kind, "path_template" | "json" | "query_params" | "webhook") {
        return Ok(());
    }
    invalid_workflow_publication_option(&format!(
        "human_http publications do not support parser `{kind}`"
    ))
}

fn invalid_workflow_publication_option<T>(message: &str) -> Result<T, DaemonError> {
    Err(DaemonError::LocalTransport {
        operation: "create workflow publication",
        message: message.to_string(),
    })
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
