use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::local::{
    AddWorkflowEdgeRequest, AddWorkflowNodeRequest, AliasWorkflowEndpointRequest,
    AliasWorkflowRequest, BindWorkflowEndpointRequest, CreateWorkflowEndpointRequest,
    CreateWorkflowRequest, ListWorkflowsRequest, LocalDaemonResponse, RemoveWorkflowEdgeRequest,
    RemoveWorkflowNodeRequest, ResolveWorkflowRequest, SetWorkflowFlushContextRequest,
    SetWorkflowIntermediateOutputSchemaRequest, SetWorkflowLaunchPolicyRequest,
    SetWorkflowNodeCanCompleteRunRequest, SetWorkflowNodeCanEmitIntermediateOutputRequest,
    SetWorkflowNodeIntermediateOutputSchemaRequest, SetWorkflowNodeMaxTurnsRequest,
    SetWorkflowRunOutputSchemaRequest, UpdateWorkflowNodeInstructionsRequest,
};

pub(crate) struct KernelWorkflowService<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> KernelWorkflowService<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn create_workflow(
        &mut self,
        request: CreateWorkflowRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow = self
            .app
            .sessions_mut()
            .create_workflow(&request.session_id, request.alias)?;
        let session = self.app.local_api_session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowCreated { workflow, session })
    }

    pub(crate) fn alias_workflow(
        &mut self,
        request: AliasWorkflowRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow = self.app.sessions_mut().assign_workflow_alias(
            &request.session_id,
            &request.workflow_ref,
            request.alias,
        )?;
        let session = self.app.local_api_session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowAliased { workflow, session })
    }

    pub(crate) fn list_workflows(
        &mut self,
        request: ListWorkflowsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowsListed {
            workflows: self.app.sessions().list_workflows(&request.session_id)?,
        })
    }

    pub(crate) fn resolve_workflow(
        &mut self,
        request: ResolveWorkflowRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowResolved {
            workflow: self
                .app
                .sessions()
                .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?,
        })
    }

    pub(crate) fn create_workflow_endpoint(
        &mut self,
        request: CreateWorkflowEndpointRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let endpoint = self.app.sessions_mut().create_workflow_endpoint(
            &request.session_id,
            &request.workflow_ref,
            &request.entry_node_id,
            request.alias,
        )?;
        let workflow = self
            .app
            .sessions()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.app.local_api_session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEndpointCreated {
            endpoint,
            workflow,
            session,
        })
    }

    pub(crate) fn alias_workflow_endpoint(
        &mut self,
        request: AliasWorkflowEndpointRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let endpoint = self.app.sessions_mut().assign_workflow_endpoint_alias(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            request.alias,
        )?;
        let workflow = self
            .app
            .sessions()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.app.local_api_session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEndpointAliased {
            endpoint,
            workflow,
            session,
        })
    }

    pub(crate) fn bind_workflow_endpoint(
        &mut self,
        request: BindWorkflowEndpointRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let endpoint = self.app.sessions_mut().bind_workflow_endpoint(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            &request.entry_node_id,
        )?;
        let workflow = self
            .app
            .sessions()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.app.local_api_session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEndpointBound {
            endpoint,
            workflow,
            session,
        })
    }

    pub(crate) fn add_workflow_node(
        &mut self,
        request: AddWorkflowNodeRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let agent_exists = self
            .app
            .agents()
            .get_session_agents(&request.session_id)
            .into_iter()
            .any(|agent| agent.id() == request.agent_id);
        if !agent_exists {
            return Err(DaemonError::AgentNotFound {
                agent_id: request.agent_id,
            });
        }
        let node = self.app.sessions_mut().add_workflow_node(
            &request.session_id,
            &request.workflow_ref,
            &request.agent_id,
        )?;
        let workflow = self
            .app
            .sessions()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.app.local_api_session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeAdded {
            node,
            workflow,
            session,
        })
    }

    pub(crate) fn remove_workflow_node(
        &mut self,
        request: RemoveWorkflowNodeRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let node = self.app.sessions_mut().remove_workflow_node(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
        )?;
        let workflow = self
            .app
            .sessions()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.app.local_api_session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeRemoved {
            node,
            workflow,
            session,
        })
    }

    pub(crate) fn update_workflow_node_instructions(
        &mut self,
        request: UpdateWorkflowNodeInstructionsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let node = self.app.sessions_mut().update_workflow_node_instructions(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
            request.instructions.clone(),
        )?;
        let workflow = self
            .app
            .sessions()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.app.local_api_session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeInstructionsUpdated {
            node,
            workflow,
            session,
        })
    }

    pub(crate) fn set_workflow_node_can_complete_run(
        &mut self,
        request: SetWorkflowNodeCanCompleteRunRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let node = self.app.sessions_mut().set_workflow_node_can_complete_run(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
            request.can_complete_workflow_run,
        )?;
        let workflow = self
            .app
            .sessions()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.app.local_api_session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeCanCompleteRunUpdated {
            node,
            workflow,
            session,
        })
    }

    pub(crate) fn set_workflow_node_can_emit_intermediate_output(
        &mut self,
        request: SetWorkflowNodeCanEmitIntermediateOutputRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let node = self
            .app
            .sessions_mut()
            .set_workflow_node_can_emit_intermediate_output(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
                request.can_emit_intermediate_workflow_run_output,
            )?;
        let workflow = self
            .app
            .sessions()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.app.local_api_session_snapshot(&request.session_id)?;
        Ok(
            LocalDaemonResponse::WorkflowNodeCanEmitIntermediateOutputUpdated {
                node,
                workflow,
                session,
            },
        )
    }

    pub(crate) fn set_workflow_node_intermediate_output_schema(
        &mut self,
        request: SetWorkflowNodeIntermediateOutputSchemaRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let node = self
            .app
            .sessions_mut()
            .set_workflow_node_intermediate_output_schema_ref(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
                request.intermediate_output_schema_ref.clone(),
            )?;
        let workflow = self
            .app
            .sessions()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.app.local_api_session_snapshot(&request.session_id)?;
        Ok(
            LocalDaemonResponse::WorkflowNodeIntermediateOutputSchemaUpdated {
                node,
                workflow,
                session,
            },
        )
    }

    pub(crate) fn set_workflow_node_max_turns(
        &mut self,
        request: SetWorkflowNodeMaxTurnsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let node = self.app.sessions_mut().set_workflow_node_max_turns(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
            request.max_turns,
        )?;
        let workflow = self
            .app
            .sessions()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.app.local_api_session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeMaxTurnsUpdated {
            node,
            workflow,
            session,
        })
    }

    pub(crate) fn add_workflow_edge(
        &mut self,
        request: AddWorkflowEdgeRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let edge = self.app.sessions_mut().add_workflow_edge(
            &request.session_id,
            &request.workflow_ref,
            &request.from_node_id,
            &request.to_node_id,
            request.output_schema_ref.clone(),
            request.validation_policy,
        )?;
        let workflow = self
            .app
            .sessions()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.app.local_api_session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEdgeAdded {
            edge,
            workflow,
            session,
        })
    }

    pub(crate) fn remove_workflow_edge(
        &mut self,
        request: RemoveWorkflowEdgeRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let edge = self.app.sessions_mut().remove_workflow_edge(
            &request.session_id,
            &request.workflow_ref,
            &request.edge_id,
        )?;
        let workflow = self
            .app
            .sessions()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.app.local_api_session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEdgeRemoved {
            edge,
            workflow,
            session,
        })
    }

    pub(crate) fn set_workflow_flush_context(
        &mut self,
        request: SetWorkflowFlushContextRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow = self
            .app
            .sessions_mut()
            .set_workflow_flush_agent_context_before_run(
                &request.session_id,
                &request.workflow_ref,
                request.flush_agent_context_before_run,
            )?;
        let session = self.app.local_api_session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowFlushContextUpdated { workflow, session })
    }

    pub(crate) fn set_workflow_run_output_schema(
        &mut self,
        request: SetWorkflowRunOutputSchemaRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow = self.app.sessions_mut().set_workflow_run_output_schema_ref(
            &request.session_id,
            &request.workflow_ref,
            request.run_output_schema_ref.clone(),
        )?;
        let session = self.app.local_api_session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowRunOutputSchemaUpdated { workflow, session })
    }

    pub(crate) fn set_workflow_intermediate_output_schema(
        &mut self,
        request: SetWorkflowIntermediateOutputSchemaRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow = self
            .app
            .sessions_mut()
            .set_workflow_intermediate_output_schema_ref(
                &request.session_id,
                &request.workflow_ref,
                request.intermediate_output_schema_ref.clone(),
            )?;
        let session = self.app.local_api_session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowIntermediateOutputSchemaUpdated { workflow, session })
    }

    pub(crate) fn set_workflow_launch_policy(
        &mut self,
        request: SetWorkflowLaunchPolicyRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let session = self
            .app
            .sessions_mut()
            .set_workflow_launch_policy(&request.session_id, request.policy)?;
        let mut session = session;
        session.set_agents(self.app.agents().get_session_agents(&request.session_id));
        Ok(LocalDaemonResponse::WorkflowLaunchPolicyUpdated { session })
    }
}
