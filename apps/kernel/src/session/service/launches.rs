use super::*;
use crate::session::WorkflowPublicationInvocationEnvelope;

impl SessionService {
    pub fn invoke_workflow_endpoint(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        prompt: Option<String>,
    ) -> Result<WorkflowRun, DaemonError> {
        self.invoke_workflow_endpoint_with_publication_invocation(
            session_id,
            workflow_ref,
            endpoint_ref,
            prompt,
            None,
        )
    }

    pub fn invoke_workflow_endpoint_with_publication_invocation(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        prompt: Option<String>,
        publication_invocation: Option<WorkflowPublicationInvocationEnvelope>,
    ) -> Result<WorkflowRun, DaemonError> {
        let workflow = self.resolve_workflow_ref(session_id, workflow_ref)?;
        let endpoint =
            self.resolve_workflow_endpoint_ref(session_id, workflow_ref, endpoint_ref)?;
        self.validate_workflow_runnable(session_id, &workflow, &endpoint)?;
        let entry_node = workflow.node(endpoint.entry_node_id()).ok_or_else(|| {
            DaemonError::WorkflowNodeNotFound {
                session_id: session_id.to_string(),
                workflow_id: workflow.id().to_string(),
                node_id: endpoint.entry_node_id().to_string(),
            }
        })?;

        let node_run = WorkflowNodeRun::new(
            self.next_workflow_node_run_id(),
            entry_node.id().to_string(),
            entry_node.agent_id().to_string(),
            1,
            WorkflowNodeRunStatus::Ready,
        );
        let messages = prompt
            .as_ref()
            .filter(|value| !value.trim().is_empty())
            .map(|value| {
                let mut message = WorkflowMessage::new(
                    self.next_workflow_message_id(),
                    None,
                    entry_node.id().to_string(),
                    "invocation",
                    "workflow invoked",
                    value.clone(),
                );
                message.set_consumed_by_node_run_id(node_run.id().to_string());
                vec![message]
            })
            .unwrap_or_default();
        let workflow_run = WorkflowRun::new(
            self.next_workflow_run_id(),
            workflow.id().to_string(),
            endpoint.id().to_string(),
            entry_node.id().to_string(),
            prompt,
            publication_invocation,
            vec![node_run],
            messages,
        );
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        Ok(session.create_workflow_run(workflow_run))
    }

    pub fn list_workflow_prompt_queues(
        &self,
        session_id: &str,
        workflow_ref: Option<&str>,
    ) -> Result<Vec<WorkflowPromptQueueDefinition>, DaemonError> {
        let session = self.get_session(session_id)?;
        let Some(workflow_ref) = workflow_ref else {
            return Ok(session.workflow_prompt_queues().to_vec());
        };
        let workflow = self.resolve_workflow_ref(session_id, workflow_ref)?;
        Ok(session.workflow_prompt_queues_for_workflow(workflow.id()))
    }

