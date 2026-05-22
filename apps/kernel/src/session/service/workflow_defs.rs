use super::*;
use crate::session::{WorkflowCanvasLayout, WorkflowCanvasLayoutPatch, WorkflowCanvasPoint};

impl SessionService {
    pub fn apply_workflow_design_op(
        &mut self,
        session_id: &str,
        op: crate::local::WorkflowDesignOp,
        owner_user_id: String,
    ) -> Result<WorkflowDefinition, DaemonError> {
        match op {
            crate::local::WorkflowDesignOp::WorkflowCreate { workflow } => {
                let mut definition = WorkflowDefinition::new(workflow.id, workflow.alias);
                if let Some(value) = workflow.flush_agent_context_before_run {
                    definition.set_flush_agent_context_before_run(value);
                }
                if workflow.run_output_schema_ref.is_some() {
                    definition.set_run_output_schema_ref(workflow.run_output_schema_ref);
                }
                if workflow.intermediate_output_schema_ref.is_some() {
                    definition.set_intermediate_output_schema_ref(
                        workflow.intermediate_output_schema_ref,
                    );
                }
                let session =
                    self.store
                        .get_mut(session_id)
                        .ok_or_else(|| DaemonError::SessionNotFound {
                            session_id: session_id.to_string(),
                        })?;
                Ok(session.create_workflow(definition))
            }
            crate::local::WorkflowDesignOp::WorkflowUpdate { workflow_id, patch } => {
                let workflow_id = self
                    .resolve_workflow_ref(session_id, &workflow_id)?
                    .id()
                    .to_string();
                let session =
                    self.store
                        .get_mut(session_id)
                        .ok_or_else(|| DaemonError::SessionNotFound {
                            session_id: session_id.to_string(),
                        })?;
                let workflow = session.workflow_mut(&workflow_id).ok_or_else(|| {
                    DaemonError::WorkflowNotFound {
                        session_id: session_id.to_string(),
                        workflow_id: workflow_id.clone(),
                    }
                })?;
                if let Some(alias) = patch.alias {
                    workflow.set_alias(alias);
                }
                if let Some(value) = patch.flush_agent_context_before_run {
                    workflow.set_flush_agent_context_before_run(value);
                }
                if let Some(value) = patch.run_output_schema_ref {
                    workflow.set_run_output_schema_ref(value);
                }
                if let Some(value) = patch.intermediate_output_schema_ref {
                    workflow.set_intermediate_output_schema_ref(value);
                }
                Ok(workflow.clone())
            }
            crate::local::WorkflowDesignOp::WorkflowRemove { workflow_id } => {
                let workflow_id = self
                    .resolve_workflow_ref(session_id, &workflow_id)?
                    .id()
                    .to_string();
                let session =
                    self.store
                        .get_mut(session_id)
                        .ok_or_else(|| DaemonError::SessionNotFound {
                            session_id: session_id.to_string(),
                        })?;
                session
                    .remove_workflow(&workflow_id)
                    .ok_or_else(|| DaemonError::WorkflowNotFound {
                        session_id: session_id.to_string(),
                        workflow_id,
                    })
            }
            crate::local::WorkflowDesignOp::NodeAdd {
                workflow_id,
                node,
                position,
            } => {
                let workflow_id = self
                    .resolve_workflow_ref(session_id, &workflow_id)?
                    .id()
                    .to_string();
                let session =
                    self.store
                        .get_mut(session_id)
                        .ok_or_else(|| DaemonError::SessionNotFound {
                            session_id: session_id.to_string(),
                        })?;
                let workflow = session.workflow_mut(&workflow_id).ok_or_else(|| {
                    DaemonError::WorkflowNotFound {
                        session_id: session_id.to_string(),
                        workflow_id: workflow_id.clone(),
                    }
                })?;
                let mut definition = WorkflowNodeDefinition::new(node.id.clone(), node.agent_id);
                definition.set_owner_user_id(owner_user_id);
                if let Some(label) = node.label {
                    definition.set_public_label(label);
                }
                if node.instructions.is_some() {
                    definition.set_instructions(node.instructions);
                }
                if let Some(value) = node.can_complete_workflow_run {
                    definition.set_can_complete_workflow_run(value);
                }
                if let Some(value) = node.can_emit_intermediate_run_output {
                    definition.set_can_emit_intermediate_run_output(value);
                }
                if node.intermediate_output_schema_ref.is_some() {
                    definition
                        .set_intermediate_output_schema_ref(node.intermediate_output_schema_ref);
                }
                if node.max_turns.is_some() {
                    definition.set_max_turns(node.max_turns);
                }
                workflow.add_node(definition);
                if let Some(point) = position {
                    workflow.set_node_position(
                        node.id,
                        WorkflowCanvasPoint {
                            x: point.x,
                            y: point.y,
                        },
                    );
                }
                Ok(workflow.clone())
            }
            crate::local::WorkflowDesignOp::NodeUpdate {
                workflow_id,
                node_id,
                patch,
            } => {
                let workflow_id = self
                    .resolve_workflow_ref(session_id, &workflow_id)?
                    .id()
                    .to_string();
                let session =
                    self.store
                        .get_mut(session_id)
                        .ok_or_else(|| DaemonError::SessionNotFound {
                            session_id: session_id.to_string(),
                        })?;
                let workflow = session.workflow_mut(&workflow_id).ok_or_else(|| {
                    DaemonError::WorkflowNotFound {
                        session_id: session_id.to_string(),
                        workflow_id: workflow_id.clone(),
                    }
                })?;
                let node = workflow.node_mut(&node_id).ok_or_else(|| {
                    DaemonError::WorkflowNodeNotFound {
                        session_id: session_id.to_string(),
                        workflow_id: workflow_id.clone(),
                        node_id: node_id.clone(),
                    }
                })?;
                if let Some(label) = patch.label {
                    node.set_public_label(label);
                }
                if let Some(value) = patch.instructions {
                    node.set_instructions(value);
                }
                if let Some(value) = patch.can_complete_workflow_run {
                    node.set_can_complete_workflow_run(value);
                }
                if let Some(value) = patch.can_emit_intermediate_run_output {
                    node.set_can_emit_intermediate_run_output(value);
                }
                if let Some(value) = patch.intermediate_output_schema_ref {
                    node.set_intermediate_output_schema_ref(value);
                }
                if let Some(value) = patch.max_turns {
                    node.set_max_turns(value);
                }
                workflow.bump_revision();
                Ok(workflow.clone())
            }
            crate::local::WorkflowDesignOp::NodeMove {
                workflow_id,
                node_id,
                position,
            } => {
                let workflow_id = self
                    .resolve_workflow_ref(session_id, &workflow_id)?
                    .id()
                    .to_string();
                let session =
                    self.store
                        .get_mut(session_id)
                        .ok_or_else(|| DaemonError::SessionNotFound {
                            session_id: session_id.to_string(),
                        })?;
                let workflow = session.workflow_mut(&workflow_id).ok_or_else(|| {
                    DaemonError::WorkflowNotFound {
                        session_id: session_id.to_string(),
                        workflow_id: workflow_id.clone(),
                    }
                })?;
                workflow.set_node_position(
                    node_id,
                    WorkflowCanvasPoint {
                        x: position.x,
                        y: position.y,
                    },
                );
                Ok(workflow.clone())
            }
            crate::local::WorkflowDesignOp::NodeRemove {
                workflow_id,
                node_id,
            } => {
                let workflow_id = self
                    .resolve_workflow_ref(session_id, &workflow_id)?
                    .id()
                    .to_string();
                let session =
                    self.store
                        .get_mut(session_id)
                        .ok_or_else(|| DaemonError::SessionNotFound {
                            session_id: session_id.to_string(),
                        })?;
                let workflow = session.workflow_mut(&workflow_id).ok_or_else(|| {
                    DaemonError::WorkflowNotFound {
                        session_id: session_id.to_string(),
                        workflow_id: workflow_id.clone(),
                    }
                })?;
                workflow.remove_node(&node_id);
                Ok(workflow.clone())
            }
            crate::local::WorkflowDesignOp::EdgeAdd { workflow_id, edge } => {
                let workflow_id = self
                    .resolve_workflow_ref(session_id, &workflow_id)?
                    .id()
                    .to_string();
                let session =
                    self.store
                        .get_mut(session_id)
                        .ok_or_else(|| DaemonError::SessionNotFound {
                            session_id: session_id.to_string(),
                        })?;
                let workflow = session.workflow_mut(&workflow_id).ok_or_else(|| {
                    DaemonError::WorkflowNotFound {
                        session_id: session_id.to_string(),
                        workflow_id: workflow_id.clone(),
                    }
                })?;
                let mut definition = WorkflowEdgeDefinition::new(
                    edge.id,
                    edge.from_node_id,
                    edge.to_node_id,
                    edge.handoff_schema_ref,
                    edge.validation_policy,
                );
                definition.set_created_by_user_id(owner_user_id);
                workflow.add_edge(definition);
                Ok(workflow.clone())
            }
            crate::local::WorkflowDesignOp::EdgeUpdate {
                workflow_id,
                edge_id,
                patch,
            } => {
                let workflow_id = self
                    .resolve_workflow_ref(session_id, &workflow_id)?
                    .id()
                    .to_string();
                let session =
                    self.store
                        .get_mut(session_id)
                        .ok_or_else(|| DaemonError::SessionNotFound {
                            session_id: session_id.to_string(),
                        })?;
                let workflow = session.workflow_mut(&workflow_id).ok_or_else(|| {
                    DaemonError::WorkflowNotFound {
                        session_id: session_id.to_string(),
                        workflow_id: workflow_id.clone(),
                    }
                })?;
                let edge = workflow.edge_mut(&edge_id).ok_or_else(|| {
                    DaemonError::WorkflowEdgeNotFound {
                        session_id: session_id.to_string(),
                        workflow_id: workflow_id.clone(),
                        edge_id: edge_id.clone(),
                    }
                })?;
                if let Some(value) = patch.handoff_schema_ref {
                    edge.set_handoff_schema_ref(value);
                }
                if let Some(value) = patch.validation_policy {
                    edge.set_validation_policy(value);
                }
                workflow.bump_revision();
                Ok(workflow.clone())
            }
            crate::local::WorkflowDesignOp::EdgeRemove {
                workflow_id,
                edge_id,
            } => {
                let workflow_id = self
                    .resolve_workflow_ref(session_id, &workflow_id)?
                    .id()
                    .to_string();
                let session =
                    self.store
                        .get_mut(session_id)
                        .ok_or_else(|| DaemonError::SessionNotFound {
                            session_id: session_id.to_string(),
                        })?;
                let workflow = session.workflow_mut(&workflow_id).ok_or_else(|| {
                    DaemonError::WorkflowNotFound {
                        session_id: session_id.to_string(),
                        workflow_id: workflow_id.clone(),
                    }
                })?;
                workflow.remove_edge(&edge_id);
                Ok(workflow.clone())
            }
            crate::local::WorkflowDesignOp::EndpointAdd {
                workflow_id,
                endpoint,
                position,
            } => {
                let workflow_id = self
                    .resolve_workflow_ref(session_id, &workflow_id)?
                    .id()
                    .to_string();
                let session =
                    self.store
                        .get_mut(session_id)
                        .ok_or_else(|| DaemonError::SessionNotFound {
                            session_id: session_id.to_string(),
                        })?;
                let workflow = session.workflow_mut(&workflow_id).ok_or_else(|| {
                    DaemonError::WorkflowNotFound {
                        session_id: session_id.to_string(),
                        workflow_id: workflow_id.clone(),
                    }
                })?;
                let mut definition = WorkflowEndpointDefinition::new(
                    endpoint.id.clone(),
                    endpoint.alias,
                    endpoint.entry_node_id,
                );
                definition.set_owner_user_id(owner_user_id);
                workflow.add_endpoint(definition);
                if let Some(point) = position {
                    workflow.set_endpoint_position(
                        endpoint.id,
                        WorkflowCanvasPoint {
                            x: point.x,
                            y: point.y,
                        },
                    );
                }
                Ok(workflow.clone())
            }
            crate::local::WorkflowDesignOp::EndpointUpdate {
                workflow_id,
                endpoint_id,
                patch,
            } => {
                let workflow_id = self
                    .resolve_workflow_ref(session_id, &workflow_id)?
                    .id()
                    .to_string();
                let session =
                    self.store
                        .get_mut(session_id)
                        .ok_or_else(|| DaemonError::SessionNotFound {
                            session_id: session_id.to_string(),
                        })?;
                let workflow = session.workflow_mut(&workflow_id).ok_or_else(|| {
                    DaemonError::WorkflowNotFound {
                        session_id: session_id.to_string(),
                        workflow_id: workflow_id.clone(),
                    }
                })?;
                let endpoint = workflow.endpoint_mut(&endpoint_id).ok_or_else(|| {
                    DaemonError::WorkflowEndpointNotFound {
                        session_id: session_id.to_string(),
                        workflow_id: workflow_id.clone(),
                        endpoint_id: endpoint_id.clone(),
                    }
                })?;
                if let Some(alias) = patch.alias {
                    endpoint.set_alias(alias);
                }
                if let Some(entry_node_id) = patch.entry_node_id {
                    endpoint.set_entry_node_id(entry_node_id);
                }
                workflow.bump_revision();
                Ok(workflow.clone())
            }
            crate::local::WorkflowDesignOp::EndpointMove {
                workflow_id,
                endpoint_id,
                position,
            } => {
                let workflow_id = self
                    .resolve_workflow_ref(session_id, &workflow_id)?
                    .id()
                    .to_string();
                let session =
                    self.store
                        .get_mut(session_id)
                        .ok_or_else(|| DaemonError::SessionNotFound {
                            session_id: session_id.to_string(),
                        })?;
                let workflow = session.workflow_mut(&workflow_id).ok_or_else(|| {
                    DaemonError::WorkflowNotFound {
                        session_id: session_id.to_string(),
                        workflow_id: workflow_id.clone(),
                    }
                })?;
                workflow.set_endpoint_position(
                    endpoint_id,
                    WorkflowCanvasPoint {
                        x: position.x,
                        y: position.y,
                    },
                );
                Ok(workflow.clone())
            }
            crate::local::WorkflowDesignOp::EndpointRemove {
                workflow_id,
                endpoint_id,
            } => {
                let workflow_id = self
                    .resolve_workflow_ref(session_id, &workflow_id)?
                    .id()
                    .to_string();
                let session =
                    self.store
                        .get_mut(session_id)
                        .ok_or_else(|| DaemonError::SessionNotFound {
                            session_id: session_id.to_string(),
                        })?;
                let workflow = session.workflow_mut(&workflow_id).ok_or_else(|| {
                    DaemonError::WorkflowNotFound {
                        session_id: session_id.to_string(),
                        workflow_id: workflow_id.clone(),
                    }
                })?;
                workflow.remove_endpoint(&endpoint_id);
                Ok(workflow.clone())
            }
        }
    }

