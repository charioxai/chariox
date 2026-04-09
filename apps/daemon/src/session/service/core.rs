use super::*;

impl SessionService {
    pub fn new(config: &DaemonConfig) -> Self {
        Self {
            store: SessionStore::new(),
            host_machine_id: config.host_machine_id.clone(),
            host_daemon_id: config.daemon_id.clone(),
            next_prompt_number: 0,
            next_workflow_number: 0,
            next_workflow_endpoint_number: 0,
            next_workflow_node_number: 0,
            next_workflow_edge_number: 0,
            next_workflow_run_number: 0,
            next_workflow_node_run_number: 0,
            next_workflow_message_number: 0,
            next_workflow_watchdog_number: 0,
            next_queued_workflow_launch_number: 0,
        }
    }

    pub fn create_session(
        &mut self,
        request: CreateSessionRequest,
    ) -> Result<RuntimeSession, DaemonError> {
        let alias = normalize_session_alias(request.alias)?;
        if let Some(alias) = alias.as_deref() {
            self.ensure_alias_available(&request.workspace_id, alias)?;
        }
        let session = RuntimeSession::new(
            self.store.next_session_id(),
            alias,
            request.workspace_id,
            request.worktree_id,
            self.host_machine_id.clone(),
            self.host_daemon_id.clone(),
        );

        Ok(self.store.insert(session))
    }

    pub fn get_session(&self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        self.store
            .get(session_id)
            .cloned()
            .ok_or_else(|| DaemonError::SessionNotFound {
                session_id: session_id.to_string(),
            })
    }

    pub fn list_sessions(&self) -> Vec<RuntimeSession> {
        self.store.non_ended_sessions().cloned().collect()
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
        let alias = normalize_workflow_alias(alias)?;
        if let Some(alias) = alias.as_deref() {
            self.ensure_workflow_alias_available(session_id, alias)?;
        }
        let workflow = WorkflowDefinition::new(self.next_workflow_id(), alias);
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        Ok(session.create_workflow(workflow))
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

    pub fn set_workflow_intermediate_output_schema_ref(
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
        workflow.set_intermediate_output_schema_ref(value);
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

    pub fn resolve_queued_workflow_launch_ref(
        &self,
        session_id: &str,
        queue_item_ref: &str,
    ) -> Result<String, DaemonError> {
        let normalized_ref = queue_item_ref.trim().to_lowercase();
        let session = self.get_session(session_id)?;
        if let Some(queued_launch) = session
            .queued_workflow_launches()
            .iter()
            .find(|queued_launch| queued_launch.id() == normalized_ref)
        {
            return Ok(queued_launch.id().to_string());
        }
        let id_matches = session
            .queued_workflow_launches()
            .iter()
            .filter(|queued_launch| queued_launch.id().starts_with(&normalized_ref))
            .map(|queued_launch| queued_launch.id().to_string())
            .collect::<Vec<_>>();
        if id_matches.len() == 1 {
            return Ok(id_matches[0].clone());
        }
        Err(DaemonError::InvalidWorkflowGraphReference {
            session_id: session_id.to_string(),
            workflow_id: normalized_ref.clone(),
            reference: queue_item_ref.to_string(),
            message: "queued workflow launch was not found",
        })
    }
}