    pub fn create_workflow_prompt_queue(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        alias: String,
        priority: i32,
    ) -> Result<WorkflowPromptQueueDefinition, DaemonError> {
        let max_queues = self.max_workflow_queues_per_workflow;
        let alias = normalize_workflow_queue_alias(alias)?;
        let queue_id = self.next_workflow_prompt_queue_id();
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
        if session
            .workflow_prompt_queues_for_workflow(&workflow_id)
            .len()
            >= max_queues
        {
            return Err(DaemonError::InvalidConfig {
                field: "workflow.max_queues_per_workflow",
                message: "workflow prompt queue limit reached",
            });
        }
        if session
            .workflow_prompt_queue(&workflow_id, &alias)
            .is_some()
        {
            return Err(DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id,
                reference: alias,
                message: "workflow prompt queue alias already exists",
            });
        }
        let queue = WorkflowPromptQueueDefinition::new(queue_id, workflow_id, alias, priority);
        Ok(session.add_workflow_prompt_queue(queue))
    }

    pub fn update_workflow_prompt_queue(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        queue_ref: &str,
        alias: Option<String>,
        priority: Option<i32>,
        enabled: Option<bool>,
    ) -> Result<WorkflowPromptQueueDefinition, DaemonError> {
        let alias = alias.map(normalize_workflow_queue_alias).transpose()?;
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
        if let Some(alias) = alias.as_deref() {
            let queue_id = session
                .workflow_prompt_queue(&workflow_id, queue_ref)
                .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                    session_id: session_id.to_string(),
                    workflow_id: workflow_id.clone(),
                    reference: queue_ref.to_string(),
                    message: "workflow prompt queue was not found",
                })?
                .id()
                .to_string();
            if session
                .workflow_prompt_queues_for_workflow(&workflow_id)
                .iter()
                .any(|queue| queue.alias() == alias && queue.id() != queue_id)
            {
                return Err(DaemonError::InvalidWorkflowGraphReference {
                    session_id: session_id.to_string(),
                    workflow_id: workflow_id.clone(),
                    reference: alias.to_string(),
                    message: "workflow prompt queue alias already exists",
                });
            }
        }
        let queue = session
            .workflow_prompt_queue_mut(&workflow_id, queue_ref)
            .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                reference: queue_ref.to_string(),
                message: "workflow prompt queue was not found",
            })?;
        if let Some(alias) = alias {
            queue.set_alias(alias);
        }
        if let Some(priority) = priority {
            queue.set_priority(priority);
        }
        if let Some(enabled) = enabled {
            queue.set_enabled(enabled);
        }
        Ok(queue.clone())
    }

    pub fn remove_workflow_prompt_queue(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        queue_ref: &str,
    ) -> Result<WorkflowPromptQueueDefinition, DaemonError> {
        if queue_ref == "default" {
            let workflow_id = self
                .resolve_workflow_ref(session_id, workflow_ref)?
                .id()
                .to_string();
            return Err(DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id,
                reference: queue_ref.to_string(),
                message: "default workflow prompt queue cannot be removed",
            });
        }
        let workflow_id = self
            .resolve_workflow_ref(session_id, workflow_ref)?
            .id()
            .to_string();
        let queue_id =
            self.resolve_workflow_prompt_queue_ref(session_id, &workflow_id, queue_ref)?;
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        if session
            .workflow_queued_prompts()
            .iter()
            .any(|item| item.queue_id() == queue_id)
        {
            return Err(DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id,
                reference: queue_ref.to_string(),
                message: "workflow prompt queue has queued prompts",
            });
        }
        session
            .remove_workflow_prompt_queue(&workflow_id, &queue_id)
            .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id,
                reference: queue_ref.to_string(),
                message: "workflow prompt queue was not found",
            })
    }

    pub fn list_queued_workflow_prompts(
        &self,
        session_id: &str,
    ) -> Result<Vec<WorkflowQueuedPrompt>, DaemonError> {
        Ok(self
            .get_session(session_id)?
            .workflow_queued_prompts()
            .iter()
            .cloned()
            .collect())
    }

    pub fn enqueue_workflow_prompt(
        &mut self,
        session_id: &str,
        workflow_id: &str,
        endpoint_id: &str,
        prompt: Option<String>,
        queue_ref: Option<&str>,
        source: WorkflowQueuedPromptSource,
        watchdog_id: Option<String>,
    ) -> Result<WorkflowQueuedPrompt, DaemonError> {
        self.enqueue_workflow_prompt_with_publication_invocation(
            session_id,
            workflow_id,
            endpoint_id,
            prompt,
            queue_ref,
            source,
            watchdog_id,
            None,
        )
    }

    pub fn enqueue_workflow_prompt_with_publication_invocation(
        &mut self,
        session_id: &str,
        workflow_id: &str,
        endpoint_id: &str,
        prompt: Option<String>,
        queue_ref: Option<&str>,
        source: WorkflowQueuedPromptSource,
        watchdog_id: Option<String>,
        publication_invocation: Option<WorkflowPublicationInvocationEnvelope>,
    ) -> Result<WorkflowQueuedPrompt, DaemonError> {
        let queue_ref = queue_ref.unwrap_or("default");
        let workflow = self.resolve_workflow_ref(session_id, workflow_id)?;
        let queue_id =
            self.resolve_workflow_prompt_queue_ref(session_id, workflow.id(), queue_ref)?;
        let endpoint =
            self.resolve_workflow_endpoint_ref(session_id, workflow.id(), endpoint_id)?;
        self.validate_workflow_runnable(session_id, &workflow, &endpoint)?;
        let queued = WorkflowQueuedPrompt::new(
            self.next_workflow_queued_prompt_id(),
            queue_id,
            workflow.id().to_string(),
            endpoint.id().to_string(),
            prompt,
            publication_invocation,
            source,
            watchdog_id,
        );
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        Ok(session.enqueue_workflow_prompt(queued))
    }

    pub fn update_queued_workflow_prompt(
        &mut self,
        session_id: &str,
        queue_item_ref: &str,
        prompt: Option<String>,
        queue_ref: Option<&str>,
    ) -> Result<WorkflowQueuedPrompt, DaemonError> {
        let queue_item_id = self.resolve_queued_workflow_prompt_ref(session_id, queue_item_ref)?;
        let queued = self
            .get_session(session_id)?
            .workflow_queued_prompts()
            .iter()
            .find(|item| item.id() == queue_item_id)
            .cloned()
            .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: "workflow".to_string(),
                reference: queue_item_ref.to_string(),
                message: "queued workflow prompt was not found",
            })?;
        let queue_id = match queue_ref {
            Some(queue_ref) => Some(self.resolve_workflow_prompt_queue_ref(
                session_id,
                queued.workflow_id(),
                queue_ref,
            )?),
            None => None,
        };
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        session
            .update_queued_workflow_prompt(&queue_item_id, prompt, queue_id)
            .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: "workflow".to_string(),
                reference: queue_item_ref.to_string(),
                message: "queued workflow prompt was not found or was already dispatched",
            })
    }

    pub fn remove_queued_workflow_prompt(
        &mut self,
        session_id: &str,
        queue_item_ref: &str,
    ) -> Result<WorkflowQueuedPrompt, DaemonError> {
        let queue_item_id = self.resolve_queued_workflow_prompt_ref(session_id, queue_item_ref)?;
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        session
            .remove_queued_workflow_prompt(&queue_item_id)
            .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: "workflow".to_string(),
                reference: queue_item_ref.to_string(),
                message: "queued workflow prompt was not found or was already dispatched",
            })
    }

    pub fn clear_workflow_queue(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        queue_ref: &str,
    ) -> Result<Vec<WorkflowQueuedPrompt>, DaemonError> {
        let workflow_id = self
            .resolve_workflow_ref(session_id, workflow_ref)?
            .id()
            .to_string();
        let queue_id =
            self.resolve_workflow_prompt_queue_ref(session_id, &workflow_id, queue_ref)?;
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        Ok(session.clear_workflow_queue(&queue_id))
    }

    pub fn dequeue_next_workflow_prompt(
        &mut self,
        session_id: &str,
    ) -> Result<Option<WorkflowQueuedPrompt>, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        if session.has_active_workflow_run() {
            return Ok(None);
        }
        Ok(session.pop_next_workflow_queued_prompt())
    }

    fn validate_workflow_runnable(
        &self,
        session_id: &str,
        workflow: &WorkflowDefinition,
        endpoint: &WorkflowEndpointDefinition,
    ) -> Result<(), DaemonError> {
        let entry_node_id = endpoint.entry_node_id();
        workflow
            .node(entry_node_id)
            .ok_or_else(|| DaemonError::WorkflowNodeNotFound {
                session_id: session_id.to_string(),
                workflow_id: workflow.id().to_string(),
                node_id: entry_node_id.to_string(),
            })?;

        let node_ids = workflow
            .nodes()
            .iter()
            .map(|node| node.id().to_string())
            .collect::<BTreeSet<_>>();
        for edge in workflow.edges() {
            if !node_ids.contains(edge.from_node_id()) {
                return Err(DaemonError::InvalidWorkflowGraphReference {
                    session_id: session_id.to_string(),
                    workflow_id: workflow.id().to_string(),
                    reference: edge.from_node_id().to_string(),
                    message: "edge references missing source node",
                });
            }
            if !node_ids.contains(edge.to_node_id()) {
                return Err(DaemonError::InvalidWorkflowGraphReference {
                    session_id: session_id.to_string(),
                    workflow_id: workflow.id().to_string(),
                    reference: edge.to_node_id().to_string(),
                    message: "edge references missing target node",
                });
            }
        }

        Ok(())
    }
}