    pub fn add_workflow_node(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        agent_id: &str,
    ) -> Result<WorkflowNodeDefinition, DaemonError> {
        self.add_workflow_node_owned(
            session_id,
            workflow_ref,
            agent_id,
            DEFAULT_LOCAL_USER_ID.to_string(),
            agent_id.to_string(),
        )
    }

    pub fn add_workflow_node_owned(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        agent_id: &str,
        owner_user_id: String,
        public_label: String,
    ) -> Result<WorkflowNodeDefinition, DaemonError> {
        let workflow_id = self
            .resolve_workflow_ref(session_id, workflow_ref)?
            .id()
            .to_string();
        let next_node_id = self.next_workflow_node_id();
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
        if workflow
            .nodes()
            .iter()
            .any(|node| node.agent_id() == agent_id)
        {
            return Err(DaemonError::WorkflowNodeConflict {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                agent_id: agent_id.to_string(),
            });
        }
        let mut node = WorkflowNodeDefinition::new(next_node_id, agent_id.to_string());
        node.set_owner_user_id(owner_user_id);
        node.set_public_label(public_label);
        Ok(workflow.add_node(node))
    }

    pub fn update_workflow_node_instructions(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        node_id: &str,
        instructions: Option<String>,
    ) -> Result<WorkflowNodeDefinition, DaemonError> {
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
        let node = workflow
            .node_mut(node_id)
            .ok_or_else(|| DaemonError::WorkflowNodeNotFound {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                node_id: node_id.to_string(),
            })?;
        node.set_instructions(instructions);
        let node = node.clone();
        workflow.bump_revision();
        Ok(node)
    }

