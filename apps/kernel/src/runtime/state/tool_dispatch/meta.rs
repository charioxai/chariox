use super::*;

use crate::transport::runtime_tools::{
    MetaAckEventArgs, MetaCommandListArgs, MetaCommandSearchArgs, MetaCompleteTaskArgs,
    MetaListEventsArgs, MetaMarkBlockedArgs, MetaReadEventArgs, MetaReadPlanArgs, MetaReadTaskArgs,
    MetaResolveRuntimeInteractionArgs, MetaSessionOverviewArgs, MetaSubscribeEventsArgs,
    MetaTurnBlobArgs, MetaTurnOverviewArgs, MetaUnsubscribeEventsArgs, MetaUpdatePlanArgs,
    MetaUpdateTaskArgs, RuntimeToolResult, META_ACK_EVENT_TOOL, META_COMMAND_DOCS_TOOL,
    META_COMPLETE_TASK_TOOL, META_LIST_COMMANDS_TOOL, META_LIST_EVENTS_TOOL,
    META_LIST_SUBSCRIPTIONS_TOOL, META_MARK_BLOCKED_TOOL, META_READ_EVENT_TOOL,
    META_READ_PLAN_TOOL, META_READ_TASK_TOOL, META_RESOLVE_RUNTIME_INTERACTION_TOOL,
    META_RUN_COMMAND_TOOL, META_SEARCH_COMMANDS_TOOL, META_SESSION_OVERVIEW_TOOL,
    META_SUBSCRIBE_EVENTS_TOOL, META_TURN_BLOB_TOOL, META_TURN_OVERVIEW_TOOL,
    META_UNSUBSCRIBE_EVENTS_TOOL, META_UPDATE_PLAN_TOOL, META_UPDATE_TASK_TOOL,
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
        let metaagent_provider_runs = metaagent_provider_runs_for_auth_token(self, &provider_runs);
        let [provider_run] = metaagent_provider_runs.as_slice() else {
            return Err(DaemonError::LocalTransport {
                operation: "runtime_tool_meta",
                message:
                    "metaagent runtime tools require exactly one active metaagent provider run"
                        .to_string(),
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
        matches!(
            metaagent_provider_runs_for_auth_token(self, &provider_runs).as_slice(),
            [_]
        )
    }

    pub(super) async fn dispatch_meta_runtime_tool_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let (session, agent) = self.metaagent_for_provider_run(provider_run)?;
        self.dispatch_meta_runtime_tool_call_for_session_agent(
            &session, &agent, tool_name, arguments,
        )
        .await
    }

    pub(crate) async fn dispatch_meta_runtime_tool_call_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session.id() || !agent.is_metaagent() {
            return Err(DaemonError::LocalTransport {
                operation: "runtime_tool_meta",
                message: "metaagent runtime tools are only available to session metaagents"
                    .to_string(),
            });
        }
        self.dispatch_meta_runtime_tool_call_for_session_agent(
            &session, &agent, tool_name, arguments,
        )
        .await
    }

    async fn dispatch_meta_runtime_tool_call_for_session_agent(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        match tool_name {
            META_SESSION_OVERVIEW_TOOL => {
                let args = serde_json::from_value::<MetaSessionOverviewArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_session_overview(session, agent, args)
            }
            META_SEARCH_COMMANDS_TOOL => {
                let args = serde_json::from_value::<MetaCommandSearchArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "commands": crate::runtime::metaagent_command_registry::search_commands(args),
                    }),
                })
            }
            META_LIST_COMMANDS_TOOL => {
                let args = serde_json::from_value::<MetaCommandListArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "commands": crate::runtime::metaagent_command_registry::list_commands(args),
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
                self.persist_metaagent_event_record("metaagent.event.read", &event);
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
                let acked =
                    self.owned
                        .metaagent_events
                        .ack(agent.id(), &event_ids, args.up_to_sequence);
                for event in &acked {
                    self.persist_metaagent_event_record("metaagent.event.acked", event);
                }
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "acked": acked,
                    }),
                })
            }
            META_TURN_OVERVIEW_TOOL => {
                let args = serde_json::from_value::<MetaTurnOverviewArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_turn_overview(session, agent, args).await
            }
            META_TURN_BLOB_TOOL => {
                let args = serde_json::from_value::<MetaTurnBlobArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_turn_blob(session, agent, args).await
            }
            META_SUBSCRIBE_EVENTS_TOOL => {
                let args = serde_json::from_value::<MetaSubscribeEventsArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                let subscription =
                    self.owned
                        .metaagent_events
                        .subscribe(agent.id(), args.kind, args.filter);
                self.persist_metaagent_subscription(
                    "metaagent.subscription.created",
                    &subscription,
                );
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "subscription": subscription,
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
                let removed = self
                    .owned
                    .metaagent_events
                    .unsubscribe(agent.id(), &args.subscription_id);
                if let Some(subscription) = removed.as_ref() {
                    self.persist_metaagent_subscription(
                        "metaagent.subscription.deleted",
                        subscription,
                    );
                }
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "subscription_id": args.subscription_id,
                        "status": if removed.is_some() { "removed" } else { "not_found" },
                    }),
                })
            }
            META_LIST_SUBSCRIPTIONS_TOOL => {
                let mut subscriptions = required_metaagent_subscriptions(agent.id());
                subscriptions.extend(
                    self.owned
                        .metaagent_events
                        .list_subscriptions(agent.id())
                        .into_iter()
                        .map(|subscription| serde_json::json!(subscription)),
                );
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "subscriptions": subscriptions,
                    }),
                })
            }
            META_READ_TASK_TOOL => {
                let _args = serde_json::from_value::<MetaReadTaskArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: metaagent_task_payload(session, agent),
                })
            }
            META_UPDATE_TASK_TOOL => {
                let args = serde_json::from_value::<MetaUpdateTaskArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                let updated = self
                    .owned
                    .session_store
                    .write()
                    .update_metaagent_task_markdown(session.id(), agent.id(), args.markdown)?;
                self.owned.session_projection.update(updated.clone());
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: metaagent_task_payload(&updated, agent),
                })
            }
            META_READ_PLAN_TOOL => {
                let _args = serde_json::from_value::<MetaReadPlanArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: metaagent_plan_payload(session, agent),
                })
            }
            META_UPDATE_PLAN_TOOL => {
                let args = serde_json::from_value::<MetaUpdatePlanArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                let updated = self
                    .owned
                    .session_store
                    .write()
                    .update_metaagent_plan_markdown(session.id(), agent.id(), args.markdown)?;
                self.owned.session_projection.update(updated.clone());
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: metaagent_plan_payload(&updated, agent),
                })
            }
            META_COMPLETE_TASK_TOOL => {
                let args = serde_json::from_value::<MetaCompleteTaskArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                let updated = self.owned.session_store.write().complete_metaagent_task(
                    session.id(),
                    agent.id(),
                    args.summary,
                )?;
                self.owned.session_projection.update(updated.clone());
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: metaagent_task_payload(&updated, agent),
                })
            }
            META_MARK_BLOCKED_TOOL => {
                let args = serde_json::from_value::<MetaMarkBlockedArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                let updated = self.owned.session_store.write().block_metaagent_task(
                    session.id(),
                    agent.id(),
                    args.reason,
                )?;
                self.owned.session_projection.update(updated.clone());
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: metaagent_task_payload(&updated, agent),
                })
            }
            META_RESOLVE_RUNTIME_INTERACTION_TOOL => {
                let args = serde_json::from_value::<MetaResolveRuntimeInteractionArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_resolve_runtime_interaction(session, agent, args)
                    .await
            }
            _ => Err(DaemonError::LocalTransport {
                operation: "runtime_tool_meta",
                message: format!("unsupported metaagent tool `{tool_name}`"),
            }),
        }
    }

    fn persist_metaagent_event_record(
        &self,
        kind: &'static str,
        record: &crate::runtime::metaagent_event::MetaagentEventRecord,
    ) {
        if let Err(error) = self.owned.durable_state_store.append_event(
            kind,
            Some(record.event_id.clone()),
            serde_json::json!({
                "record": record,
            }),
        ) {
            crate::logging::warn_with_fields(
                "metaagent.event",
                "failed to persist metaagent event mutation",
                serde_json::json!({
                    "kind": kind,
                    "event_id": &record.event_id,
                    "metaagent_id": &record.metaagent_id,
                    "error": error.to_string(),
                }),
            );
        }
    }

    fn persist_metaagent_subscription(
        &self,
        kind: &'static str,
        subscription: &crate::runtime::metaagent_event::MetaagentEventSubscription,
    ) {
        if let Err(error) = self.owned.durable_state_store.append_event(
            kind,
            Some(subscription.subscription_id.clone()),
            serde_json::json!({
                "subscription": subscription,
            }),
        ) {
            crate::logging::warn_with_fields(
                "metaagent.event",
                "failed to persist metaagent event subscription mutation",
                serde_json::json!({
                    "kind": kind,
                    "subscription_id": &subscription.subscription_id,
                    "metaagent_id": &subscription.metaagent_id,
                    "error": error.to_string(),
                }),
            );
        }
    }

    fn persist_metaagent_interaction_resolution(
        &self,
        session: &crate::session::RuntimeSession,
        metaagent: &crate::agent::AgentInstance,
        target: &crate::agent::AgentInstance,
        interaction: &crate::session::RuntimeInteraction,
        choice_id: &str,
        custom_reply: Option<&str>,
        provider_run_id: Option<&str>,
    ) {
        let correlation_id = format!(
            "metaagent:{}:runtime-interaction:{}",
            metaagent.id(),
            interaction.id()
        );
        if let Err(error) = self.owned.durable_state_store.append_event(
            "metaagent.interaction.resolved",
            Some(interaction.id().to_string()),
            serde_json::json!({
                "session_id": session.id(),
                "user_id": metaagent.owner_user_id(),
                "metaagent_id": metaagent.id(),
                "target_agent_id": target.id(),
                "interaction_id": interaction.id(),
                "interaction_kind": format!("{:?}", interaction.kind()),
                "choice_id": choice_id,
                "input": custom_reply.map(|reply| serde_json::json!({
                    "kind": "custom",
                    "char_count": reply.chars().count(),
                })),
                "provider_run_id": provider_run_id,
                "causation_id": interaction.id(),
                "correlation_id": correlation_id,
                "timestamp_ms": crate::session::unix_epoch_ms(),
            }),
        ) {
            crate::logging::warn_with_fields(
                "metaagent.audit",
                "failed to persist metaagent interaction resolution audit",
                serde_json::json!({
                    "session_id": session.id(),
                    "metaagent_id": metaagent.id(),
                    "target_agent_id": target.id(),
                    "interaction_id": interaction.id(),
                    "choice_id": choice_id,
                    "error": error.to_string(),
                }),
            );
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
}

fn metaagent_provider_runs_for_auth_token(
    runtime_state: &KernelRuntimeState,
    provider_runs: &[crate::provider::RuntimeProviderRun],
) -> Vec<crate::provider::RuntimeProviderRun> {
    provider_runs
        .iter()
        .filter(|provider_run| {
            runtime_state
                .metaagent_for_provider_run(provider_run)
                .is_ok()
        })
        .cloned()
        .collect()
}

fn invalid_meta_args(error: serde_json::Error) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "runtime_tool_meta",
        message: format!("invalid tool arguments: {error}"),
    }
}

fn metaagent_task_payload(
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

fn metaagent_plan_payload(
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

fn required_metaagent_subscriptions(metaagent_id: &str) -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "subscription_id": format!("required:{metaagent_id}:agent.turn.completed"),
            "kind": "agent.turn.completed",
            "required": true,
            "scope": "owned_regular_agents",
        }),
        serde_json::json!({
            "subscription_id": format!("required:{metaagent_id}:agent.turn.failed"),
            "kind": "agent.turn.failed",
            "required": true,
            "scope": "owned_regular_agents",
        }),
        serde_json::json!({
            "subscription_id": format!("required:{metaagent_id}:runtime.interaction"),
            "kind": "runtime.interaction",
            "required": true,
            "scope": "owned_regular_agents",
        }),
    ]
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
