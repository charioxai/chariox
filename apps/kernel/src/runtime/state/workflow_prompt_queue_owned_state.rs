//! Workflow prompt queue advancement.

use super::*;

const LIVE_WORKFLOW_ORPHAN_GRACE_PERIOD_MS: u64 = 5_000;

impl KernelRuntimeOwnedState {
    fn workflow_ensure_dispatchable_runtime_instance(
        &self,
        session_id: &str,
    ) -> Result<bool, DaemonError> {
        let _provision_guard = self
            .workflow_instance_provision_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.workflow_cleanup_runtime_instances(session_id)?;
        let Some(candidate) = self
            .session_store
            .write()
            .workflow_runtime_instance_provision_candidate(session_id)?
        else {
            return Ok(false);
        };

        if candidate.primary {
            let instance = self
                .session_store
                .write()
                .ensure_primary_workflow_runtime_instance(session_id)?;
            let Some(instance) = instance else {
                return Ok(true);
            };
            if let Err(error) =
                self.persist_workflow_runtime_session(session_id, "workflow_instance_provisioned")
            {
                let _ = self
                    .session_store
                    .write()
                    .remove_workflow_runtime_instance(session_id, instance.id());
                return Err(error);
            }
            return Ok(true);
        }

        let instance_id = format!("workflow-instance-{:032x}", rand::random::<u128>());
        let mut node_agent_ids = std::collections::BTreeMap::new();
        let mut materialized_agents = Vec::new();
        let worktree_id = {
            let worktree_root = self
                .config_projection
                .snapshot()
                .workflow_runtime_artifact_root()
                .join("instances")
                .join(session_id);
            std::fs::create_dir_all(&worktree_root).map_err(|error| {
                DaemonError::LocalTransport {
                    operation: "provision workflow runtime instance",
                    message: format!(
                        "failed to create workflow instance directory `{}`: {error}",
                        worktree_root.display()
                    ),
                }
            })?;
            let target = worktree_root.join(&instance_id);
            let placement = crate::agent::GitWorktreePlacement {
                target_directory: Some(target.display().to_string()),
                branch: None,
                from_ref: Some("HEAD".to_string()),
            };
            let worktree_id =
                crate::git_worktree_placement::prepare_workflow_runtime_worktree_or_reuse_directory(
                    &placement,
                    std::path::Path::new(&candidate.source_worktree_id),
                    None,
                    "provision workflow runtime instance",
                )?;
            let mut agent_id_map: BTreeMap<String, String> = BTreeMap::new();
            for node in candidate.workflow.nodes() {
                let runtime_agent_id = if let Some(agent_id) = agent_id_map.get(node.agent_id()) {
                    agent_id.clone()
                } else {
                    let source_agent = match self.agent_store.get_agent(node.agent_id()) {
                        Ok(source_agent) => source_agent,
                        Err(error) => {
                            self.workflow_rollback_runtime_instance_provision(
                                &candidate.source_worktree_id,
                                &worktree_id,
                                false,
                                &materialized_agents,
                            );
                            return Err(error);
                        }
                    };
                    let materialized = self.agent_store.materialize_workflow_runtime_agent(
                        source_agent,
                        session_id,
                        &worktree_id,
                    );
                    materialized_agents.push(materialized.clone());
                    agent_id_map.insert(node.agent_id().to_string(), materialized.id().to_string());
                    materialized.id().to_string()
                };
                node_agent_ids.insert(node.id().to_string(), runtime_agent_id);
            }
            worktree_id
        };

        let instance = crate::session::WorkflowEndpointRuntimeInstance::new(
            instance_id.clone(),
            candidate.workflow.id(),
            candidate.endpoint.id(),
            candidate.workflow.revision(),
            candidate.ordinal,
            false,
            node_agent_ids,
            worktree_id.clone(),
        );
        if let Err(error) = self
            .session_store
            .write()
            .register_workflow_runtime_instance(session_id, instance)
        {
            self.workflow_rollback_runtime_instance_provision(
                &candidate.source_worktree_id,
                &worktree_id,
                false,
                &materialized_agents,
            );
            return Err(error);
        }
        if let Err(error) =
            self.persist_workflow_runtime_session(session_id, "workflow_instance_provisioned")
        {
            let _ = self
                .session_store
                .write()
                .remove_workflow_runtime_instance(session_id, &instance_id);
            self.workflow_rollback_runtime_instance_provision(
                &candidate.source_worktree_id,
                &worktree_id,
                false,
                &materialized_agents,
            );
            return Err(error);
        }
        if !materialized_agents.is_empty() {
            if let Err(error) = self.durable_state_store.append_event(
                "agents.created",
                Some(session_id.to_string()),
                serde_json::json!({
                    "session_id": session_id,
                    "agents": &materialized_agents,
                }),
            ) {
                let _ = self
                    .session_store
                    .write()
                    .remove_workflow_runtime_instance(session_id, &instance_id);
                let _ = self.persist_workflow_runtime_session(
                    session_id,
                    "workflow_instance_provision_rollback",
                );
                self.workflow_rollback_runtime_instance_provision(
                    &candidate.source_worktree_id,
                    &worktree_id,
                    false,
                    &materialized_agents,
                );
                return Err(error);
            }
        }
        Ok(true)
    }