    pub fn set_workflow_node_can_complete_run(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        node_id: &str,
        value: bool,
    ) -> Result<WorkflowNodeDefinition, DaemonError> {
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
        let node = workflow
            .node_mut(node_id)
            .ok_or_else(|| DaemonError::WorkflowNodeNotFound {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                node_id: node_id.to_string(),
            })?;
        node.set_can_complete_workflow_run(value);
        let node = node.clone();
        workflow.bump_revision();
        Ok(node)
    }

    pub fn set_workflow_node_can_emit_intermediate_output(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        node_id: &str,
        value: bool,
    ) -> Result<WorkflowNodeDefinition, DaemonError> {
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
        let node = workflow
            .node_mut(node_id)
            .ok_or_else(|| DaemonError::WorkflowNodeNotFound {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                node_id: node_id.to_string(),
            })?;
        node.set_can_emit_intermediate_run_output(value);
        let node = node.clone();
        workflow.bump_revision();
        Ok(node)
    }

    pub fn set_workflow_node_intermediate_output_schema_ref(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        node_id: &str,
        value: Option<String>,
    ) -> Result<WorkflowNodeDefinition, DaemonError> {
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
        let node = workflow
            .node_mut(node_id)
            .ok_or_else(|| DaemonError::WorkflowNodeNotFound {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                node_id: node_id.to_string(),
            })?;
        node.set_intermediate_output_schema_ref(value);
        let node = node.clone();
        workflow.bump_revision();
        Ok(node)
    }

