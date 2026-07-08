use super::*;

impl KernelRuntimeState {
    pub(super) fn meta_session_overview(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaSessionOverviewArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let include_workflows = args.include_workflows.unwrap_or(true);
        let include_events = args.include_events.unwrap_or(true);
        let session = self.meta_coherent_session_snapshot(session, agent)?;
        let session_agents = self.owned.agent_store.get_session_agents(session.id());
        let owned_agents = session_agents
            .iter()
            .filter(|candidate| {
                !candidate.is_metaagent()
                    && candidate.controlled_by_metaagent_id() == Some(agent.id())
            })
            .collect::<Vec<_>>();
        let pending_interactions = session
            .active_interactions()
            .iter()
            .filter(|interaction| {
                interaction.agent_id() != agent.id()
                    && self
                        .owned
                        .agent_store
                        .get_agent(interaction.agent_id())
                        .is_ok_and(|target| {
                            !target.is_metaagent()
                                && target.controlled_by_metaagent_id() == Some(agent.id())
                        })
            })
            .collect::<Vec<_>>();
        let owned_workflow_ids = session
            .workflows()
            .iter()
            .filter(|workflow| workflow.controlled_by_metaagent_id() == Some(agent.id()))
            .map(|workflow| workflow.id().to_string())
            .collect::<std::collections::BTreeSet<_>>();
        let owned_workflows = session
            .workflows()
            .iter()
            .filter(|workflow| workflow.controlled_by_metaagent_id() == Some(agent.id()))
            .cloned()
            .collect::<Vec<_>>();
        let owned_workflow_runs = session
            .workflow_runs()
            .iter()
            .filter(|run| owned_workflow_ids.contains(run.workflow_id()))
            .cloned()
            .collect::<Vec<_>>();
        let agent_activity = self.agent_activity_for_session(&session);
        let owned_agent_refs = owned_agents
            .iter()
            .map(|agent| meta_owned_agent_ref_json(agent))
            .collect::<Vec<_>>();
        let completion_recommendation = meta_completion_recommendation(
            &session,
            agent,
            &owned_agents,
            &owned_workflow_runs,
            pending_interactions.len(),
            &agent_activity,
        );
        Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "session": {
                    "id": session.id(),
                    "workspace_id": session.workspace_id(),
                    "worktree_id": session.worktree_id(),
                    "status": session.status(),
                    "owner_user_id": session.owner_user_id(),
                    "focused_agent_id": session.focused_agent_id(),
                },
                "metaagent": {
                    "id": agent.id(),
                    "agent_ref": agent.agent_ref(),
                    "alias": agent.alias(),
                    "provider": agent.provider(),
                    "model": agent.model(),
                    "owner_user_id": agent.owner_user_id(),
                    "status": agent.state(),
                    "is_processing": agent.is_processing(),
                },
                "task": metaagent_task_payload(&session, agent),
                "agents": {
                    "total": session_agents.len(),
                    "owned_total": owned_agents.len(),
                    "owned": owned_agent_refs,
                },
                "workflows": if include_workflows {
                    serde_json::json!({
                        "definitions": owned_workflows,
                        "runs": owned_workflow_runs,
                    })
                } else {
                    serde_json::Value::Null
                },
                "pending_interactions": pending_interactions,
                "events": if include_events {
                    self.owned.metaagent_events.counts(agent.id())
                } else {
                    serde_json::Value::Null
                },
                "completion_recommendation": completion_recommendation,
            }),
        })
    }

    pub(super) fn meta_coherent_session_snapshot(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        let projected = self.owned.session_snapshot(session.id())?;
        if self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&projected, agent.id())
            .is_some()
            && session.metaagent_task(agent.id()).is_some()
            && projected.metaagent_task(agent.id()).is_none()
        {
            return Err(DaemonError::LocalTransport {
                operation: "runtime_tool_meta.session_snapshot",
                message: "metaagent task snapshot is temporarily unavailable while the kernel projection catches up; retry shortly".to_string(),
            });
        }
        Ok(projected)
    }

    pub(super) async fn meta_turn_overview(
        &self,
        session: &crate::session::RuntimeSession,
        metaagent: &crate::agent::AgentInstance,
        args: MetaTurnOverviewArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let target = match args.agent_ref.as_deref() {
            Some(reference) => {
                match self.meta_owned_regular_agent(session.id(), metaagent, reference) {
                    Ok(agent) => agent,
                    Err(error) => {
                        return Ok(RuntimeToolResult {
                            ok: false,
                            payload: serde_json::json!({ "error": error.to_string() }),
                        });
                    }
                }
            }
            None => {
                let agents = self.meta_owned_regular_agents(session.id(), metaagent);
                let Some(agent) = agents.into_iter().next() else {
                    return Ok(RuntimeToolResult {
                        ok: false,
                        payload: serde_json::json!({
                            "error": "turn_overview requires agent_ref when no owned regular agents exist"
                        }),
                    });
                };
                agent
            }
        };
        let latest_prompt_count = args.turns_back.unwrap_or(0).saturating_add(1).clamp(1, 20);
        let request = crate::local::GetSessionHistoryOutlineRequest {
            session_id: session.id().to_string(),
            agent_ids: Some(vec![target.id().to_string()]),
            latest_prompt_count: Some(latest_prompt_count),
            cursor: None,
        };
        let response = crate::runtime::history_requests::execute_session_history_outline_request(
            self.owned.operational_history_store.clone(),
            request,
        )
        .await?;
        let crate::local::LocalDaemonResponse::SessionHistoryOutline { mut agents } = response
        else {
            return Err(DaemonError::LocalTransport {
                operation: "runtime_tool_meta.turn_overview",
                message: "history outline returned an unexpected response".to_string(),
            });
        };
        let mut turns = agents.pop().map(|agent| agent.turns).unwrap_or_default();
        if let Some(turn_ref) = args.turn_ref.as_deref() {
            turns.retain(|turn| {
                turn.turn_id == turn_ref || turn.prompt_id.as_deref() == Some(turn_ref)
            });
        } else {
            let turns_back = args.turns_back.unwrap_or(0);
            if turns_back > 0 {
                turns = turns.into_iter().skip(turns_back).take(1).collect();
            } else {
                turns.truncate(1);
            }
        }
        let limit = args.limit.unwrap_or(200).clamp(1, 200);
        Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "agent": meta_agent_ref_json(&target),
                "turns": turns.into_iter().map(|turn| {
                    let mut items = Vec::new();
                    items.push(serde_json::json!({
                        "kind": "user_prompt",
                        "title": "prompt",
                        "entry": turn.user_prompt,
                    }));
                    for entry in turn.entries.into_iter().take(limit) {
                        items.push(serde_json::json!({
                            "kind": "assistant",
                            "title": "assistant",
                            "entry": entry,
                        }));
                    }
                    for blob in turn.blobs.into_iter().take(limit) {
                        items.push(serde_json::json!({
                            "kind": format!("{:?}", blob.kind),
                            "title": blob.title,
                            "summary": blob.summary,
                            "blob_id": blob.blob_id,
                            "sequence_start": blob.sequence_start,
                            "sequence_end": blob.sequence_end,
                            "entry_count": blob.entry_count,
                            "total_chars": blob.total_chars,
                            "timestamp_ms": blob.timestamp_ms,
                        }));
                    }
                    if let Some(summary) = turn.summary {
                        items.push(serde_json::json!({
                            "kind": "assistant_summary",
                            "title": "summary",
                            "entry": summary,
                        }));
                    }
                    serde_json::json!({
                        "turn_id": turn.turn_id,
                        "prompt_id": turn.prompt_id,
                        "started_at_ms": turn.started_at_ms,
                        "items": items.into_iter().take(limit).collect::<Vec<_>>(),
                    })
                }).collect::<Vec<_>>(),
            }),
        })
    }

    pub(super) async fn meta_turn_blob(
        &self,
        session: &crate::session::RuntimeSession,
        metaagent: &crate::agent::AgentInstance,
        args: MetaTurnBlobArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        for target in self.meta_owned_regular_agents(session.id(), metaagent) {
            let request = crate::local::GetSessionHistoryBlobContentRequest {
                session_id: session.id().to_string(),
                agent_id: target.id().to_string(),
                blob_id: args.blob_id.clone(),
            };
            let response =
                crate::runtime::history_requests::execute_session_history_blob_content_request(
                    self.owned.operational_history_store.clone(),
                    request,
                )
                .await?;
            let crate::local::LocalDaemonResponse::SessionHistoryBlobContent { blob_id, entries } =
                response
            else {
                return Err(DaemonError::LocalTransport {
                    operation: "runtime_tool_meta.turn_blob",
                    message: "history blob returned an unexpected response".to_string(),
                });
            };
            if !entries.is_empty() {
                return Ok(RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "agent": meta_agent_ref_json(&target),
                        "blob_id": blob_id,
                        "entries": entries,
                    }),
                });
            }
        }
        Ok(RuntimeToolResult {
            ok: false,
            payload: serde_json::json!({
                "error": format!("blob `{}` was not found for an owned regular agent", args.blob_id),
            }),
        })
    }

    pub(super) async fn meta_resolve_runtime_interaction(
        &self,
        session: &crate::session::RuntimeSession,
        metaagent: &crate::agent::AgentInstance,
        args: MetaResolveRuntimeInteractionArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let Some(interaction) = session
            .active_interactions()
            .iter()
            .find(|interaction| interaction.id() == args.interaction_id)
            .cloned()
        else {
            return Ok(RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": format!("runtime interaction `{}` is not active", args.interaction_id),
                }),
            });
        };
        if interaction.agent_id() == metaagent.id() {
            return Ok(RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": "agents in Meta mode cannot resolve their own runtime interactions",
                    "interaction_id": interaction.id(),
                }),
            });
        }
        let target = match self.owned.agent_store.get_agent(interaction.agent_id()) {
            Ok(agent) => agent,
            Err(error) => {
                return Ok(RuntimeToolResult {
                    ok: false,
                    payload: serde_json::json!({ "error": error.to_string() }),
                });
            }
        };
        if target.is_metaagent() || target.controlled_by_metaagent_id() != Some(metaagent.id()) {
            return Ok(RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": "agents in Meta mode may only resolve interactions for owned regular agents",
                    "interaction_id": interaction.id(),
                    "target_agent_id": target.id(),
                }),
            });
        }
        let choice_id = args.choice_id.or_else(|| {
            if let Some(input) = args.input.as_deref() {
                interaction
                    .custom_choice()
                    .filter(|choice| choice.id() == input)
                    .map(|choice| choice.id().to_string())
            } else {
                None
            }
        });
        let Some(choice_id) = choice_id else {
            return Ok(RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": "resolve_runtime_interaction requires choice_id",
                    "interaction_id": interaction.id(),
                }),
            });
        };
        let custom_reply = interaction
            .custom_choice()
            .filter(|choice| choice.id() == choice_id)
            .and_then(|_| args.input.as_deref());
        let provider_run_id = self
            .owned
            .provider_store
            .get_run_for_agent(session.id(), target.id())
            .map(|run| run.id().to_string());
        match self
            .resolve_runtime_interaction(session.id(), interaction.id(), &choice_id, custom_reply)
            .await
        {
            Ok(()) => {
                self.persist_metaagent_interaction_resolution(
                    session,
                    metaagent,
                    &target,
                    &interaction,
                    &choice_id,
                    custom_reply,
                    provider_run_id.as_deref(),
                );
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "interaction_id": interaction.id(),
                        "choice_id": choice_id,
                        "target_agent": meta_agent_ref_json(&target),
                        "resolved_by": {
                            "kind": "metaagent",
                            "metaagent_id": metaagent.id(),
                            "owner_user_id": metaagent.owner_user_id(),
                        },
                    }),
                })
            }
            Err(error) => Ok(RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": error.to_string(),
                    "interaction_id": interaction.id(),
                }),
            }),
        }
    }

    pub(super) fn meta_owned_regular_agents(
        &self,
        session_id: &str,
        metaagent: &crate::agent::AgentInstance,
    ) -> Vec<crate::agent::AgentInstance> {
        self.owned
            .agent_store
            .get_session_agents(session_id)
            .into_iter()
            .filter(|agent| {
                !agent.is_metaagent() && agent.controlled_by_metaagent_id() == Some(metaagent.id())
            })
            .collect()
    }

    pub(super) fn meta_owned_regular_agent(
        &self,
        session_id: &str,
        metaagent: &crate::agent::AgentInstance,
        reference: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.meta_owned_regular_agents(session_id, metaagent)
            .into_iter()
            .find(|agent| {
                agent.id() == reference
                    || agent.agent_ref() == reference
                    || agent.alias() == Some(reference)
            })
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "runtime_tool_meta",
                message: owned_regular_agent_error_message(
                    reference,
                    &self.meta_owned_regular_agents(session_id, metaagent),
                ),
            })
    }
}