    pub(super) fn workflow_cleanup_runtime_instances(
        &self,
        session_id: &str,
    ) -> Result<(), DaemonError> {
        let (source_worktree_id, missing_agent_instances) = {
            let sessions = self.session_store.read();
            let session = sessions.get_session(session_id)?;
            let missing = session
                .workflow_runtime_instances()
                .iter()
                .filter(|instance| !instance.primary())
                .filter(|instance| {
                    instance
                        .node_agent_ids()
                        .values()
                        .any(|agent_id| self.agent_store.get_agent(agent_id).is_err())
                })
                .map(|instance| {
                    (
                        instance.id().to_string(),
                        instance.active_run_id().map(str::to_string),
                    )
                })
                .collect::<Vec<_>>();
            (session.worktree_id().to_string(), missing)
        };
        self.workflow_cleanup_orphaned_runtime_worktrees(session_id, &source_worktree_id);
        for (instance_id, active_run_id) in missing_agent_instances {
            if let Some(active_run_id) = active_run_id {
                let node_run_id = self
                    .session_store
                    .read()
                    .resolve_workflow_run_ref(session_id, &active_run_id)
                    .ok()
                    .and_then(|run| run.node_runs().first().map(|node| node.id().to_string()));
                if let Some(node_run_id) = node_run_id {
                    let _ = self.session_store.write().fail_workflow_node_run(
                        session_id,
                        &active_run_id,
                        &node_run_id,
                    );
                } else {
                    let _ = self
                        .session_store
                        .write()
                        .release_workflow_runtime_instance_for_run(session_id, &active_run_id);
                }
            }
            self.session_store
                .write()
                .mark_workflow_runtime_instance_stale(session_id, &instance_id)?;
        }
        let cleanup_ready = self
            .session_store
            .write()
            .cleanup_ready_workflow_runtime_instances(session_id)?;
        if cleanup_ready.is_empty() {
            return Ok(());
        }
        for instance in &cleanup_ready {
            if !instance.primary() {
                let agent_ids = instance
                    .node_agent_ids()
                    .values()
                    .cloned()
                    .collect::<std::collections::BTreeSet<_>>();
                for agent_id in agent_ids {
                    if let Some(run) = self.provider_store.get_run_for_agent(session_id, &agent_id)
                    {
                        if run.state() != crate::provider::ProviderRunState::Ended {
                            let ended = self
                                .provider_store
                                .terminate_run_provider_only(session_id, run.id())?
                                .into_run();
                            self.provider_run_projection.update(ended.clone());
                            self.remove_provider_process_tracking_for_run(ended.id(), None);
                        }
                    }
                    self.clear_agent_prompt_runtime_state(session_id, &agent_id);
                    self.prompt_state_owner.remove_agent(session_id, &agent_id);
                    self.external_provider_sessions
                        .detach_agent(session_id, &agent_id);
                    self.attached_provider_transcript_cursors
                        .detach_agent(session_id, &agent_id);
                    if let Ok(agent) = self.agent_store.get_agent(&agent_id) {
                        let Some(removed_agent) =
                            self.agent_store.remove_workflow_runtime_agent(&agent_id)
                        else {
                            return Err(DaemonError::LocalTransport {
                                operation: "cleanup workflow runtime instance",
                                message: format!("runtime agent `{agent_id}` remained active"),
                            });
                        };
                        if let Err(error) = self.durable_state_store.append_event(
                            "agent.deleted",
                            Some(agent.session_id().to_string()),
                            serde_json::json!({ "agent": &agent }),
                        ) {
                            self.agent_store.restore_agent(removed_agent);
                            return Err(error);
                        }
                    }
                }
                crate::git_worktree_placement::remove_workflow_runtime_worktree(
                    &source_worktree_id,
                    instance.worktree_id(),
                    "cleanup workflow runtime instance",
                )?;
            }
            self.session_store
                .write()
                .remove_workflow_runtime_instance(session_id, instance.id())?;
        }
        self.persist_workflow_runtime_session(session_id, "workflow_instances_cleaned")?;
        let instance_root = self
            .config_projection
            .snapshot()
            .workflow_runtime_artifact_root()
            .join("instances")
            .join(session_id);
        let _ = std::fs::remove_dir(&instance_root);
        Ok(())
    }

