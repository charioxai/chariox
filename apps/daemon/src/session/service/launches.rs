use super::*;

impl SessionService {
    pub fn invoke_workflow_endpoint(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        prompt: Option<String>,
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

    pub fn set_workflow_launch_policy(
        &mut self,
        session_id: &str,
        policy: WorkflowLaunchPolicy,
    ) -> Result<RuntimeSession, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        session.set_workflow_launch_policy(policy);
        Ok(session.clone())
    }

    pub fn list_queued_workflow_launches(
        &self,
        session_id: &str,
    ) -> Result<Vec<QueuedWorkflowLaunch>, DaemonError> {
        Ok(self
            .get_session(session_id)?
            .queued_workflow_launches()
            .iter()
            .cloned()
            .collect())
    }

    pub fn remove_queued_workflow_launch(
        &mut self,
        session_id: &str,
        queue_item_ref: &str,
    ) -> Result<QueuedWorkflowLaunch, DaemonError> {
        let queue_item_id = self.resolve_queued_workflow_launch_ref(session_id, queue_item_ref)?;
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        session
            .remove_queued_workflow_launch(&queue_item_id)
            .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: queue_item_id.clone(),
                reference: queue_item_id,
                message: "queued workflow launch was not found",
            })
    }

    pub fn clear_queued_workflow_launches(
        &mut self,
        session_id: &str,
    ) -> Result<Vec<QueuedWorkflowLaunch>, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        Ok(session.clear_queued_workflow_launches())
    }

    pub fn admit_manual_workflow_launch(
        &mut self,
        session_id: &str,
        workflow_id: &str,
        endpoint_id: &str,
        prompt: Option<String>,
    ) -> Result<WorkflowLaunchAdmission, DaemonError> {
        let session = self.get_session(session_id)?;
        if !session.has_active_workflow_run() {
            return Ok(WorkflowLaunchAdmission::StartNow);
        }
        match session.workflow_launch_policy() {
            WorkflowLaunchPolicy::Reject => Err(DaemonError::WorkflowLaunchRejected {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.to_string(),
                endpoint_id: endpoint_id.to_string(),
                message:
                    "another workflow run is already active in this session; change `/workflow launch-policy queue` to queue workflow launches"
                        .to_string(),
            }),
            WorkflowLaunchPolicy::Queue => {
                let queued = QueuedWorkflowLaunch::new(
                    self.next_queued_workflow_launch_id(),
                    workflow_id.to_string(),
                    endpoint_id.to_string(),
                    prompt,
                    QueuedWorkflowLaunchSource::Manual,
                    None,
                );
                let session = self
                    .store
                    .get_mut(session_id)
                    .ok_or_else(|| DaemonError::SessionNotFound {
                        session_id: session_id.to_string(),
                    })?;
                Ok(WorkflowLaunchAdmission::Queued(
                    session.enqueue_workflow_launch(queued),
                ))
            }
        }
    }

    pub fn queue_watchdog_workflow_launch(
        &mut self,
        session_id: &str,
        workflow_id: &str,
        endpoint_id: &str,
        prompt: Option<String>,
        watchdog_id: &str,
    ) -> Result<QueuedWorkflowLaunch, DaemonError> {
        let queued = QueuedWorkflowLaunch::new(
            self.next_queued_workflow_launch_id(),
            workflow_id.to_string(),
            endpoint_id.to_string(),
            prompt,
            QueuedWorkflowLaunchSource::Watchdog,
            Some(watchdog_id.to_string()),
        );
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        Ok(session.enqueue_workflow_launch(queued))
    }

    pub fn dequeue_next_workflow_launch(
        &mut self,
        session_id: &str,
    ) -> Result<Option<QueuedWorkflowLaunch>, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        if session.has_active_workflow_run() {
            return Ok(None);
        }
        Ok(session.dequeue_workflow_launch())
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