pub(super) fn metaagent_task_payload(
    session: &crate::session::RuntimeSession,
    agent: &crate::agent::AgentInstance,
) -> serde_json::Value {
    match session.metaagent_task(agent.id()) {
        Some(task) => serde_json::json!({
            "status": task.status(),
            "metaagent_id": agent.id(),
            "task": task,
        }),
        None => serde_json::json!({
            "status": "none",
            "metaagent_id": agent.id(),
            "task": null,
        }),
    }
}

pub(super) fn metaagent_plan_payload(
    session: &crate::session::RuntimeSession,
    agent: &crate::agent::AgentInstance,
) -> serde_json::Value {
    match session.metaagent_task(agent.id()) {
        Some(task) => serde_json::json!({
            "status": task.status(),
            "metaagent_id": agent.id(),
            "plan_markdown": task.plan_markdown(),
            "task": task,
        }),
        None => serde_json::json!({
            "status": "none",
            "metaagent_id": agent.id(),
            "plan_markdown": "",
            "task": null,
        }),
    }
}

pub(super) fn meta_agent_ref_json(agent: &crate::agent::AgentInstance) -> serde_json::Value {
    serde_json::json!({
        "id": agent.id(),
        "agent_ref": agent.agent_ref(),
        "prompt_ref": meta_agent_prompt_ref(agent),
        "alias": agent.alias(),
        "provider": agent.provider(),
        "model": agent.model(),
        "owner_user_id": agent.owner_user_id(),
        "role": agent.role(),
        "state": agent.state(),
        "example_prompt_command": format!(
            "prompt {} \"<objective, context, constraints, expected report>\"",
            shell_quote_for_meta_command(&meta_agent_prompt_ref(agent)),
        ),
    })
}

