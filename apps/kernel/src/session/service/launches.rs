use super::*;
use crate::session::WorkflowPublicationInvocationEnvelope;

#[derive(Debug, Clone)]
pub(crate) struct WorkflowRuntimeInstanceProvisionCandidate {
    pub(crate) workflow: WorkflowDefinition,
    pub(crate) endpoint: WorkflowEndpointDefinition,
    pub(crate) ordinal: u16,
    pub(crate) primary: bool,
    pub(crate) source_worktree_id: String,
}

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
        self.invoke_workflow_endpoint_with_context(
            session_id,
            workflow_ref,
            endpoint_ref,
            prompt,
            publication_invocation,
            None,
            None,
            None,
        )
    }

    pub fn invoke_queued_workflow_endpoint(
        &mut self,
        session_id: &str,
        queued_prompt: &WorkflowQueuedPrompt,
    ) -> Result<WorkflowRun, DaemonError> {
        self.invoke_workflow_endpoint_with_context(
            session_id,
            queued_prompt.workflow_id(),
            queued_prompt.endpoint_id(),
            queued_prompt.prompt().map(str::to_string),
            queued_prompt.publication_invocation().cloned(),
            Some(queued_prompt),
            None,
            None,
        )
    }

    fn invoke_queued_workflow_endpoint_on_instance(
        &mut self,
        session_id: &str,
        queued_prompt: &WorkflowQueuedPrompt,
        runtime_instance: &crate::session::WorkflowEndpointRuntimeInstance,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> Result<WorkflowRun, DaemonError> {
        self.invoke_workflow_endpoint_with_context(
            session_id,
            queued_prompt.workflow_id(),
            queued_prompt.endpoint_id(),
            queued_prompt.prompt().map(str::to_string),
            queued_prompt.publication_invocation().cloned(),
            Some(queued_prompt),
            Some(runtime_instance),
            Some((workflow_run_id, workflow_node_run_id)),
        )
    }

    fn invoke_workflow_endpoint_with_context(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        prompt: Option<String>,
        publication_invocation: Option<WorkflowPublicationInvocationEnvelope>,
        queued_prompt: Option<&WorkflowQueuedPrompt>,
        runtime_instance: Option<&crate::session::WorkflowEndpointRuntimeInstance>,
        reserved_run_identity: Option<(&str, &str)>,
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
            reserved_run_identity
                .map(|(_, node_run_id)| node_run_id.to_string())
                .unwrap_or_else(|| self.next_workflow_node_run_id()),
            entry_node.id().to_string(),
            runtime_instance
                .and_then(|instance| instance.agent_id_for_node(entry_node.id()))
                .unwrap_or_else(|| entry_node.agent_id())
                .to_string(),
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
        let mut workflow_run = WorkflowRun::new(
            reserved_run_identity
                .map(|(run_id, _)| run_id.to_string())
                .unwrap_or_else(|| self.next_workflow_run_id()),
            workflow.id().to_string(),
            endpoint.id().to_string(),
            entry_node.id().to_string(),
            prompt,
            publication_invocation,
            vec![node_run],
            messages,
        );
        workflow_run.set_invocation_context(
            workflow.revision(),
            queued_prompt
                .map(|prompt| prompt.queue_id().to_string())
                .or_else(|| {
                    workflow_run
                        .publication_invocation()
                        .and_then(|invocation| invocation.queue_ref.clone())
                }),
            queued_prompt.map(|prompt| prompt.id().to_string()),
            queued_prompt
                .map(WorkflowQueuedPrompt::created_at_ms)
                .unwrap_or_else(|| workflow_run.created_at_ms()),
            queued_prompt.map(WorkflowQueuedPrompt::created_at_ms),
        );
        if let Some(runtime_instance) = runtime_instance {
            workflow_run.set_runtime_instance_context(
                runtime_instance.id().to_string(),
                queued_prompt
                    .map(WorkflowQueuedPrompt::source)
                    .unwrap_or(WorkflowQueuedPromptSource::Manual),
                runtime_instance.node_agent_ids().clone(),
            );
        }
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        if let (Some(runtime_instance), Some((reserved_run_id, _))) =
            (runtime_instance, reserved_run_identity)
        {
            let reservation_is_current = session
                .workflow_runtime_instance(runtime_instance.id())
                .is_some_and(|instance| instance.active_run_id() == Some(reserved_run_id));
            if !reservation_is_current {
                return Err(DaemonError::LocalTransport {
                    operation: "invoke workflow endpoint",
                    message: format!(
                        "workflow runtime instance `{}` lost run reservation `{reserved_run_id}`",
                        runtime_instance.id()
                    ),
                });
            }
        }
        if let Some(runtime_instance) = runtime_instance.filter(|_| reserved_run_identity.is_none())
        {
            session
                .claim_workflow_runtime_instance(runtime_instance.id(), workflow_run.id())
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "invoke workflow endpoint",
                    message: format!(
                        "workflow runtime instance `{}` was no longer idle",
                        runtime_instance.id()
                    ),
                })?;
        }
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

    pub fn mark_workflow_run_settling(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
    ) -> Result<bool, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        Ok(session.mark_workflow_run_settling(workflow_run_id))
    }

    pub fn clear_workflow_run_settling(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
    ) -> Result<(), DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        session.clear_workflow_run_settling(workflow_run_id);
        Ok(())
    }

    pub fn reconcile_live_orphaned_workflow_runs(
        &mut self,
        session_id: &str,
        now_ms: u64,
        grace_period_ms: u64,
    ) -> Result<usize, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        Ok(session.reconcile_live_orphaned_workflow_runs(now_ms, grace_period_ms))
    }

    pub fn has_queued_workflow_prompt_for_watchdog(
        &self,
        session_id: &str,
        watchdog_id: &str,
    ) -> Result<bool, DaemonError> {
        let session = self
            .store
            .get(session_id)
            .ok_or_else(|| DaemonError::SessionNotFound {
                session_id: session_id.to_string(),
            })?;
        Ok(session.workflow_queued_prompts().iter().any(|prompt| {
            prompt.watchdog_id() == Some(watchdog_id)
                && prompt.status() == WorkflowQueuedPromptStatus::Queued
        }))
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
        let queued = WorkflowQueuedPrompt::new(crate::session::WorkflowQueuedPromptInput {
            id: self.next_workflow_queued_prompt_id(),
            queue_id,
            workflow_id: workflow.id().to_string(),
            endpoint_id: endpoint.id().to_string(),
            prompt,
            publication_invocation,
            source,
            schedule_id: watchdog_id,
        });
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        Ok(session.enqueue_workflow_prompt(queued))
    }

    pub fn enqueue_workflow_prompt_and_maybe_create_run(
        &mut self,
        session_id: &str,
        workflow_id: &str,
        endpoint_id: &str,
        prompt: Option<String>,
        queue_ref: Option<&str>,
        source: WorkflowQueuedPromptSource,
        watchdog_id: Option<String>,
        publication_invocation: Option<WorkflowPublicationInvocationEnvelope>,
    ) -> Result<
        (
            WorkflowQueuedPrompt,
            Option<(
                WorkflowQueuedPrompt,
                WorkflowRun,
                WorkflowDefinition,
                WorkflowEndpointDefinition,
            )>,
        ),
        DaemonError,
    > {
        let queued_prompt = self.enqueue_workflow_prompt_with_publication_invocation(
            session_id,
            workflow_id,
            endpoint_id,
            prompt,
            queue_ref,
            source,
            watchdog_id,
            publication_invocation,
        )?;
        let claimed_run = self.dequeue_next_workflow_prompt_and_create_run(session_id)?;
        Ok((queued_prompt, claimed_run))
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
        if session.has_active_metaagent_task() {
            return Ok(None);
        }
        if let (Some(meta_task), Some(workflow_created_at_ms)) = (
            session.queued_metaagent_tasks().front(),
            session.next_workflow_queued_prompt_created_at_ms(),
        ) {
            if meta_task.created_at_ms() <= workflow_created_at_ms {
                return Ok(None);
            }
        } else if !session.queued_metaagent_tasks().is_empty() {
            return Ok(None);
        }
        Ok(session.pop_next_workflow_queued_prompt())
    }

    pub(crate) fn workflow_runtime_instance_provision_candidate(
        &mut self,
        session_id: &str,
    ) -> Result<Option<WorkflowRuntimeInstanceProvisionCandidate>, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        session.reconcile_workflow_runtime_instances();
        let mut queued = session
            .workflow_queued_prompts()
            .iter()
            .filter(|item| item.status() == WorkflowQueuedPromptStatus::Queued)
            .filter_map(|item| {
                let queue = session.workflow_prompt_queue(item.workflow_id(), item.queue_id())?;
                queue
                    .enabled()
                    .then_some((queue.priority(), item.created_at_ms(), item.clone()))
            })
            .collect::<Vec<_>>();
        queued.sort_by_key(|(priority, created_at_ms, _)| {
            (std::cmp::Reverse(*priority), *created_at_ms)
        });
        for (_, _, queued_prompt) in queued {
            let Some(workflow) = session.workflow(queued_prompt.workflow_id()).cloned() else {
                continue;
            };
            let Some(endpoint) = workflow.endpoint(queued_prompt.endpoint_id()).cloned() else {
                continue;
            };
            if session
                .idle_workflow_runtime_instance(workflow.id(), endpoint.id(), workflow.revision())
                .is_some()
            {
                return Ok(None);
            }
            let count = session.current_workflow_runtime_instance_count(
                workflow.id(),
                endpoint.id(),
                workflow.revision(),
            );
            if count >= endpoint.max_instances() as usize {
                continue;
            }
            let ordinal =
                session.next_workflow_runtime_instance_ordinal(workflow.id(), endpoint.id());
            return Ok(Some(WorkflowRuntimeInstanceProvisionCandidate {
                workflow,
                endpoint,
                ordinal,
                primary: count == 0,
                source_worktree_id: session.worktree_id().to_string(),
            }));
        }
        Ok(None)
    }

    pub(crate) fn ensure_primary_workflow_runtime_instance(
        &mut self,
        session_id: &str,
    ) -> Result<Option<crate::session::WorkflowEndpointRuntimeInstance>, DaemonError> {
        let Some(candidate) = self.workflow_runtime_instance_provision_candidate(session_id)?
        else {
            return Ok(None);
        };
        if !candidate.primary {
            return Ok(None);
        }
        let node_agent_ids = candidate
            .workflow
            .nodes()
            .iter()
            .map(|node| (node.id().to_string(), node.agent_id().to_string()))
            .collect::<BTreeMap<_, _>>();
        let instance = crate::session::WorkflowEndpointRuntimeInstance::new(
            format!("workflow-instance-{:032x}", rand::random::<u128>()),
            candidate.workflow.id(),
            candidate.endpoint.id(),
            candidate.workflow.revision(),
            candidate.ordinal,
            true,
            node_agent_ids,
            candidate.source_worktree_id,
        );
        self.register_workflow_runtime_instance(session_id, instance)
            .map(Some)
    }

    pub(crate) fn register_workflow_runtime_instance(
        &mut self,
        session_id: &str,
        instance: crate::session::WorkflowEndpointRuntimeInstance,
    ) -> Result<crate::session::WorkflowEndpointRuntimeInstance, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        if session.workflow_runtime_instance(instance.id()).is_some() {
            return Err(DaemonError::LocalTransport {
                operation: "register workflow runtime instance",
                message: format!("runtime instance `{}` already exists", instance.id()),
            });
        }
        Ok(session.add_workflow_runtime_instance(instance))
    }

    pub(crate) fn remove_workflow_runtime_instance(
        &mut self,
        session_id: &str,
        instance_id: &str,
    ) -> Result<Option<crate::session::WorkflowEndpointRuntimeInstance>, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        Ok(session.remove_workflow_runtime_instance(instance_id))
    }

    pub(crate) fn mark_workflow_runtime_instance_stale(
        &mut self,
        session_id: &str,
        instance_id: &str,
    ) -> Result<Option<crate::session::WorkflowEndpointRuntimeInstance>, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        Ok(session.mark_workflow_runtime_instance_stale(instance_id))
    }

    pub(crate) fn release_workflow_runtime_instance_for_run(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
    ) -> Result<Option<crate::session::WorkflowEndpointRuntimeInstance>, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        Ok(session.release_workflow_runtime_instance_for_run(workflow_run_id))
    }

    pub(crate) fn cleanup_ready_workflow_runtime_instances(
        &mut self,
        session_id: &str,
    ) -> Result<Vec<crate::session::WorkflowEndpointRuntimeInstance>, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        Ok(session.cleanup_ready_workflow_runtime_instances())
    }

    pub fn dequeue_next_workflow_prompt_and_create_run(
        &mut self,
        session_id: &str,
    ) -> Result<
        Option<(
            WorkflowQueuedPrompt,
            WorkflowRun,
            WorkflowDefinition,
            WorkflowEndpointDefinition,
        )>,
        DaemonError,
    > {
        loop {
            let workflow_run_id = self.next_workflow_run_id();
            let workflow_node_run_id = self.next_workflow_node_run_id();
            let (queued_prompt, runtime_instance) = {
                let session =
                    self.store
                        .get_mut(session_id)
                        .ok_or_else(|| DaemonError::SessionNotFound {
                            session_id: session_id.to_string(),
                        })?;
                if session.has_active_metaagent_task() {
                    return Ok(None);
                }
                if let (Some(meta_task), Some(workflow_created_at_ms)) = (
                    session.queued_metaagent_tasks().front(),
                    session.next_dispatchable_workflow_queued_prompt_created_at_ms(),
                ) {
                    if meta_task.created_at_ms() <= workflow_created_at_ms {
                        return Ok(None);
                    }
                } else if !session.queued_metaagent_tasks().is_empty() {
                    return Ok(None);
                }
                let Some(claimed) =
                    session.pop_next_workflow_queued_prompt_with_idle_instance(&workflow_run_id)
                else {
                    return Ok(None);
                };
                claimed
            };
            if let Some(watchdog_id) = queued_prompt.watchdog_id() {
                if !self.prepare_workflow_watchdog_queued_start(session_id, watchdog_id)? {
                    continue;
                }
            }
            let workflow = self.resolve_workflow_ref(session_id, queued_prompt.workflow_id())?;
            let endpoint = self.resolve_workflow_endpoint_ref(
                session_id,
                queued_prompt.workflow_id(),
                queued_prompt.endpoint_id(),
            )?;
            let workflow_run = match self.invoke_queued_workflow_endpoint_on_instance(
                session_id,
                &queued_prompt,
                &runtime_instance,
                &workflow_run_id,
                &workflow_node_run_id,
            ) {
                Ok(workflow_run) => workflow_run,
                Err(error) => {
                    let session = self.store.get_mut(session_id).ok_or_else(|| {
                        DaemonError::SessionNotFound {
                            session_id: session_id.to_string(),
                        }
                    })?;
                    let _ = session.release_workflow_runtime_instance_for_run(&workflow_run_id);
                    let mut queued_prompt = queued_prompt;
                    queued_prompt.mark_queued_for_retry();
                    session.enqueue_workflow_prompt(queued_prompt);
                    return Err(error);
                }
            };
            return Ok(Some((queued_prompt, workflow_run, workflow, endpoint)));
        }
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
