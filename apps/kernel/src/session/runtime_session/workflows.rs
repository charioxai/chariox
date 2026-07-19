use super::*;

impl RuntimeSession {
    pub fn create_workflow(&mut self, workflow: WorkflowDefinition) -> WorkflowDefinition {
        let workflow_id = workflow.id().to_string();
        self.workflows.push(workflow.clone());
        self.ensure_default_workflow_prompt_queue(&workflow_id);
        workflow
    }

    pub(crate) fn replace_publication_runtime_workflows(
        &mut self,
        workflows: Vec<WorkflowDefinition>,
        workflow_prompt_queues: Vec<WorkflowPromptQueueDefinition>,
        workflow_schedules: Vec<WorkflowScheduleDefinition>,
    ) {
        self.workflows = workflows;
        self.workflow_prompt_queues = workflow_prompt_queues;
        self.workflow_schedules = workflow_schedules;
        let workflow_ids = self
            .workflows
            .iter()
            .map(|workflow| workflow.id().to_string())
            .collect::<Vec<_>>();
        for workflow_id in workflow_ids {
            self.ensure_default_workflow_prompt_queue(&workflow_id);
        }
    }

    pub fn remove_workflow(&mut self, workflow_id: &str) -> Option<WorkflowDefinition> {
        let index = self
            .workflows
            .iter()
            .position(|workflow| workflow.id() == workflow_id)?;
        Some(self.workflows.remove(index))
    }

    pub fn workflow(&self, workflow_id: &str) -> Option<&WorkflowDefinition> {
        self.workflows
            .iter()
            .find(|workflow| workflow.id() == workflow_id)
    }

    pub fn workflow_mut(&mut self, workflow_id: &str) -> Option<&mut WorkflowDefinition> {
        self.workflows
            .iter_mut()
            .find(|workflow| workflow.id() == workflow_id)
    }

    pub fn create_workflow_publication(
        &mut self,
        publication: WorkflowPublicationDefinition,
        source_snapshot: Option<WorkflowPublicationSnapshot>,
    ) -> WorkflowPublicationDefinition {
        if let Some(source_snapshot) = source_snapshot {
            self.workflow_publication_state
                .workflow_publication_snapshots
                .insert(publication.id().to_string(), source_snapshot);
        }
        self.workflow_publication_state
            .workflow_publications
            .push(publication.clone());
        publication
    }

    pub fn workflow_publication(
        &self,
        publication_id: &str,
    ) -> Option<&WorkflowPublicationDefinition> {
        self.workflow_publication_state
            .workflow_publications
            .iter()
            .find(|publication| publication.id() == publication_id)
    }

    pub fn workflow_publication_mut(
        &mut self,
        publication_id: &str,
    ) -> Option<&mut WorkflowPublicationDefinition> {
        self.workflow_publication_state
            .workflow_publications
            .iter_mut()
            .find(|publication| publication.id() == publication_id)
    }

    pub fn create_workflow_run(&mut self, workflow_run: WorkflowRun) -> WorkflowRun {
        self.workflow_runs.push(workflow_run.clone());
        workflow_run
    }

    pub fn has_active_workflow_run(&self) -> bool {
        self.workflow_runs.iter().any(|workflow_run| {
            matches!(
                workflow_run.status(),
                WorkflowRunStatus::Created
                    | WorkflowRunStatus::Running
                    | WorkflowRunStatus::Waiting
            )
        })
    }