    pub(super) fn workflow_cleanup_runtime_instances_exclusive(
        &self,
        session_id: &str,
    ) -> Result<(), DaemonError> {
        let _provision_guard = self
            .workflow_instance_provision_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.workflow_cleanup_runtime_instances(session_id)
    }

    fn workflow_cleanup_orphaned_runtime_worktrees(
        &self,
        session_id: &str,
        source_worktree_id: &str,
    ) {
        let instance_root = self
            .config_projection
            .snapshot()
            .workflow_runtime_artifact_root()
            .join("instances")
            .join(session_id);
        let referenced_worktrees = self
            .session_store
            .read()
            .get_session(session_id)
            .map(|session| {
                session
                    .workflow_runtime_instances()
                    .iter()
                    .filter(|instance| !instance.primary())
                    .map(|instance| std::path::PathBuf::from(instance.worktree_id()))
                    .collect::<std::collections::BTreeSet<_>>()
            })
            .unwrap_or_default();
        let Ok(entries) = std::fs::read_dir(&instance_root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if referenced_worktrees.contains(&path) {
                continue;
            }
            if let Err(error) = crate::git_worktree_placement::remove_git_worktree(
                source_worktree_id,
                &path,
                "cleanup orphaned workflow runtime instance",
            ) {
                let fallback = std::fs::symlink_metadata(&path).and_then(|metadata| {
                    if metadata.file_type().is_symlink() {
                        std::fs::remove_file(&path)
                    } else {
                        std::fs::remove_dir_all(&path)
                    }
                });
                if fallback.is_ok() {
                    let _ = crate::git_worktree_placement::remove_git_worktree(
                        source_worktree_id,
                        &path,
                        "prune orphaned workflow runtime instance",
                    );
                    continue;
                }
                crate::logging::warn_with_fields(
                    "daemon.runtime",
                    "orphaned workflow runtime worktree cleanup failed",
                    serde_json::json!({
                        "session_id": session_id,
                        "worktree": path,
                        "error": error.to_string(),
                    }),
                );
            }
        }
        let _ = std::fs::remove_dir(&instance_root);
    }

