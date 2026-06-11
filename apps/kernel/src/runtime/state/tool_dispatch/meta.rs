use super::*;

use crate::transport::runtime_tools::{
    MetaAckEventArgs, MetaListEventsArgs, MetaReadEventArgs, MetaResolveRuntimeInteractionArgs,
    MetaSessionOverviewArgs, MetaSubscribeEventsArgs, MetaTurnBlobArgs, MetaTurnOverviewArgs,
    MetaUnsubscribeEventsArgs, RuntimeToolResult, META_ACK_EVENT_TOOL, META_COMMAND_DOCS_TOOL,
    META_LIST_COMMANDS_TOOL, META_LIST_EVENTS_TOOL, META_LIST_SUBSCRIPTIONS_TOOL,
    META_READ_EVENT_TOOL, META_RESOLVE_RUNTIME_INTERACTION_TOOL, META_RUN_COMMAND_TOOL,
    META_SEARCH_COMMANDS_TOOL, META_SESSION_OVERVIEW_TOOL, META_SUBSCRIBE_EVENTS_TOOL,
    META_TURN_BLOB_TOOL, META_TURN_OVERVIEW_TOOL, META_UNSUBSCRIBE_EVENTS_TOOL,
};

impl KernelRuntimeState {
    pub(crate) fn metaagent_context_for_auth_token(
        &self,
        auth_token: &str,
    ) -> Result<
        (
            crate::provider::RuntimeProviderRun,
            crate::session::RuntimeSession,
            crate::agent::AgentInstance,
        ),
        DaemonError,
    > {
        let provider_runs = self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(auth_token);
        let [provider_run] = provider_runs.as_slice() else {
            return Err(DaemonError::LocalTransport {
                operation: "runtime_tool_meta",
                message: "metaagent runtime tools require one active provider run".to_string(),
            });
        };
        let (session, agent) = self.metaagent_for_provider_run(provider_run)?;
        Ok((provider_run.clone(), session, agent))
    }

    pub(super) fn meta_runtime_tool_specs_enabled_for_auth_token(&self, auth_token: &str) -> bool {
        let provider_runs = self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(auth_token);
        let [provider_run] = provider_runs.as_slice() else {
            return false;
        };
        self.metaagent_for_provider_run(provider_run).is_ok()
    }