    pub fn reconcile_after_kernel_restart(&mut self) -> KernelRestartReconciliation {
        let mut reconciliation = KernelRestartReconciliation::default();
        if self.active_provider_run_id.take().is_some() {
            reconciliation.cleared_active_provider_run = true;
        }
        reconciliation.cleared_attachment_count = self.clear_attachments();
        reconciliation.recoverable_prompt_count = self
            .prompt_runtime
            .prompt_states()
            .values()
            .filter(|state| state.active_prompt().is_some())
            .count();
        reconciliation.recoverable_workflow_run_count = self
            .workflow_runs
            .iter()
            .filter(|workflow_run| {
                !matches!(
                    workflow_run.status(),
                    WorkflowRunStatus::Completed
                        | WorkflowRunStatus::Failed
                        | WorkflowRunStatus::Stopped
                )
            })
            .count();

        let durable_workflow_prompt_targets = self
            .prompt_runtime
            .prompt_states()
            .values()
            .flat_map(|state| {
                state
                    .active_prompt()
                    .into_iter()
                    .chain(state.queued_prompts())
            })
            .filter_map(|prompt| {
                Some((
                    prompt.workflow_run_id()?.to_string(),
                    prompt.workflow_node_run_id()?.to_string(),
                ))
            })
            .collect::<BTreeSet<_>>();
        let mut orphaned_prepared_workflow_run_ids = Vec::new();
        for workflow_run in &mut self.workflow_runs {
            let Some(active_node_run_id) = workflow_run.active_node_run_id().map(str::to_string)
            else {
                continue;
            };
            if durable_workflow_prompt_targets
                .contains(&(workflow_run.id().to_string(), active_node_run_id.clone()))
            {
                continue;
            }
            let Some(node_run) = workflow_run.node_run_mut(&active_node_run_id) else {
                continue;
            };
            let orphaned_prepared_turn = node_run.status() == WorkflowNodeRunStatus::Ready
                && node_run.turn_envelope().is_some_and(|envelope| {
                    envelope.state() == crate::session::WorkflowTurnRuntimeState::Prepared
                });
            if !orphaned_prepared_turn {
                continue;
            }
            node_run.set_status(WorkflowNodeRunStatus::Stopped);
            if let Some(envelope) = node_run.turn_envelope_mut() {
                envelope.mark_cancelled();
            }
            workflow_run.clear_active_node_run();
            workflow_run.add_failure_event(WorkflowFailureEvent::new(
                WorkflowFailureKind::RunStopped,
                active_node_run_id,
                Vec::new(),
                "prepared workflow turn had no durable active or queued prompt after kernel restart",
            ));
            workflow_run.set_status(WorkflowRunStatus::Stopped);
            orphaned_prepared_workflow_run_ids.push(workflow_run.id().to_string());
            reconciliation.stopped_workflow_run_count += 1;
        }
        for workflow_run_id in orphaned_prepared_workflow_run_ids {
            self.prompt_runtime.remove_queued_prompts_by_workflow_run(
                &workflow_run_id,
                self.focused_agent_id.as_deref(),
            );
        }

        reconciliation
    }

    pub(crate) fn interrupt_runtime_for_shutdown(&mut self) -> KernelRestartReconciliation {
        let mut reconciliation = self.reconcile_after_kernel_restart();

        reconciliation.interrupted_prompt_count = self
            .prompt_runtime
            .interrupt_active_prompts(self.focused_agent_id.as_deref())
            .len();

        let mut stopped_workflow_run_ids = Vec::new();
        for workflow_run in &mut self.workflow_runs {
            let should_stop = !matches!(
                workflow_run.status(),
                WorkflowRunStatus::Completed
                    | WorkflowRunStatus::Failed
                    | WorkflowRunStatus::Stopped
            );
            if !should_stop {
                continue;
            }

            let source_node_run_id = workflow_run
                .active_node_run_id()
                .map(str::to_string)
                .or_else(|| {
                    workflow_run
                        .node_runs()
                        .iter()
                        .find(|node_run| {
                            !matches!(
                                node_run.status(),
                                WorkflowNodeRunStatus::Completed
                                    | WorkflowNodeRunStatus::Failed
                                    | WorkflowNodeRunStatus::Stopped
                            )
                        })
                        .map(|node_run| node_run.id().to_string())
                })
                .unwrap_or_else(|| workflow_run.id().to_string());

            for node_run in workflow_run.node_runs_mut() {
                if !matches!(
                    node_run.status(),
                    WorkflowNodeRunStatus::Completed
                        | WorkflowNodeRunStatus::Failed
                        | WorkflowNodeRunStatus::Stopped
                ) {
                    node_run.set_status(WorkflowNodeRunStatus::Stopped);
                    if let Some(envelope) = node_run.turn_envelope_mut() {
                        envelope.mark_cancelled();
                    }
                }
            }
            workflow_run.clear_active_node_run();
            workflow_run.add_failure_event(WorkflowFailureEvent::new(
                WorkflowFailureKind::RunStopped,
                source_node_run_id,
                Vec::new(),
                "workflow run was interrupted by kernel restart; relaunch or resume it explicitly",
            ));
            workflow_run.set_status(WorkflowRunStatus::Stopped);
            stopped_workflow_run_ids.push(workflow_run.id().to_string());
            reconciliation.stopped_workflow_run_count += 1;
        }
        for workflow_run_id in stopped_workflow_run_ids {
            self.remove_queued_prompts_by_workflow_run(&workflow_run_id);
        }

        reconciliation
    }

