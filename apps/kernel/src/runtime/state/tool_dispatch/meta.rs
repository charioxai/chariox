use super::*;

mod session_tools;
mod trace;
mod workflow_code;

use self::session_tools::{metaagent_plan_payload, metaagent_task_payload};
use self::trace::suggest_metaagent_event_kinds;

use crate::transport::runtime_tools::{
    MetaAckEventArgs, MetaCommandListArgs, MetaCommandSearchArgs, MetaCompleteTaskArgs,
    MetaGuideListArgs, MetaGuideSearchArgs, MetaListEventsArgs, MetaMarkBlockedArgs,
    MetaPollTraceArgs, MetaReadEventArgs, MetaReadGuideArgs, MetaReadPlanArgs, MetaReadTaskArgs,
    MetaResolveRuntimeInteractionArgs, MetaSessionOverviewArgs, MetaSubscribeEventsArgs,
    MetaSubscribeTraceArgs, MetaTurnBlobArgs, MetaTurnOverviewArgs, MetaUnsubscribeEventsArgs,
    MetaUnsubscribeTraceArgs, MetaUpdatePlanArgs, MetaUpdateTaskArgs, MetaWorkflowCodeApplyArgs,
    MetaWorkflowCodeCanvasContractArgs, MetaWorkflowCodeCreateArgs, MetaWorkflowCodeDeleteArgs,
    MetaWorkflowCodeExportArgs, MetaWorkflowCodeImportArgs, MetaWorkflowCodeListArgs,
    MetaWorkflowCodePackageExportArgs, MetaWorkflowCodePackageImportArgs, MetaWorkflowCodeReadArgs,
    MetaWorkflowCodeRunArgs, MetaWorkflowCodeSourceExportArgs, MetaWorkflowCodeSourceExportDirArgs,
    MetaWorkflowCodeUpdateArgs, MetaWorkflowCodeValidateArgs, MetaWorkflowRegistryAddArgs,
    MetaWorkflowRegistryAddFromWorkflowArgs, MetaWorkflowRegistryDeleteArgs,
    MetaWorkflowRegistryGetArgs, MetaWorkflowRegistryListArgs, MetaWorkflowRegistryLoadArgs,
    MetaWorkflowRegistryRunArgs, RuntimeToolResult, META_ACK_EVENT_TOOL, META_COMMAND_DOCS_TOOL,
    META_COMPLETE_TASK_TOOL, META_EVENT_KINDS, META_LIST_COMMANDS_TOOL, META_LIST_EVENTS_TOOL,
    META_LIST_GUIDES_TOOL, META_LIST_SUBSCRIPTIONS_TOOL, META_MARK_BLOCKED_TOOL,
    META_POLL_TRACE_TOOL, META_READ_EVENT_TOOL, META_READ_GUIDE_TOOL, META_READ_PLAN_TOOL,
    META_READ_TASK_TOOL, META_RESOLVE_RUNTIME_INTERACTION_TOOL, META_RUN_COMMAND_TOOL,
    META_SEARCH_COMMANDS_TOOL, META_SEARCH_GUIDES_TOOL, META_SESSION_OVERVIEW_TOOL,
    META_SUBSCRIBE_EVENTS_TOOL, META_SUBSCRIBE_TRACE_TOOL, META_TURN_BLOB_TOOL,
    META_TURN_OVERVIEW_TOOL, META_UNSUBSCRIBE_EVENTS_TOOL, META_UNSUBSCRIBE_TRACE_TOOL,
    META_UPDATE_PLAN_TOOL, META_UPDATE_TASK_TOOL, META_WAIT_TRACE_TOOL,
    META_WORKFLOW_CODE_APPLY_TOOL, META_WORKFLOW_CODE_CANVAS_CONTRACT_TOOL,
    META_WORKFLOW_CODE_CREATE_TOOL, META_WORKFLOW_CODE_DELETE_TOOL, META_WORKFLOW_CODE_EXPORT_TOOL,
    META_WORKFLOW_CODE_IMPORT_TOOL, META_WORKFLOW_CODE_LIST_TOOL,
    META_WORKFLOW_CODE_PACKAGE_EXPORT_TOOL, META_WORKFLOW_CODE_PACKAGE_IMPORT_TOOL,
    META_WORKFLOW_CODE_READ_TOOL, META_WORKFLOW_CODE_RUN_TOOL,
    META_WORKFLOW_CODE_SOURCE_EXPORT_DIRECTORY_TOOL, META_WORKFLOW_CODE_SOURCE_EXPORT_TOOL,
    META_WORKFLOW_CODE_UPDATE_TOOL, META_WORKFLOW_CODE_VALIDATE_TOOL,
    META_WORKFLOW_REGISTRY_ADD_FROM_WORKFLOW_TOOL, META_WORKFLOW_REGISTRY_ADD_TOOL,
    META_WORKFLOW_REGISTRY_DELETE_TOOL, META_WORKFLOW_REGISTRY_GET_TOOL,
    META_WORKFLOW_REGISTRY_LIST_TOOL, META_WORKFLOW_REGISTRY_LOAD_TOOL,
    META_WORKFLOW_REGISTRY_RUN_TOOL,
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
                    "Meta mode runtime tools require exactly one active provider run for an agent in Meta mode"
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
                message:
                    "Meta mode runtime tools are only available to agents currently in Meta mode"
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
        let tool_name = crate::transport::runtime_tools::canonical_meta_tool_name(tool_name)
            .unwrap_or(tool_name);
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
            META_SEARCH_GUIDES_TOOL => {
                let args = serde_json::from_value::<MetaGuideSearchArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                let guide_context =
                    crate::runtime::metaagent_guides::MetaagentGuideContext::for_workspace(
                        agent.worktree_id().unwrap_or_else(|| session.worktree_id()),
                    );
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "guides": crate::runtime::metaagent_guides::search_guides_with_context(
                            crate::runtime::metaagent_guides::MetaagentGuideSearchArgs {
                                query: args.query,
                                tag: args.tag,
                                command: args.command,
                                limit: args.limit,
                            },
                            &guide_context,
                        ),
                    }),
                })
            }
            META_LIST_GUIDES_TOOL => {
                let args = serde_json::from_value::<MetaGuideListArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                let guide_context =
                    crate::runtime::metaagent_guides::MetaagentGuideContext::for_workspace(
                        agent.worktree_id().unwrap_or_else(|| session.worktree_id()),
                    );
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "guides": crate::runtime::metaagent_guides::list_guides_with_context(
                            crate::runtime::metaagent_guides::MetaagentGuideSearchArgs {
                                query: None,
                                tag: args.tag,
                                command: args.command,
                                limit: args.limit,
                            },
                            &guide_context,
                        ),
                    }),
                })
            }
            META_READ_GUIDE_TOOL => {
                let args = serde_json::from_value::<MetaReadGuideArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                let guide_context =
                    crate::runtime::metaagent_guides::MetaagentGuideContext::for_workspace(
                        agent.worktree_id().unwrap_or_else(|| session.worktree_id()),
                    );
                Ok(
                    match crate::runtime::metaagent_guides::read_guide_with_context(
                        &args.guide,
                        &guide_context,
                    ) {
                        Some(guide) => RuntimeToolResult {
                            ok: true,
                            payload: guide,
                        },
                        None => RuntimeToolResult {
                            ok: false,
                            payload: serde_json::json!({
                                "error": format!("unknown metaagent guide `{}`", args.guide)
                            }),
                        },
                    },
                )
            }
            META_RUN_COMMAND_TOOL => Ok(RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": "chariox.meta.run_command must be dispatched through the runtime MCP router",
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
            META_SUBSCRIBE_TRACE_TOOL => {
                let args = serde_json::from_value::<MetaSubscribeTraceArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_subscribe_trace(session, agent, args)
            }
            META_POLL_TRACE_TOOL => {
                let args = serde_json::from_value::<MetaPollTraceArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                Box::pin(self.meta_poll_trace(session, agent, args, false)).await
            }
            META_WAIT_TRACE_TOOL => {
                let args = serde_json::from_value::<MetaPollTraceArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                Box::pin(self.meta_poll_trace(session, agent, args, true)).await
            }
            META_UNSUBSCRIBE_TRACE_TOOL => {
                let args = serde_json::from_value::<MetaUnsubscribeTraceArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_unsubscribe_trace(session, agent, args)
            }
            META_SUBSCRIBE_EVENTS_TOOL => {
                let args = serde_json::from_value::<MetaSubscribeEventsArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                if !crate::transport::runtime_tools::is_known_metaagent_event_kind(&args.kind) {
                    return Ok(RuntimeToolResult {
                        ok: false,
                        payload: serde_json::json!({
                            "error": format!("unknown metaagent event kind `{}`", args.kind),
                            "kind": args.kind,
                            "suggestions": suggest_metaagent_event_kinds(&args.kind),
                            "valid_event_kinds": META_EVENT_KINDS,
                        }),
                    });
                }
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
                        "valid_event_kinds": META_EVENT_KINDS,
                    }),
                })
            }
            META_READ_TASK_TOOL => {
                let _args = serde_json::from_value::<MetaReadTaskArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                let session = self.meta_coherent_session_snapshot(session, agent)?;
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: metaagent_task_payload(&session, agent),
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
                let projected = self.owned.session_snapshot(updated.id())?;
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: metaagent_task_payload(&projected, agent),
                })
            }
            META_READ_PLAN_TOOL => {
                let _args = serde_json::from_value::<MetaReadPlanArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                let session = self.meta_coherent_session_snapshot(session, agent)?;
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: metaagent_plan_payload(&session, agent),
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
                let projected = self.persist_metaagent_task_session_update(
                    updated.id(),
                    "metaagent_plan_updated",
                )?;
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: metaagent_plan_payload(&projected, agent),
                })
            }
            META_COMPLETE_TASK_TOOL => {
                let args = serde_json::from_value::<MetaCompleteTaskArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                if self.metaagent_has_unfinished_controlled_work(session, agent) {
                    return Err(DaemonError::LocalTransport {
                        operation: "complete_metaagent_task",
                        message: "cannot complete the Meta task while a controlled agent or workflow still has active, queued, completing, or paused work; wait for it to settle or stop it first".to_string(),
                    });
                }
                let updated = self.owned.session_store.write().complete_metaagent_task(
                    session.id(),
                    agent.id(),
                    args.summary,
                )?;
                self.persist_metaagent_task_session_update(
                    updated.id(),
                    "metaagent_task_completed",
                )?;
                let projected = self
                    .deactivate_meta_mode_for_terminal_task(
                        updated.id(),
                        agent.id(),
                        "meta task completion",
                    )
                    .await?;
                self.spawn_workflow_prompt_dispatches(
                    self.owned
                        .workflow_maybe_start_next_queued_prompt(updated.id()),
                );
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: metaagent_task_payload(&projected, agent),
                })
            }
            META_MARK_BLOCKED_TOOL => {
                let args = serde_json::from_value::<MetaMarkBlockedArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                if self.metaagent_has_unfinished_controlled_work(session, agent) {
                    return Err(DaemonError::LocalTransport {
                        operation: "block_metaagent_task",
                        message: "cannot block the Meta task while a controlled agent or workflow still has active, queued, completing, or paused work; wait for it to settle or stop it first".to_string(),
                    });
                }
                let updated = self.owned.session_store.write().block_metaagent_task(
                    session.id(),
                    agent.id(),
                    args.reason,
                )?;
                self.persist_metaagent_task_session_update(updated.id(), "metaagent_task_blocked")?;
                let projected = self
                    .deactivate_meta_mode_for_terminal_task(
                        updated.id(),
                        agent.id(),
                        "meta task blocked",
                    )
                    .await?;
                self.spawn_workflow_prompt_dispatches(
                    self.owned
                        .workflow_maybe_start_next_queued_prompt(updated.id()),
                );
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: metaagent_task_payload(&projected, agent),
                })
            }
            META_WORKFLOW_CODE_CREATE_TOOL => {
                let args = serde_json::from_value::<MetaWorkflowCodeCreateArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_workflow_code_create(session, agent, args).await
            }
            META_WORKFLOW_CODE_READ_TOOL => {
                let args = serde_json::from_value::<MetaWorkflowCodeReadArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_workflow_code_read(session, agent, args).await
            }
            META_WORKFLOW_CODE_LIST_TOOL => {
                let args = serde_json::from_value::<MetaWorkflowCodeListArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_workflow_code_list(session, agent, args).await
            }
            META_WORKFLOW_CODE_UPDATE_TOOL => {
                let args = serde_json::from_value::<MetaWorkflowCodeUpdateArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_workflow_code_update(session, agent, args).await
            }
            META_WORKFLOW_CODE_DELETE_TOOL => {
                let args = serde_json::from_value::<MetaWorkflowCodeDeleteArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_workflow_code_delete(session, agent, args).await
            }
            META_WORKFLOW_CODE_VALIDATE_TOOL => {
                let args = serde_json::from_value::<MetaWorkflowCodeValidateArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_workflow_code_validate(session, agent, args).await
            }
            META_WORKFLOW_CODE_APPLY_TOOL => {
                let args = serde_json::from_value::<MetaWorkflowCodeApplyArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_workflow_code_apply(session, agent, args).await
            }
            META_WORKFLOW_CODE_RUN_TOOL => {
                let args = serde_json::from_value::<MetaWorkflowCodeRunArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_workflow_code_run(session, agent, args).await
            }
            META_WORKFLOW_CODE_EXPORT_TOOL => {
                let args = serde_json::from_value::<MetaWorkflowCodeExportArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_workflow_code_export(session, agent, args).await
            }
            META_WORKFLOW_CODE_IMPORT_TOOL => {
                let args = serde_json::from_value::<MetaWorkflowCodeImportArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_workflow_code_import(session, agent, args).await
            }
            META_WORKFLOW_CODE_PACKAGE_EXPORT_TOOL => {
                let args = serde_json::from_value::<MetaWorkflowCodePackageExportArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_workflow_code_package_export(session, agent, args)
                    .await
            }
            META_WORKFLOW_CODE_PACKAGE_IMPORT_TOOL => {
                let args = serde_json::from_value::<MetaWorkflowCodePackageImportArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_workflow_code_package_import(session, agent, args)
                    .await
            }
            META_WORKFLOW_CODE_SOURCE_EXPORT_TOOL => {
                let args = serde_json::from_value::<MetaWorkflowCodeSourceExportArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_workflow_code_source_export(session, agent, args)
                    .await
            }
            META_WORKFLOW_CODE_SOURCE_EXPORT_DIRECTORY_TOOL => {
                let args = serde_json::from_value::<MetaWorkflowCodeSourceExportDirArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_workflow_code_source_export(
                    session,
                    agent,
                    MetaWorkflowCodeSourceExportArgs {
                        name: args.name,
                        format: crate::workflow_code::WorkflowCodeSourceExportFormat::Directory,
                    },
                )
                .await
            }
            META_WORKFLOW_CODE_CANVAS_CONTRACT_TOOL => {
                let _args = serde_json::from_value::<MetaWorkflowCodeCanvasContractArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "canvas_contract": crate::workflow_code::workflow_code_canvas_contract(),
                    }),
                })
            }
            META_WORKFLOW_REGISTRY_LIST_TOOL => {
                let args = serde_json::from_value::<MetaWorkflowRegistryListArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_workflow_registry_list(session, agent, args).await
            }
            META_WORKFLOW_REGISTRY_GET_TOOL => {
                let args = serde_json::from_value::<MetaWorkflowRegistryGetArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_workflow_registry_get(session, agent, args).await
            }
            META_WORKFLOW_REGISTRY_ADD_TOOL => {
                let args = serde_json::from_value::<MetaWorkflowRegistryAddArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_workflow_registry_add(session, agent, args).await
            }
            META_WORKFLOW_REGISTRY_ADD_FROM_WORKFLOW_TOOL => {
                let args =
                    serde_json::from_value::<MetaWorkflowRegistryAddFromWorkflowArgs>(arguments)
                        .map_err(invalid_meta_args)?;
                self.meta_workflow_registry_add_from_workflow(session, agent, args)
                    .await
            }
            META_WORKFLOW_REGISTRY_DELETE_TOOL => {
                let args = serde_json::from_value::<MetaWorkflowRegistryDeleteArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_workflow_registry_delete(session, agent, args)
                    .await
            }
            META_WORKFLOW_REGISTRY_LOAD_TOOL => {
                let args = serde_json::from_value::<MetaWorkflowRegistryLoadArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_workflow_registry_load(session, agent, args).await
            }
            META_WORKFLOW_REGISTRY_RUN_TOOL => {
                let args = serde_json::from_value::<MetaWorkflowRegistryRunArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                self.meta_workflow_registry_run(session, agent, args).await
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
                message: "Meta mode tools require an agent-bound provider run".to_string(),
            });
        };
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if !agent.is_metaagent() {
            return Err(DaemonError::LocalTransport {
                operation: "runtime_tool_meta",
                message: "Meta mode tools are only available to agents currently in Meta mode"
                    .to_string(),
            });
        }
        let session = self
            .owned
            .session_store
            .get_session(provider_run.session_id())?;
        Ok((session, agent))
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