    pub(super) async fn dispatch_meta_runtime_tool_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let (session, agent) = self.metaagent_for_provider_run(provider_run)?;
        match tool_name {
            META_SESSION_OVERVIEW_TOOL => {
                let args = serde_json::from_value::<MetaSessionOverviewArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_session_overview(&session, &agent, args)
            }
            META_SEARCH_COMMANDS_TOOL | META_LIST_COMMANDS_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::MetaCommandSearchArgs,
                >(arguments)
                .map_err(invalid_meta_args)?;
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "commands": crate::runtime::metaagent_command_registry::search_commands(args),
                    }),
                })
            }
            META_COMMAND_DOCS_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::MetaCommandDocsArgs,
                >(arguments)
                .map_err(invalid_meta_args)?;
                let command_for_error = args.command.clone();
                Ok(
                    match crate::runtime::metaagent_command_registry::command_docs(args) {
                        Some(payload) => RuntimeToolResult { ok: true, payload },
                        None => RuntimeToolResult {
                            ok: false,
                            payload: serde_json::json!({
                                "error": format!("unknown metaagent command `{command_for_error}`")
                            }),
                        },
                    },
                )
            }
            META_RUN_COMMAND_TOOL => Ok(RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": "arroba.meta.run_command must be dispatched through the runtime MCP router",
                    "tool": tool_name,
                    "agent_ref": agent.agent_ref(),
                }),
            }),
            META_LIST_EVENTS_TOOL => {
                let args = serde_json::from_value::<MetaListEventsArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "events": self.owned.metaagent_events.list(
                            agent.id(),
                            args.kind.as_deref(),
                            args.status.as_deref(),
                            args.limit.unwrap_or(50).clamp(1, 100),
                        ),
                    }),
                })
            }
            META_READ_EVENT_TOOL => {
                let args = serde_json::from_value::<MetaReadEventArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                let Some(event) = self.owned.metaagent_events.read(agent.id(), &args.event_id)
                else {
                    return Ok(RuntimeToolResult {
                        ok: false,
                        payload: serde_json::json!({
                            "error": format!("metaagent event `{}` not found", args.event_id),
                        }),
                    });
                };
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({ "event": event }),
                })
            }
            META_ACK_EVENT_TOOL => {
                let args = serde_json::from_value::<MetaAckEventArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                let mut event_ids = args.event_ids.unwrap_or_default();
                if let Some(event_id) = args.event_id {
                    event_ids.push(event_id);
                }
                if event_ids.is_empty() && args.up_to_sequence.is_none() {
                    return Ok(RuntimeToolResult {
                        ok: false,
                        payload: serde_json::json!({
                            "error": "ack_event requires event_id, event_ids, or up_to_sequence",
                        }),
                    });
                }
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "acked": self.owned.metaagent_events.ack(
                            agent.id(),
                            &event_ids,
                            args.up_to_sequence,
                        ),
                    }),
                })
            }
            META_TURN_OVERVIEW_TOOL => {
                let args = serde_json::from_value::<MetaTurnOverviewArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_turn_overview(&session, &agent, args).await
            }
            META_TURN_BLOB_TOOL => {
                let args = serde_json::from_value::<MetaTurnBlobArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_turn_blob(&session, &agent, args).await
            }
            META_SUBSCRIBE_EVENTS_TOOL => {
                let args = serde_json::from_value::<MetaSubscribeEventsArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "subscription": {
                            "subscription_id": format!("optional:{}:{}", agent.id(), args.kind),
                            "kind": args.kind,
                            "filter": args.filter,
                            "required": false,
                            "status": "registered",
                        }
                    }),
                })
            }
            META_UNSUBSCRIBE_EVENTS_TOOL => {
                let args = serde_json::from_value::<MetaUnsubscribeEventsArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                if args.subscription_id.starts_with("required:") {
                    return Ok(RuntimeToolResult {
                        ok: false,
                        payload: serde_json::json!({
                            "error": "required metaagent event subscriptions cannot be removed",
                            "subscription_id": args.subscription_id,
                        }),
                    });
                }
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "subscription_id": args.subscription_id,
                        "status": "removed",
                    }),
                })
            }
            META_LIST_SUBSCRIPTIONS_TOOL => Ok(RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({
                    "subscriptions": [
                        {
                            "subscription_id": format!("required:{}:agent.turn.completed", agent.id()),
                            "kind": "agent.turn.completed",
                            "required": true,
                            "scope": "owned_regular_agents",
                        },
                        {
                            "subscription_id": format!("required:{}:agent.turn.failed", agent.id()),
                            "kind": "agent.turn.failed",
                            "required": true,
                            "scope": "owned_regular_agents",
                        },
                        {
                            "subscription_id": format!("required:{}:runtime.interaction", agent.id()),
                            "kind": "runtime.interaction",
                            "required": true,
                            "scope": "owned_regular_agents",
                        },
                    ],
                }),
            }),
            META_RESOLVE_RUNTIME_INTERACTION_TOOL => {
                let args = serde_json::from_value::<MetaResolveRuntimeInteractionArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_resolve_runtime_interaction(&session, &agent, args)
                    .await
            }
            _ => Err(DaemonError::LocalTransport {
                operation: "runtime_tool_meta",
                message: format!("unsupported metaagent tool `{tool_name}`"),
            }),
        }
    }

    fn metaagent_for_provider_run(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
    ) -> Result<(crate::session::RuntimeSession, crate::agent::AgentInstance), DaemonError> {
        let Some(agent_id) = provider_run.agent_instance_id() else {
            return Err(DaemonError::LocalTransport {
                operation: "runtime_tool_meta",
                message: "metaagent tools require an agent-bound provider run".to_string(),
            });
        };
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if !agent.is_metaagent() {
            return Err(DaemonError::LocalTransport {
                operation: "runtime_tool_meta",
                message: "metaagent tools are only available to session metaagents".to_string(),
            });
        }
        let session = self
            .owned
            .session_store
            .get_session(provider_run.session_id())?;
        Ok((session, agent))
    }

    fn meta_session_overview(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaSessionOverviewArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let include_workflows = args.include_workflows.unwrap_or(true);
        let include_events = args.include_events.unwrap_or(true);
        let session_agents = self.owned.agent_store.get_session_agents(session.id());
        let owned_agents = session_agents
            .iter()
            .filter(|candidate| candidate.owner_user_id() == agent.owner_user_id())
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
                                && target.owner_user_id() == agent.owner_user_id()
                        })
            })
            .collect::<Vec<_>>();
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
                "agents": {
                    "total": session_agents.len(),
                    "owned_total": owned_agents.len(),
                    "owned": owned_agents,
                },
                "workflows": if include_workflows {
                    serde_json::json!({
                        "definitions": session.workflows(),
                        "runs": session.workflow_runs(),
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
            }),
        })
    }

    async fn meta_turn_overview(
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

    async fn meta_turn_blob(
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

    async fn meta_resolve_runtime_interaction(
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
                    "error": "metaagents cannot resolve their own runtime interactions",
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
        if target.is_metaagent() || target.owner_user_id() != metaagent.owner_user_id() {
            return Ok(RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": "metaagents may only resolve interactions for owned regular agents",
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
        match self
            .resolve_runtime_interaction(session.id(), interaction.id(), &choice_id, custom_reply)
            .await
        {
            Ok(()) => Ok(RuntimeToolResult {
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
            }),
            Err(error) => Ok(RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": error.to_string(),
                    "interaction_id": interaction.id(),
                }),
            }),
        }
    }

    fn meta_owned_regular_agents(
        &self,
        session_id: &str,
        metaagent: &crate::agent::AgentInstance,
    ) -> Vec<crate::agent::AgentInstance> {
        self.owned
            .agent_store
            .get_session_agents(session_id)
            .into_iter()
            .filter(|agent| {
                !agent.is_metaagent() && agent.owner_user_id() == metaagent.owner_user_id()
            })
            .collect()
    }

    fn meta_owned_regular_agent(
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
                message: format!(
                    "agent `{reference}` is not an owned regular agent in this session"
                ),
            })
    }

    pub(crate) fn metaagent_turn_completion_prompt(
        &self,
        session_id: &str,
        completed_agent_id: &str,
        completion: &crate::session::PromptCompletion,
        prompt_id: String,
    ) -> Option<crate::session::PromptQueueItem> {
        let completed_agent = self.owned.agent_store.get_agent(completed_agent_id).ok()?;
        if completed_agent.is_metaagent() {
            return None;
        }
        let metaagent = self
            .owned
            .agent_store
            .get_session_agents(session_id)
            .into_iter()
            .find(|agent| {
                agent.is_metaagent() && agent.owner_user_id() == completed_agent.owner_user_id()
            })?;
        let title = format!(
            "{} completed a turn",
            completed_agent
                .alias()
                .unwrap_or_else(|| completed_agent.agent_ref())
        );
        let prompt_preview = completion
            .completed
            .prompt()
            .chars()
            .take(240)
            .collect::<String>();
        let summary = format!(
            "Agent {} completed prompt {}. User prompt preview: {}",
            completed_agent.agent_ref(),
            completion.completed.id(),
            if prompt_preview.trim().is_empty() {
                "<empty>"
            } else {
                prompt_preview.trim()
            }
        );
        let record = self.owned.metaagent_events.record(
            crate::runtime::metaagent_event::NewMetaagentEvent {
                session_id: session_id.to_string(),
                metaagent_id: metaagent.id().to_string(),
                owner_user_id: metaagent.owner_user_id().to_string(),
                kind: "agent.turn.completed".to_string(),
                source_agent_id: Some(completed_agent.id().to_string()),
                title: title.clone(),
                summary: summary.clone(),
                detail: serde_json::json!({
                    "completed_prompt_id": completion.completed.id(),
                    "source_attachment_id": completion.completed.source_attachment_id(),
                    "completed_agent_id": completed_agent.id(),
                    "completed_agent_ref": completed_agent.agent_ref(),
                    "completed_agent_alias": completed_agent.alias(),
                    "started_next_prompt_id": completion.started_next.as_ref().map(|prompt| prompt.id()),
                }),
                injected_prompt_id: Some(prompt_id.clone()),
            },
        );
        let assembly = crate::scheduler::prompt_injection::render_metaagent_event_prompt_assembly(
            crate::scheduler::prompt_injection::MetaagentEventPromptContext {
                event_id: record.event_id,
                event_kind: record.kind,
                source: completed_agent.agent_ref().to_string(),
                title,
                body: summary,
            },
        );
        Some(crate::session::PromptQueueItem::new(
            prompt_id,
            completion.completed.source_attachment_id(),
            metaagent.id(),
            assembly.visible_user_prompt,
            crate::session::PromptStatus::Queued,
        ))
    }
}

fn invalid_meta_args(error: serde_json::Error) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "runtime_tool_meta",
        message: format!("invalid tool arguments: {error}"),
    }
}

fn meta_agent_ref_json(agent: &crate::agent::AgentInstance) -> serde_json::Value {
    serde_json::json!({
        "id": agent.id(),
        "agent_ref": agent.agent_ref(),
        "alias": agent.alias(),
        "provider": agent.provider(),
        "model": agent.model(),
        "owner_user_id": agent.owner_user_id(),
        "role": agent.role(),
        "state": agent.state(),
    })
}