    pub fn add_workflow_prompt_queue(
        &mut self,
        queue: WorkflowPromptQueueDefinition,
    ) -> WorkflowPromptQueueDefinition {
        self.workflow_prompt_queues.push(queue.clone());
        queue
    }

    pub fn workflow_prompt_queues_for_workflow(
        &self,
        workflow_id: &str,
    ) -> Vec<WorkflowPromptQueueDefinition> {
        self.workflow_prompt_queues
            .iter()
            .filter(|queue| queue.workflow_id() == workflow_id)
            .cloned()
            .collect()
    }

    pub fn workflow_prompt_queue(
        &self,
        workflow_id: &str,
        queue_id: &str,
    ) -> Option<&WorkflowPromptQueueDefinition> {
        self.workflow_prompt_queues.iter().find(|queue| {
            queue.workflow_id() == workflow_id
                && (queue.id() == queue_id || queue.alias() == queue_id)
        })
    }

    pub fn workflow_prompt_queue_mut(
        &mut self,
        workflow_id: &str,
        queue_id: &str,
    ) -> Option<&mut WorkflowPromptQueueDefinition> {
        self.workflow_prompt_queues.iter_mut().find(|queue| {
            queue.workflow_id() == workflow_id
                && (queue.id() == queue_id || queue.alias() == queue_id)
        })
    }

    pub fn remove_workflow_prompt_queue(
        &mut self,
        workflow_id: &str,
        queue_id: &str,
    ) -> Option<WorkflowPromptQueueDefinition> {
        let index = self.workflow_prompt_queues.iter().position(|queue| {
            queue.workflow_id() == workflow_id
                && (queue.id() == queue_id || queue.alias() == queue_id)
        })?;
        Some(self.workflow_prompt_queues.remove(index))
    }

    pub fn ensure_default_workflow_prompt_queue(&mut self, workflow_id: &str) {
        if self
            .workflow_prompt_queues
            .iter()
            .any(|queue| queue.workflow_id() == workflow_id && queue.alias() == "default")
        {
            return;
        }
        self.workflow_prompt_queues
            .push(WorkflowPromptQueueDefinition::default_queue(workflow_id));
    }

    pub fn enqueue_workflow_prompt(
        &mut self,
        queued_prompt: WorkflowQueuedPrompt,
    ) -> WorkflowQueuedPrompt {
        self.workflow_queued_prompts
            .push_back(queued_prompt.clone());
        queued_prompt
    }

    pub fn update_queued_workflow_prompt(
        &mut self,
        queue_item_id: &str,
        prompt: Option<String>,
        queue_id: Option<String>,
    ) -> Option<WorkflowQueuedPrompt> {
        let queued_prompt = self
            .workflow_queued_prompts
            .iter_mut()
            .find(|item| item.id() == queue_item_id)?;
        if queued_prompt.status() != WorkflowQueuedPromptStatus::Queued {
            return None;
        }
        if let Some(queue_id) = queue_id {
            queued_prompt.set_queue_id(queue_id);
        }
        queued_prompt.set_prompt(prompt);
        Some(queued_prompt.clone())
    }

    pub fn remove_queued_workflow_prompt(
        &mut self,
        queue_item_id: &str,
    ) -> Option<WorkflowQueuedPrompt> {
        let index = self
            .workflow_queued_prompts
            .iter()
            .position(|queued_prompt| queued_prompt.id() == queue_item_id)?;
        if self.workflow_queued_prompts[index].status() != WorkflowQueuedPromptStatus::Queued {
            return None;
        }
        self.workflow_queued_prompts.remove(index)
    }

    pub fn remove_queued_workflow_prompts_for_watchdog(
        &mut self,
        watchdog_id: &str,
    ) -> Vec<WorkflowQueuedPrompt> {
        let mut removed = Vec::new();
        self.workflow_queued_prompts.retain(|prompt| {
            if prompt.watchdog_id() == Some(watchdog_id) {
                removed.push(prompt.clone());
                false
            } else {
                true
            }
        });
        removed
    }

    pub fn clear_workflow_queue(&mut self, queue_id: &str) -> Vec<WorkflowQueuedPrompt> {
        let mut removed = Vec::new();
        let mut kept = VecDeque::new();
        while let Some(item) = self.workflow_queued_prompts.pop_front() {
            if item.queue_id() == queue_id && item.status() == WorkflowQueuedPromptStatus::Queued {
                removed.push(item);
            } else {
                kept.push_back(item);
            }
        }
        self.workflow_queued_prompts = kept;
        removed
    }