    pub fn set_workflow_node_max_turns(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        node_id: &str,
        value: Option<u32>,
    ) -> Result<WorkflowNodeDefinition, DaemonError> {
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
        let node = workflow
            .node_mut(node_id)
            .ok_or_else(|| DaemonError::WorkflowNodeNotFound {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                node_id: node_id.to_string(),
            })?;
        node.set_max_turns(value);
        let node = node.clone();
        workflow.bump_revision();
        Ok(node)
    }

    pub fn remove_workflow_node(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        node_id: &str,
    ) -> Result<WorkflowNodeDefinition, DaemonError> {
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
        workflow
            .remove_node(node_id)
            .ok_or_else(|| DaemonError::WorkflowNodeNotFound {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                node_id: node_id.to_string(),
            })
    }

    pub fn add_workflow_edge(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        from_node_id: &str,
        to_node_id: &str,
        handoff_schema_ref: Option<String>,
        validation_policy: Option<WorkflowHandoffValidationPolicy>,
    ) -> Result<WorkflowEdgeDefinition, DaemonError> {
        self.add_workflow_edge_owned(
            session_id,
            workflow_ref,
            from_node_id,
            to_node_id,
            DEFAULT_LOCAL_USER_ID.to_string(),
            handoff_schema_ref,
            validation_policy,
        )
    }