pub(super) fn meta_owned_agent_ref_json(agent: &crate::agent::AgentInstance) -> serde_json::Value {
    meta_agent_ref_json(agent)
}

fn meta_agent_prompt_ref(agent: &crate::agent::AgentInstance) -> String {
    agent
        .alias()
        .filter(|alias| !alias.trim().is_empty())
        .unwrap_or_else(|| agent.agent_ref())
        .to_string()
}

fn owned_regular_agent_error_message(
    reference: &str,
    owned_agents: &[crate::agent::AgentInstance],
) -> String {
    if owned_agents.is_empty() {
        return format!(
            "agent `{reference}` is not an owned regular agent in this session. No owned regular agents are available; spawn one first with `agent spawn <alias>`."
        );
    }
    let available = owned_agents
        .iter()
        .map(|agent| {
            format!(
                "{} (agent_ref: {}, id: {})",
                meta_agent_prompt_ref(agent),
                agent.agent_ref(),
                agent.id()
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let example_ref = meta_agent_prompt_ref(&owned_agents[0]);
    format!(
        "agent `{reference}` is not an owned regular agent in this session. Available owned agents: {available}. Use a listed alias or agent_ref, for example `prompt {} \"<objective, context, constraints, expected report>\"`.",
        shell_quote_for_meta_command(&example_ref)
    )
}

fn shell_quote_for_meta_command(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/' | ':'))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn meta_completion_recommendation(
    session: &crate::session::RuntimeSession,
    metaagent: &crate::agent::AgentInstance,
    owned_agents: &[&crate::agent::AgentInstance],
    owned_workflow_runs: &[crate::session::WorkflowRun],
    pending_interaction_count: usize,
    agent_activity: &std::collections::BTreeMap<
        String,
        crate::runtime::projection::AgentRuntimeActivity,
    >,
) -> serde_json::Value {
    let active_owned_workers = owned_agents
        .iter()
        .filter(|agent| {
            agent_activity
                .get(agent.id())
                .is_some_and(|activity| activity.busy)
                || agent.is_processing()
        })
        .map(|agent| meta_owned_agent_ref_json(agent))
        .collect::<Vec<_>>();
    let active_workflow_runs = owned_workflow_runs
        .iter()
        .filter(|run| {
            matches!(
                run.status(),
                crate::session::WorkflowRunStatus::Created
                    | crate::session::WorkflowRunStatus::Running
                    | crate::session::WorkflowRunStatus::Waiting
                    | crate::session::WorkflowRunStatus::Completing
            )
        })
        .collect::<Vec<_>>();
    let task_status = session
        .metaagent_task(metaagent.id())
        .map(|task| task.status());
    let (kind, reason, suggested_next_action) = if !active_owned_workers.is_empty()
        || !active_workflow_runs.is_empty()
    {
        (
            "should_wait",
            "owned worker or workflow activity is still active",
            "call wait_trace with a clear condition, inspect workflow runs, or yield for kernel continuation",
        )
    } else if pending_interaction_count > 0 {
        (
            "should_wait",
            "owned workers have pending runtime interactions",
            "resolve or ask the user about the pending interaction before completing",
        )
    } else if task_status.is_none() {
        (
            "needs_worker",
            "no active Meta-mode task is visible in the session snapshot",
            "retry session_overview shortly or ask the user to start a task with /meta",
        )
    } else if owned_agents.is_empty() && owned_workflow_runs.is_empty() {
        (
            "needs_worker",
            "no owned workers or workflow runs exist yet",
            "spawn a regular worker or create and run a workflow before judging the task complete",
        )
    } else if matches!(
        task_status,
        Some(crate::session::MetaagentTaskStatus::Blocked)
    ) {
        (
            "blocked_candidate",
            "the task is already marked blocked",
            "wait for user steering or update the plan if new information removes the block",
        )
    } else {
        (
            "can_complete",
            "no owned worker, workflow, or interaction is active",
            "review worker/workflow evidence and call complete_task if it proves the goal; otherwise prompt a worker or mark_blocked",
        )
    };
    serde_json::json!({
        "kind": kind,
        "reason": reason,
        "active_owned_workers": active_owned_workers,
        "active_workflow_run_count": active_workflow_runs.len(),
        "pending_interaction_count": pending_interaction_count,
        "suggested_next_action": suggested_next_action,
        "non_authoritative": true,
    })
}