    pub fn pop_next_workflow_queued_prompt(&mut self) -> Option<WorkflowQueuedPrompt> {
        let best = self
            .workflow_queued_prompts
            .iter()
            .enumerate()
            .filter(|(_, item)| item.status() == WorkflowQueuedPromptStatus::Queued)
            .filter_map(|(index, item)| {
                let queue = self.workflow_prompt_queue(item.workflow_id(), item.queue_id())?;
                if !queue.enabled() {
                    return None;
                }
                Some((index, queue.priority(), item.created_at_ms()))
            })
            .min_by_key(|(_, priority, created_at_ms)| {
                (std::cmp::Reverse(*priority), *created_at_ms)
            })
            .map(|(index, _, _)| index)?;
        let mut item = self.workflow_queued_prompts.remove(best)?;
        item.mark_dispatching();
        Some(item)
    }

    pub fn workflow_run(&self, workflow_run_id: &str) -> Option<&WorkflowRun> {
        self.workflow_runs
            .iter()
            .find(|workflow_run| workflow_run.id() == workflow_run_id)
    }

    pub fn workflow_run_mut(&mut self, workflow_run_id: &str) -> Option<&mut WorkflowRun> {
        self.workflow_runs
            .iter_mut()
            .find(|workflow_run| workflow_run.id() == workflow_run_id)
    }

    pub fn add_workflow_schedule(
        &mut self,
        schedule: WorkflowScheduleDefinition,
    ) -> WorkflowScheduleDefinition {
        self.workflow_schedules.push(schedule.clone());
        schedule
    }

    pub fn workflow_schedule(&self, schedule_id: &str) -> Option<&WorkflowScheduleDefinition> {
        self.workflow_schedules
            .iter()
            .find(|schedule| schedule.id() == schedule_id)
    }

    pub fn workflow_schedule_mut(
        &mut self,
        schedule_id: &str,
    ) -> Option<&mut WorkflowScheduleDefinition> {
        self.workflow_schedules
            .iter_mut()
            .find(|schedule| schedule.id() == schedule_id)
    }

    pub fn remove_workflow_schedule(
        &mut self,
        schedule_id: &str,
    ) -> Option<WorkflowScheduleDefinition> {
        let index = self
            .workflow_schedules
            .iter()
            .position(|schedule| schedule.id() == schedule_id)?;
        Some(self.workflow_schedules.remove(index))
    }

    pub fn add_workflow_watchdog(
        &mut self,
        watchdog: WorkflowWatchdogDefinition,
    ) -> WorkflowWatchdogDefinition {
        self.add_workflow_schedule(watchdog)
    }

    pub fn workflow_watchdog(&self, watchdog_id: &str) -> Option<&WorkflowWatchdogDefinition> {
        self.workflow_schedule(watchdog_id)
    }

    pub fn workflow_watchdog_mut(
        &mut self,
        watchdog_id: &str,
    ) -> Option<&mut WorkflowWatchdogDefinition> {
        self.workflow_schedule_mut(watchdog_id)
    }

    pub fn remove_workflow_watchdog(
        &mut self,
        watchdog_id: &str,
    ) -> Option<WorkflowWatchdogDefinition> {
        self.remove_workflow_schedule(watchdog_id)
    }

    pub fn workflow_node_run_mut(
        &mut self,
        workflow_node_run_id: &str,
    ) -> Option<&mut WorkflowNodeRun> {
        self.workflow_runs
            .iter_mut()
            .find_map(|workflow_run| workflow_run.node_run_mut(workflow_node_run_id))
    }

    pub fn workflow_console(&self, workflow_id: &str) -> Option<&WorkflowConsole> {
        self.workflow_consoles
            .iter()
            .find(|console| console.workflow_id() == workflow_id)
    }

    pub fn workflow_console_mut(&mut self, workflow_id: &str) -> Option<&mut WorkflowConsole> {
        self.workflow_consoles
            .iter_mut()
            .find(|console| console.workflow_id() == workflow_id)
    }

    pub fn ensure_workflow_console(
        &mut self,
        workflow_id: impl Into<String>,
    ) -> &mut WorkflowConsole {
        let workflow_id = workflow_id.into();
        if let Some(index) = self
            .workflow_consoles
            .iter()
            .position(|console| console.workflow_id() == workflow_id)
        {
            return &mut self.workflow_consoles[index];
        }
        self.workflow_consoles
            .push(WorkflowConsole::new(workflow_id));
        let index = self.workflow_consoles.len() - 1;
        &mut self.workflow_consoles[index]
    }
}