    pub fn add_workflow_edge_owned(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        from_node_id: &str,
        to_node_id: &str,
        created_by_user_id: String,
        handoff_schema_ref: Option<String>,
        validation_policy: Option<WorkflowHandoffValidationPolicy>,
    ) -> Result<WorkflowEdgeDefinition, DaemonError> {
        let workflow_id = self
            .resolve_workflow_ref(session_id, workflow_ref)?
            .id()
            .to_string();
        let mut edge = WorkflowEdgeDefinition::new(
            self.next_workflow_edge_id(),
            from_node_id.to_string(),
            to_node_id.to_string(),
            handoff_schema_ref,
            validation_policy,
        );
        edge.set_created_by_user_id(created_by_user_id);
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
        if workflow.node(from_node_id).is_none() {
            return Err(DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                reference: from_node_id.to_string(),
                message: "source node does not exist",
            });
        }
        if workflow.node(to_node_id).is_none() {
            return Err(DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                reference: to_node_id.to_string(),
                message: "target node does not exist",
            });
        }
        if from_node_id == to_node_id {
            return Err(DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                reference: from_node_id.to_string(),
                message: "source and target nodes must be different",
            });
        }
        if workflow.has_edge(from_node_id, to_node_id) {
            return Err(DaemonError::WorkflowEdgeConflict {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                from_node_id: from_node_id.to_string(),
                to_node_id: to_node_id.to_string(),
            });
        }
        Ok(workflow.add_edge(edge))
    }

    pub fn remove_workflow_edge(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        edge_id: &str,
    ) -> Result<WorkflowEdgeDefinition, DaemonError> {
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
        workflow
            .remove_edge(edge_id)
            .ok_or_else(|| DaemonError::WorkflowEdgeNotFound {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                edge_id: edge_id.to_string(),
            })
    }

    pub fn update_workflow_canvas_layout(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        patches: Vec<WorkflowCanvasLayoutPatch>,
    ) -> Result<WorkflowCanvasLayout, DaemonError> {
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
        Ok(workflow.update_canvas_layout(patches))
    }

    pub fn create_workflow_endpoint(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        entry_node_id: &str,
        alias: Option<String>,
    ) -> Result<WorkflowEndpointDefinition, DaemonError> {
        let workflow_id = self
            .resolve_workflow_ref(session_id, workflow_ref)?
            .id()
            .to_string();
        let alias = normalize_workflow_endpoint_alias(alias)?;
        if let Some(alias) = alias.as_deref() {
            self.ensure_workflow_endpoint_alias_available(session_id, &workflow_id, alias)?;
        }
        let endpoint = WorkflowEndpointDefinition::new(
            self.next_workflow_endpoint_id(),
            alias,
            entry_node_id.to_string(),
        );
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
        if workflow.node(entry_node_id).is_none() {
            return Err(DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                reference: entry_node_id.to_string(),
                message: "entry node does not exist",
            });
        }
        Ok(workflow.add_endpoint(endpoint))
    }

    pub fn set_workflow_endpoint_owner(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        owner_user_id: String,
    ) -> Result<WorkflowEndpointDefinition, DaemonError> {
        let workflow_id = self
            .resolve_workflow_ref(session_id, workflow_ref)?
            .id()
            .to_string();
        let endpoint_id = self
            .resolve_workflow_endpoint_ref(session_id, workflow_ref, endpoint_ref)?
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
        let endpoint = workflow.endpoint_mut(&endpoint_id).ok_or_else(|| {
            DaemonError::WorkflowEndpointNotFound {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                endpoint_id: endpoint_id.clone(),
            }
        })?;
        endpoint.set_owner_user_id(owner_user_id);
        let endpoint = endpoint.clone();
        workflow.bump_revision();
        Ok(endpoint)
    }

    pub fn assign_workflow_endpoint_alias(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        alias: String,
    ) -> Result<WorkflowEndpointDefinition, DaemonError> {
        let workflow_id = self
            .resolve_workflow_ref(session_id, workflow_ref)?
            .id()
            .to_string();
        let endpoint_id = self
            .resolve_workflow_endpoint_ref(session_id, workflow_ref, endpoint_ref)?
            .id()
            .to_string();
        let alias = normalize_workflow_endpoint_alias(Some(alias))?.ok_or_else(|| {
            DaemonError::InvalidWorkflowEndpointAlias {
                alias: String::new(),
                message: "alias cannot be empty",
            }
        })?;
        self.ensure_workflow_endpoint_alias_available_for_update(
            session_id,
            &workflow_id,
            &endpoint_id,
            &alias,
        )?;
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
        let endpoint = workflow.endpoint_mut(&endpoint_id).ok_or_else(|| {
            DaemonError::WorkflowEndpointNotFound {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                endpoint_id: endpoint_id.clone(),
            }
        })?;
        endpoint.set_alias(Some(alias));
        let endpoint = endpoint.clone();
        workflow.bump_revision();
        Ok(endpoint)
    }

    pub fn bind_workflow_endpoint(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        entry_node_id: &str,
    ) -> Result<WorkflowEndpointDefinition, DaemonError> {
        let workflow_id = self
            .resolve_workflow_ref(session_id, workflow_ref)?
            .id()
            .to_string();
        let endpoint_id = self
            .resolve_workflow_endpoint_ref(session_id, workflow_ref, endpoint_ref)?
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
        if workflow.node(entry_node_id).is_none() {
            return Err(DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                reference: entry_node_id.to_string(),
                message: "entry node does not exist",
            });
        }
        let endpoint = workflow.endpoint_mut(&endpoint_id).ok_or_else(|| {
            DaemonError::WorkflowEndpointNotFound {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                endpoint_id: endpoint_id.clone(),
            }
        })?;
        endpoint.set_entry_node_id(entry_node_id.to_string());
        let endpoint = endpoint.clone();
        workflow.bump_revision();
        Ok(endpoint)
    }
}