    fn workflow_rollback_runtime_instance_provision(
        &self,
        source_worktree_id: &str,
        worktree_id: &str,
        primary: bool,
        materialized_agents: &[crate::agent::AgentInstance],
    ) {
        for agent in materialized_agents.iter().rev() {
            let _ = self.agent_store.remove_workflow_runtime_agent(agent.id());
        }
        if !primary {
            let _ = crate::git_worktree_placement::remove_workflow_runtime_worktree(
                source_worktree_id,
                worktree_id,
                "rollback workflow runtime instance",
            );
        }
    }

    fn workflow_reconcile_live_orphans(&self, session_id: &str) {
        let reconciled = self
            .session_store
            .write()
            .reconcile_live_orphaned_workflow_runs(
                session_id,
                crate::session::unix_epoch_ms(),
                LIVE_WORKFLOW_ORPHAN_GRACE_PERIOD_MS,
            );
        let count = match reconciled {
            Ok(count) => count,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.runtime",
                    "live workflow orphan reconciliation failed",
                    serde_json::json!({
                        "session_id": session_id,
                        "error": error.to_string(),
                    }),
                );
                return;
            }
        };
        if count == 0 {
            return;
        }
        self.record_notice(
            session_id,
            None,
            self.attachment_store
                .list_session_attachment_ids(session_id),
            format!("Stopped {count} orphaned workflow run(s) so queued prompts can advance."),
        );
        if let Err(error) =
            self.persist_workflow_runtime_session(session_id, "workflow_live_orphan_reconciled")
        {
            crate::logging::warn_with_fields(
                "daemon.runtime",
                "live workflow orphan reconciliation persistence failed",
                serde_json::json!({
                    "session_id": session_id,
                    "error": error.to_string(),
                }),
            );
        }
    }

    pub(super) fn workflow_collect_due_watchdog_dispatches(
        &self,
        now_ms: u64,
    ) -> WorkflowPromptDispatches {
        let collection = match self
            .session_store
            .write()
            .collect_due_workflow_watchdog_invocations_with_changes(now_ms)
        {
            Ok(collection) => collection,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.runtime",
                    "workflow watchdog collection failed",
                    serde_json::json!({ "error": error.to_string() }),
                );
                return WorkflowPromptDispatches::default();
            }
        };

        let mut changed_session_ids = collection.changed_session_ids;
        let mut dispatches = WorkflowPromptDispatches::default();
        for plan in collection.plans {
            let session_id = plan.session_id.clone();
            let watchdog_id = plan.watchdog_id.clone();
            changed_session_ids.insert(session_id.clone());
            match self.workflow_start_watchdog_tick_plan(plan) {
                Ok(next_dispatches) => dispatches.extend(next_dispatches),
                Err(error) => {
                    self.mark_workflow_watchdog_launch_failed(&session_id, &watchdog_id, &error);
                    self.record_notice(
                        &session_id,
                        None,
                        self.attachment_store
                            .list_session_attachment_ids(&session_id),
                        format!("Workflow watchdog `{watchdog_id}` failed to launch: {error}"),
                    );
                }
            }
        }
        self.persist_workflow_watchdog_sessions(changed_session_ids);
        dispatches
    }

    fn workflow_start_watchdog_tick_plan(
        &self,
        plan: crate::session::WorkflowWatchdogTickPlan,
    ) -> Result<WorkflowPromptDispatches, DaemonError> {
        self.workflow_reconcile_live_orphans(&plan.session_id);
        let should_start = self
            .session_store
            .read()
            .workflow_watchdog_can_start(&plan.session_id, &plan.watchdog_id)?;
        if !should_start {
            return Ok(WorkflowPromptDispatches::default());
        }
        if plan.enqueue_prompt {
            self.session_store.write().enqueue_workflow_prompt(
                &plan.session_id,
                &plan.workflow_id,
                &plan.endpoint_id,
                Some(plan.invocation_prompt.clone()),
                plan.queue_id.as_deref(),
                crate::session::WorkflowQueuedPromptSource::Scheduled,
                Some(plan.watchdog_id.clone()),
            )?;
        }
        let outcome = self.workflow_start_next_queued_prompt_for_response(&plan.session_id)?;
        if self
            .session_store
            .read()
            .has_queued_workflow_prompt_for_watchdog(&plan.session_id, &plan.watchdog_id)?
        {
            let _ = self
                .session_store
                .write()
                .mark_workflow_watchdog_queued(&plan.session_id, &plan.watchdog_id);
        }
        Ok(outcome
            .map(|(_, dispatches)| dispatches)
            .unwrap_or_default())
    }

    pub(super) fn workflow_enqueue_prompt_and_maybe_start(
        &self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        prompt: Option<String>,
        queue_ref: Option<&str>,
        publication_invocation: Option<crate::session::WorkflowPublicationInvocationEnvelope>,
    ) -> Result<
        (
            crate::app::workflow_runtime::WorkflowLaunchOutcome,
            WorkflowPromptDispatches,
        ),
        DaemonError,
    > {
        self.workflow_reconcile_live_orphans(session_id);
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, workflow_ref)?;
        let endpoint = self.session_store.read().resolve_workflow_endpoint_ref(
            session_id,
            workflow.id(),
            endpoint_ref,
        )?;
        self.workflow_validate_agents(session_id, &workflow)?;
        let queued_prompt = self
            .session_store
            .write()
            .enqueue_workflow_prompt_with_publication_invocation(
                session_id,
                workflow.id(),
                endpoint.id(),
                prompt,
                queue_ref,
                crate::session::WorkflowQueuedPromptSource::Manual,
                None,
                publication_invocation,
            )?;
        let claimed = self.workflow_start_next_queued_prompt_for_response(session_id)?;
        let Some((claimed_outcome, dispatches)) = claimed else {
            return Ok((
                crate::app::workflow_runtime::WorkflowLaunchOutcome::Enqueued {
                    queued_prompt: Box::new(queued_prompt),
                    workflow,
                    endpoint,
                },
                WorkflowPromptDispatches::default(),
            ));
        };
        let claimed_requested_prompt = match &claimed_outcome {
            crate::app::workflow_runtime::WorkflowLaunchOutcome::Started {
                workflow_run, ..
            } => workflow_run.queue_item_id() == Some(queued_prompt.id()),
            crate::app::workflow_runtime::WorkflowLaunchOutcome::Enqueued {
                queued_prompt: claimed,
                ..
            } => claimed.id() == queued_prompt.id(),
        };
        if claimed_requested_prompt {
            Ok((claimed_outcome, dispatches))
        } else {
            Ok((
                crate::app::workflow_runtime::WorkflowLaunchOutcome::Enqueued {
                    queued_prompt: Box::new(queued_prompt),
                    workflow,
                    endpoint,
                },
                dispatches,
            ))
        }
    }

    fn workflow_schedule_queued_prompt_run(
        &self,
        session_id: &str,
        queued_prompt: crate::session::WorkflowQueuedPrompt,
        workflow_run: crate::session::WorkflowRun,
        workflow: crate::session::WorkflowDefinition,
        endpoint: crate::session::WorkflowEndpointDefinition,
    ) -> Result<
        (
            crate::app::workflow_runtime::WorkflowLaunchOutcome,
            WorkflowPromptDispatches,
        ),
        DaemonError,
    > {
        if let Some(watchdog_id) = queued_prompt.watchdog_id() {
            let _ = self.session_store.write().mark_workflow_watchdog_invoked(
                session_id,
                watchdog_id,
                workflow_run.id(),
            );
        }
        let dispatches = match self
            .workflow_validate_agents(session_id, &workflow)
            .and_then(|()| self.workflow_schedule_entry_node(session_id, &workflow_run))
        {
            Ok(dispatches) => dispatches,
            Err(error) => {
                if let Some(node_run) = workflow_run.node_runs().first() {
                    let _ = self.session_store.write().record_workflow_failure_event(
                        session_id,
                        workflow_run.id(),
                        crate::session::WorkflowFailureEvent::new(
                            crate::session::WorkflowFailureKind::TransportFailure,
                            node_run.id(),
                            Vec::new(),
                            error.to_string(),
                        ),
                    );
                    let _ = self.session_store.write().fail_workflow_node_run(
                        session_id,
                        workflow_run.id(),
                        node_run.id(),
                    );
                }
                let _ = self
                    .session_store
                    .write()
                    .release_workflow_runtime_instance_for_run(session_id, workflow_run.id());
                return Err(error);
            }
        };
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run.id())?;
        Ok((
            crate::app::workflow_runtime::WorkflowLaunchOutcome::Started {
                workflow_run: Box::new(workflow_run),
                workflow,
                endpoint,
            },
            dispatches,
        ))
    }

    pub(super) fn workflow_start_next_queued_prompt_for_response(
        &self,
        session_id: &str,
    ) -> Result<
        Option<(
            crate::app::workflow_runtime::WorkflowLaunchOutcome,
            WorkflowPromptDispatches,
        )>,
        DaemonError,
    > {
        self.workflow_reconcile_live_orphans(session_id);
        loop {
            self.workflow_ensure_dispatchable_runtime_instance(session_id)?;
            let Some((queued_prompt, workflow_run, workflow, endpoint)) = self
                .session_store
                .write()
                .dequeue_next_workflow_prompt_and_create_run(session_id)?
            else {
                return Ok(None);
            };
            match self.workflow_schedule_queued_prompt_run(
                session_id,
                queued_prompt.clone(),
                workflow_run,
                workflow,
                endpoint,
            ) {
                Ok(outcome) => return Ok(Some(outcome)),
                Err(error) => {
                    self.record_queued_workflow_prompt_launch_failure(
                        session_id,
                        &queued_prompt,
                        &error,
                    );
                }
            }
        }
    }

    pub(super) fn workflow_maybe_start_next_queued_prompt(
        &self,
        session_id: &str,
    ) -> WorkflowPromptDispatches {
        self.workflow_reconcile_live_orphans(session_id);
        let mut accumulated = WorkflowPromptDispatches::default();
        loop {
            if let Err(error) = self.workflow_ensure_dispatchable_runtime_instance(session_id) {
                self.record_notice(
                    session_id,
                    None,
                    self.attachment_store
                        .list_session_attachment_ids(session_id),
                    format!("Failed to provision workflow instance: {error}"),
                );
                return accumulated;
            }
            let next_workflow = {
                self.session_store
                    .write()
                    .dequeue_next_workflow_prompt_and_create_run(session_id)
            };
            let (queued_prompt, workflow_run, workflow, endpoint) = match next_workflow {
                Ok(Some(claimed)) => claimed,
                Ok(None) => {
                    let queued_metaagent_is_busy = self
                        .session_store
                        .get_session(session_id)
                        .ok()
                        .and_then(|session| {
                            let metaagent_id = session
                                .queued_metaagent_tasks()
                                .front()
                                .map(|task| task.metaagent_id().to_string())?;
                            let (active_prompt, queued_prompts) =
                                self.prompt_state_owner.state_parts(&session, &metaagent_id);
                            Some(active_prompt.is_some() || !queued_prompts.is_empty())
                        })
                        .unwrap_or(false);
                    if queued_metaagent_is_busy {
                        return accumulated;
                    }
                    let queued_metaagent_task = self
                        .session_store
                        .write()
                        .pop_next_queued_metaagent_task(session_id);
                    return match queued_metaagent_task {
                        Ok(Some(task)) => {
                            accumulated.starting_metaagent_tasks.push(task);
                            accumulated
                        }
                        Ok(None) => accumulated,
                        Err(error) => {
                            self.record_notice(
                                session_id,
                                None,
                                self.attachment_store
                                    .list_session_attachment_ids(session_id),
                                format!("Failed to start queued Meta task: {error}"),
                            );
                            accumulated
                        }
                    };
                }
                Err(error) => {
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store
                            .list_session_attachment_ids(session_id),
                        format!("Failed to start queued workflow prompt: {error}"),
                    );
                    return accumulated;
                }
            };
            match self.workflow_schedule_queued_prompt_run(
                session_id,
                queued_prompt.clone(),
                workflow_run,
                workflow,
                endpoint,
            ) {
                Ok((
                    crate::app::workflow_runtime::WorkflowLaunchOutcome::Started {
                        workflow_run,
                        workflow,
                        endpoint,
                    },
                    dispatches,
                )) => {
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store
                            .list_session_attachment_ids(session_id),
                        format!(
                            "Started queued workflow run `{}` for workflow `{}` endpoint `{}`.",
                            workflow_run.id(),
                            workflow.id(),
                            endpoint.id()
                        ),
                    );
                    accumulated.extend(dispatches);
                }
                Ok((crate::app::workflow_runtime::WorkflowLaunchOutcome::Enqueued { .. }, _)) => {}
                Err(error) => {
                    self.record_queued_workflow_prompt_launch_failure(
                        session_id,
                        &queued_prompt,
                        &error,
                    );
                }
            }
        }
    }

    fn record_queued_workflow_prompt_launch_failure(
        &self,
        session_id: &str,
        queued_prompt: &crate::session::WorkflowQueuedPrompt,
        error: &DaemonError,
    ) {
        crate::logging::warn_with_fields(
            "daemon.runtime",
            "queued workflow prompt launch failed",
            serde_json::json!({
                "session_id": session_id,
                "queued_prompt_id": queued_prompt.id(),
                "workflow_id": queued_prompt.workflow_id(),
                "endpoint_id": queued_prompt.endpoint_id(),
                "error": error.to_string(),
            }),
        );
        if let Some(watchdog_id) = queued_prompt.watchdog_id() {
            self.mark_workflow_watchdog_launch_failed(session_id, watchdog_id, error);
        }
        self.record_notice(
            session_id,
            None,
            self.attachment_store
                .list_session_attachment_ids(session_id),
            format!(
                "Queued workflow prompt `{}` failed: {error}",
                queued_prompt.id()
            ),
        );
    }

    fn mark_workflow_watchdog_launch_failed(
        &self,
        session_id: &str,
        watchdog_id: &str,
        error: &DaemonError,
    ) {
        let mut sessions = self.session_store.write();
        if workflow_watchdog_failure_is_terminal(error) {
            let _ = sessions.mark_workflow_watchdog_failed_and_disable(
                session_id,
                watchdog_id,
                error.to_string(),
            );
        } else {
            let _ =
                sessions.mark_workflow_watchdog_failed(session_id, watchdog_id, error.to_string());
        }
    }

    fn persist_workflow_watchdog_sessions(&self, session_ids: BTreeSet<String>) {
        for session_id in session_ids {
            if let Err(error) =
                self.persist_workflow_runtime_session(&session_id, "workflow_schedule_tick")
            {
                crate::logging::warn_with_fields(
                    "daemon.runtime",
                    "workflow schedule session persistence failed",
                    serde_json::json!({
                        "session_id": session_id,
                        "error": error.to_string(),
                    }),
                );
            }
        }
    }
}

fn workflow_watchdog_failure_is_terminal(error: &DaemonError) -> bool {
    matches!(
        error,
        DaemonError::WorkflowNotFound { .. }
            | DaemonError::WorkflowEndpointNotFound { .. }
            | DaemonError::WorkflowNodeNotFound { .. }
            | DaemonError::WorkflowNodeAgentMissing { .. }
            | DaemonError::InvalidWorkflowGraphReference { .. }
            | DaemonError::AgentNotFound { .. }
    )
}

#[cfg(test)]
mod tests;
