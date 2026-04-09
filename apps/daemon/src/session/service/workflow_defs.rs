use super::*;

impl SessionService {
    pub fn add_workflow_node(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        agent_id: &str,
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
        let node = WorkflowNodeDefinition::new(next_node_id, agent_id.to_string());
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
        Ok(node.clone())
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
        Ok(node.clone())
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
        Ok(node.clone())
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
        Ok(node.clone())
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
        Ok(node.clone())
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
        output_schema_ref: Option<String>,
        validation_policy: Option<WorkflowOutputValidationPolicy>,
    ) -> Result<WorkflowEdgeDefinition, DaemonError> {
        let workflow_id = self
            .resolve_workflow_ref(session_id, workflow_ref)?
            .id()
            .to_string();
        let edge = WorkflowEdgeDefinition::new(
            self.next_workflow_edge_id(),
            from_node_id.to_string(),
            to_node_id.to_string(),
            output_schema_ref,
            validation_policy,
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
        Ok(endpoint.clone())
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
        Ok(endpoint.clone())
    }
}
