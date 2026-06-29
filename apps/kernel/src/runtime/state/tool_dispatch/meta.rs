use super::*;

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

const META_TRACE_WAIT_DEFAULT_MS: u64 = 30_000;
const META_TRACE_WAIT_MAX_MS: u64 = 60_000;

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
                let projected = self.owned.session_snapshot(updated.id())?;
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: metaagent_plan_payload(&projected, agent),
                })
            }
            META_COMPLETE_TASK_TOOL => {
                let args = serde_json::from_value::<MetaCompleteTaskArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                let metaagent = agent.clone();
                let updated = self.owned.session_store.write().complete_metaagent_task(
                    session.id(),
                    agent.id(),
                    args.summary,
                )?;
                let projected = self
                    .deactivate_meta_mode_for_terminal_task(
                        updated.id(),
                        agent.id(),
                        "meta task completion",
                    )
                    .await?;
                self.cancel_active_metaagent_prompt_if_any(
                    updated.id(),
                    &metaagent,
                    "complete_metaagent_task",
                )
                .await?;
                Ok(RuntimeToolResult {
                    ok: true,
                    payload: metaagent_task_payload(&projected, agent),
                })
            }
            META_MARK_BLOCKED_TOOL => {
                let args = serde_json::from_value::<MetaMarkBlockedArgs>(arguments)
                    .map_err(invalid_meta_args)?;
                let metaagent = agent.clone();
                let updated = self.owned.session_store.write().block_metaagent_task(
                    session.id(),
                    agent.id(),
                    args.reason,
                )?;
                let projected = self
                    .deactivate_meta_mode_for_terminal_task(
                        updated.id(),
                        agent.id(),
                        "meta task blocked",
                    )
                    .await?;
                self.cancel_active_metaagent_prompt_if_any(
                    updated.id(),
                    &metaagent,
                    "block_metaagent_task",
                )
                .await?;
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

    async fn meta_workflow_code_create(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowCodeCreateArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::CreateWorkflowCodeArtifact(
                    crate::local::CreateWorkflowCodeArtifactRequest {
                        session_id: session.id().to_string(),
                        name: args.name,
                        language: args
                            .language
                            .unwrap_or(crate::workflow_code::WorkflowCodeLanguage::JavaScript),
                        node_path: meta_workflow_code_node_path(args.node_path)?
                            .display()
                            .to_string(),
                        source: args.source,
                    },
                ),
                agent,
            )
            .await?;
        if let crate::local::LocalDaemonResponse::WorkflowCodeArtifactCreated { artifact } =
            &response
        {
            self.persist_metaagent_workflow_code_event(
                "metaagent.workflow_code.created",
                session,
                agent,
                serde_json::json!({ "artifact": &artifact.metadata }),
            );
        }
        runtime_tool_result_from_local_response(response)
    }

    async fn meta_workflow_code_read(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowCodeReadArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::GetWorkflowCodeArtifact(
                    crate::local::GetWorkflowCodeArtifactRequest {
                        session_id: session.id().to_string(),
                        name: args.name,
                    },
                ),
                agent,
            )
            .await?;
        runtime_tool_result_from_local_response(response)
    }

    async fn meta_workflow_code_list(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        _args: MetaWorkflowCodeListArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::ListWorkflowCodeArtifacts(
                    crate::local::ListWorkflowCodeArtifactsRequest {
                        session_id: session.id().to_string(),
                    },
                ),
                agent,
            )
            .await?;
        runtime_tool_result_from_local_response(response)
    }

    async fn meta_workflow_code_update(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowCodeUpdateArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::UpdateWorkflowCodeArtifact(
                    crate::local::UpdateWorkflowCodeArtifactRequest {
                        session_id: session.id().to_string(),
                        name: args.name,
                        language: args
                            .language
                            .unwrap_or(crate::workflow_code::WorkflowCodeLanguage::JavaScript),
                        node_path: meta_workflow_code_node_path(args.node_path)?
                            .display()
                            .to_string(),
                        source: args.source,
                    },
                ),
                agent,
            )
            .await?;
        if let crate::local::LocalDaemonResponse::WorkflowCodeArtifactUpdated { artifact } =
            &response
        {
            self.persist_metaagent_workflow_code_event(
                "metaagent.workflow_code.updated",
                session,
                agent,
                serde_json::json!({ "artifact": &artifact.metadata }),
            );
        }
        runtime_tool_result_from_local_response(response)
    }

    async fn meta_workflow_code_delete(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowCodeDeleteArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::DeleteWorkflowCodeArtifact(
                    crate::local::DeleteWorkflowCodeArtifactRequest {
                        session_id: session.id().to_string(),
                        name: args.name,
                    },
                ),
                agent,
            )
            .await?;
        if let crate::local::LocalDaemonResponse::WorkflowCodeArtifactDeleted { name, path } =
            &response
        {
            self.persist_metaagent_workflow_code_event(
                "metaagent.workflow_code.deleted",
                session,
                agent,
                serde_json::json!({ "name": name, "path": path }),
            );
        }
        runtime_tool_result_from_local_response(response)
    }

    async fn meta_workflow_code_export(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowCodeExportArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::ExportWorkflowCodeArtifact(
                    crate::local::ExportWorkflowCodeArtifactRequest {
                        session_id: session.id().to_string(),
                        name: args.name,
                    },
                ),
                agent,
            )
            .await?;
        if let crate::local::LocalDaemonResponse::WorkflowCodeArtifactExported { package } =
            &response
        {
            self.persist_metaagent_workflow_code_event(
                "metaagent.workflow_code.exported",
                session,
                agent,
                serde_json::json!({
                    "name": &package.name,
                    "source_sha256": &package.source_sha256,
                    "source_bytes": package.source_bytes,
                    "package_version": package.package_version,
                }),
            );
        }
        runtime_tool_result_from_local_response(response)
    }

    async fn meta_workflow_code_import(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowCodeImportArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::ImportWorkflowCodeArtifact(
                    crate::local::ImportWorkflowCodeArtifactRequest {
                        session_id: session.id().to_string(),
                        package: args.package,
                        name: args.name,
                        overwrite: args.overwrite.unwrap_or(false),
                        node_path: meta_workflow_code_node_path(args.node_path)?
                            .display()
                            .to_string(),
                    },
                ),
                agent,
            )
            .await?;
        if let crate::local::LocalDaemonResponse::WorkflowCodeArtifactImported { artifact } =
            &response
        {
            self.persist_metaagent_workflow_code_event(
                "metaagent.workflow_code.imported",
                session,
                agent,
                serde_json::json!({ "artifact": &artifact.metadata }),
            );
        }
        runtime_tool_result_from_local_response(response)
    }

    async fn meta_workflow_code_package_export(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowCodePackageExportArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::ExportWorkflowCodePackage(
                    crate::local::ExportWorkflowCodePackageRequest {
                        session_id: session.id().to_string(),
                        name: args.name,
                        target: None,
                        agent_mode:
                            crate::workflow_code::WorkflowCodeSourceExportAgentMode::PortableGenerated,
                    },
                ),
                agent,
            )
            .await?;
        if let crate::local::LocalDaemonResponse::WorkflowCodePackageExported { package } =
            &response
        {
            self.persist_metaagent_workflow_code_event(
                "metaagent.workflow_code.package_exported",
                session,
                agent,
                serde_json::json!({
                    "name": &package.name,
                    "source_sha256": &package.source_sha256,
                    "source_bytes": package.source_bytes,
                    "package_version": package.package_version,
                }),
            );
        }
        runtime_tool_result_from_local_response(response)
    }

    async fn meta_workflow_code_package_import(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowCodePackageImportArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::ImportWorkflowCodePackage(
                    crate::local::ImportWorkflowCodePackageRequest {
                        session_id: session.id().to_string(),
                        package: args.package,
                        name: args.name,
                        overwrite: args.overwrite.unwrap_or(false),
                        node_path: meta_workflow_code_node_path(args.node_path)?
                            .display()
                            .to_string(),
                    },
                ),
                agent,
            )
            .await?;
        if let crate::local::LocalDaemonResponse::WorkflowCodePackageImported { artifact } =
            &response
        {
            self.persist_metaagent_workflow_code_event(
                "metaagent.workflow_code.package_imported",
                session,
                agent,
                serde_json::json!({ "artifact": &artifact.metadata }),
            );
        }
        runtime_tool_result_from_local_response(response)
    }

    async fn meta_workflow_code_source_export(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowCodeSourceExportArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::ExportWorkflowCodeSource(
                    crate::local::ExportWorkflowCodeSourceRequest {
                        session_id: session.id().to_string(),
                        target: crate::local::WorkflowCodeSourceExportTarget::Artifact {
                            name: args.name,
                        },
                        format: args.format,
                        agent_mode:
                            crate::workflow_code::WorkflowCodeSourceExportAgentMode::PortableGenerated,
                    },
                ),
                agent,
            )
            .await?;
        if let crate::local::LocalDaemonResponse::WorkflowCodeSourceExported { export } = &response
        {
            self.persist_metaagent_workflow_code_event(
                "metaagent.workflow_code.source_exported",
                session,
                agent,
                serde_json::json!({
                    "name": &export.name,
                    "format": export.format,
                    "source_path": &export.source_path,
                    "source_sha256": &export.source_sha256,
                    "source_bytes": export.source_bytes,
                    "definition_sha256": &export.definition_sha256,
                }),
            );
        }
        runtime_tool_result_from_local_response(response)
    }

    async fn meta_workflow_registry_list(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        _args: MetaWorkflowRegistryListArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::ListWorkflowRegistry(
                    crate::local::ListWorkflowRegistryRequest {
                        session_id: session.id().to_string(),
                    },
                ),
                agent,
            )
            .await?;
        runtime_tool_result_from_local_response(response)
    }

    async fn meta_workflow_registry_get(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowRegistryGetArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::GetWorkflowRegistryEntry(
                    crate::local::GetWorkflowRegistryEntryRequest {
                        session_id: session.id().to_string(),
                        name: args.name,
                    },
                ),
                agent,
            )
            .await?;
        runtime_tool_result_from_local_response(response)
    }

    async fn meta_workflow_registry_add(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowRegistryAddArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::AddWorkflowRegistryEntry(
                    crate::local::AddWorkflowRegistryEntryRequest {
                        session_id: session.id().to_string(),
                        name: args.name,
                        scope: args.scope,
                        source: args.source,
                        node_path: meta_workflow_code_node_path(args.node_path)?
                            .display()
                            .to_string(),
                    },
                ),
                agent,
            )
            .await?;
        if let crate::local::LocalDaemonResponse::WorkflowRegistryEntryAdded { entry } = &response {
            self.persist_metaagent_workflow_code_event(
                "metaagent.workflow_registry.added",
                session,
                agent,
                serde_json::json!({ "entry": entry }),
            );
        }
        runtime_tool_result_from_local_response(response)
    }

    async fn meta_workflow_registry_add_from_workflow(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowRegistryAddFromWorkflowArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::AddWorkflowRegistryEntryFromWorkflow(
                    crate::local::AddWorkflowRegistryEntryFromWorkflowRequest {
                        session_id: session.id().to_string(),
                        name: args.name,
                        workflow_ref: args.workflow_ref,
                        scope: args.scope,
                        agent_mode: args.agent_mode,
                    },
                ),
                agent,
            )
            .await?;
        if let crate::local::LocalDaemonResponse::WorkflowRegistryEntryAdded { entry } = &response {
            self.persist_metaagent_workflow_code_event(
                "metaagent.workflow_registry.added_from_workflow",
                session,
                agent,
                serde_json::json!({ "entry": entry }),
            );
        }
        runtime_tool_result_from_local_response(response)
    }

    async fn meta_workflow_registry_delete(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowRegistryDeleteArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::DeleteWorkflowRegistryEntry(
                    crate::local::DeleteWorkflowRegistryEntryRequest {
                        session_id: session.id().to_string(),
                        name: args.name,
                        scope: args.scope,
                    },
                ),
                agent,
            )
            .await?;
        if let crate::local::LocalDaemonResponse::WorkflowRegistryEntryDeleted { name, path } =
            &response
        {
            self.persist_metaagent_workflow_code_event(
                "metaagent.workflow_registry.deleted",
                session,
                agent,
                serde_json::json!({ "name": name, "path": path }),
            );
        }
        runtime_tool_result_from_local_response(response)
    }

    async fn meta_workflow_registry_load(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowRegistryLoadArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::LoadWorkflowRegistryEntry(
                    crate::local::LoadWorkflowRegistryEntryRequest {
                        session_id: session.id().to_string(),
                        name: args.name,
                        parameters: args.parameters,
                        provider_rebindings: args.provider_rebindings,
                    },
                ),
                agent,
            )
            .await?;
        if let crate::local::LocalDaemonResponse::WorkflowRegistryEntryLoaded {
            entry,
            result,
            ..
        } = &response
        {
            self.persist_metaagent_workflow_code_event(
                "metaagent.workflow_registry.loaded",
                session,
                agent,
                serde_json::json!({ "entry": entry, "apply": &result.apply }),
            );
        }
        runtime_tool_result_from_local_response(response)
    }

    async fn meta_workflow_registry_run(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowRegistryRunArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::RunWorkflowRegistryEntry(
                    crate::local::RunWorkflowRegistryEntryRequest {
                        session_id: session.id().to_string(),
                        name: args.name,
                        parameters: args.parameters,
                        provider_rebindings: args.provider_rebindings,
                        endpoint: args.endpoint,
                        queue_ref: args.queue,
                        prompt: args.prompt.unwrap_or_default(),
                    },
                ),
                agent,
            )
            .await?;
        if let crate::local::LocalDaemonResponse::WorkflowRegistryEntryRun {
            entry, result, ..
        } = &response
        {
            self.persist_metaagent_workflow_code_event(
                "metaagent.workflow_registry.run",
                session,
                agent,
                serde_json::json!({
                    "entry": entry,
                    "run": meta_workflow_code_run_audit_payload(result),
                }),
            );
        }
        runtime_tool_result_from_local_response(response)
    }

    async fn meta_workflow_code_validate(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowCodeValidateArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        if let (Some(name), None) = (&args.name, &args.source) {
            let artifact = meta_workflow_code_artifact(session, name)?;
            let provider_rebindings = args.provider_rebindings;
            let session_id = session.id().to_string();
            let metaagent_id = agent.id().to_string();
            let response = self
                .with_app_side_effect(move |app| {
                    let limits = app.config().workflow_code_limits();
                    let (definition, validation) = crate::app::KernelSessionService::new(app)
                        .validate_workflow_code_definition_with_rebindings(
                            &session_id,
                            &artifact.definition,
                            &limits,
                            &provider_rebindings,
                            Some(&metaagent_id),
                        )?;
                    Ok::<_, DaemonError>(crate::local::LocalDaemonResponse::WorkflowCodeValidated {
                        result: crate::workflow_code::WorkflowCodeCompileResult {
                            definition,
                            validation,
                            logs: String::new(),
                            source_spans: std::collections::BTreeMap::new(),
                        },
                    })
                })
                .await?;
            return runtime_tool_result_from_local_response(response);
        }
        let source = meta_workflow_code_source(session, args.name, args.source)?;
        let response = self
            .meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::ValidateWorkflowCode(
                    crate::local::ValidateWorkflowCodeRequest {
                        session_id: session.id().to_string(),
                        node_path: meta_workflow_code_node_path(args.node_path)?
                            .display()
                            .to_string(),
                        source,
                        language: args.language,
                        provider_rebindings: args.provider_rebindings,
                    },
                ),
                agent,
            )
            .await?;
        runtime_tool_result_from_local_response(response)
    }

    async fn meta_workflow_code_apply(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowCodeApplyArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let artifact_name = args.name.clone();
        let applies_saved_artifact = matches!((&args.name, &args.source), (Some(_), None));
        let response = if let (Some(name), None) = (&args.name, &args.source) {
            self.meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::ApplyWorkflowCodeArtifact(
                    crate::local::ApplyWorkflowCodeArtifactRequest {
                        session_id: session.id().to_string(),
                        name: name.clone(),
                        provider_rebindings: args.provider_rebindings,
                    },
                ),
                agent,
            )
            .await?
        } else {
            let source = meta_workflow_code_source(session, args.name, args.source)?;
            self.meta_workflow_code_apply_response(
                session,
                agent,
                source,
                args.node_path,
                args.provider_rebindings,
                args.language,
            )
            .await?
        };
        if let crate::local::LocalDaemonResponse::WorkflowCodeApplied { result, .. } = &response {
            if !applies_saved_artifact {
                self.record_metaagent_workflow_code_artifact_history(
                    session,
                    agent,
                    artifact_name.as_deref(),
                    crate::workflow_code::WorkflowCodeArtifactHistoryAction::Applied,
                    &result.apply,
                );
            }
            self.persist_metaagent_workflow_code_event(
                "metaagent.workflow_code.applied",
                session,
                agent,
                serde_json::json!({ "apply": &result.apply }),
            );
        }
        runtime_tool_result_from_local_response(response)
    }

    async fn meta_workflow_code_run(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: MetaWorkflowCodeRunArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let artifact_name = args.name.clone();
        let runs_saved_artifact = matches!((&args.name, &args.source), (Some(_), None));
        let response = if let (Some(name), None) = (&args.name, &args.source) {
            self.meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::RunWorkflowCodeArtifact(
                    crate::local::RunWorkflowCodeArtifactRequest {
                        session_id: session.id().to_string(),
                        name: name.clone(),
                        provider_rebindings: args.provider_rebindings,
                        endpoint: args.endpoint,
                        queue_ref: args.queue,
                        prompt: args.prompt.unwrap_or_default(),
                    },
                ),
                agent,
            )
            .await?
        } else {
            let source = meta_workflow_code_source(session, args.name, args.source)?;
            self.meta_execute_workflow_request(
                crate::local::LocalDaemonRequest::RunWorkflowCode(
                    crate::local::RunWorkflowCodeRequest {
                        session_id: session.id().to_string(),
                        node_path: meta_workflow_code_node_path(args.node_path)?
                            .display()
                            .to_string(),
                        source,
                        language: args.language,
                        provider_rebindings: args.provider_rebindings,
                        endpoint: args.endpoint,
                        queue_ref: args.queue,
                        prompt: args.prompt.unwrap_or_default(),
                    },
                ),
                agent,
            )
            .await?
        };
        let run_result = match &response {
            crate::local::LocalDaemonResponse::WorkflowCodeRun { result, .. } => result,
            _ => {
                return Err(DaemonError::LocalTransport {
                    operation: "meta.workflow_code.run",
                    message: "workflow-code run returned an unexpected response".to_string(),
                })
            }
        };
        self.persist_metaagent_workflow_code_event(
            "metaagent.workflow_code.run",
            session,
            agent,
            meta_workflow_code_run_audit_payload(run_result),
        );
        if !runs_saved_artifact {
            self.record_metaagent_workflow_code_artifact_history(
                session,
                agent,
                artifact_name.as_deref(),
                crate::workflow_code::WorkflowCodeArtifactHistoryAction::Run,
                &run_result.apply.apply,
            );
        }
        runtime_tool_result_from_local_response(response)
    }

    async fn meta_workflow_code_apply_response(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        source: String,
        node_path: Option<String>,
        provider_rebindings: Vec<crate::workflow_code::WorkflowCodeProviderRebinding>,
        language: Option<crate::workflow_code::WorkflowCodeLanguage>,
    ) -> Result<crate::local::LocalDaemonResponse, DaemonError> {
        self.meta_execute_workflow_request(
            crate::local::LocalDaemonRequest::ApplyWorkflowCode(
                crate::local::ApplyWorkflowCodeRequest {
                    session_id: session.id().to_string(),
                    node_path: meta_workflow_code_node_path(node_path)?
                        .display()
                        .to_string(),
                    source,
                    language,
                    provider_rebindings,
                },
            ),
            agent,
        )
        .await
    }

    async fn meta_execute_workflow_request(
        &self,
        request: crate::local::LocalDaemonRequest,
        agent: &crate::agent::AgentInstance,
    ) -> Result<crate::local::LocalDaemonResponse, DaemonError> {
        let (response, _) = self
            .execute_workflow_request(
                request,
                agent.owner_user_id().to_string(),
                Some(agent.id().to_string()),
            )
            .await;
        response
    }

    fn persist_metaagent_workflow_code_event(
        &self,
        kind: &'static str,
        session: &crate::session::RuntimeSession,
        metaagent: &crate::agent::AgentInstance,
        payload: serde_json::Value,
    ) {
        if let Err(error) = self.owned.durable_state_store.append_event(
            kind,
            Some(metaagent.id().to_string()),
            serde_json::json!({
                "session_id": session.id(),
                "metaagent_id": metaagent.id(),
                "owner_user_id": metaagent.owner_user_id(),
                "payload": payload,
                "timestamp_ms": crate::session::unix_epoch_ms(),
            }),
        ) {
            crate::logging::warn_with_fields(
                "metaagent.workflow_code",
                "failed to persist metaagent workflow-code audit",
                serde_json::json!({
                    "kind": kind,
                    "session_id": session.id(),
                    "metaagent_id": metaagent.id(),
                    "error": error.to_string(),
                }),
            );
        }
    }

    fn record_metaagent_workflow_code_artifact_history(
        &self,
        session: &crate::session::RuntimeSession,
        metaagent: &crate::agent::AgentInstance,
        artifact_name: Option<&str>,
        action: crate::workflow_code::WorkflowCodeArtifactHistoryAction,
        apply_report: &crate::workflow_code::WorkflowCodeApplyReport,
    ) {
        let Some(artifact_name) = artifact_name else {
            return;
        };
        let actor = crate::workflow_code::WorkflowCodeArtifactActor::new(
            metaagent.owner_user_id().to_string(),
            Some(metaagent.id().to_string()),
        );
        match meta_workflow_code_artifact_registry(session).and_then(|registry| {
            registry.record_apply_history(artifact_name, actor, action, apply_report)
        }) {
            Ok(_) => {}
            Err(error) => crate::logging::warn_with_fields(
                "metaagent.workflow_code",
                "failed to record workflow-code artifact apply history",
                serde_json::json!({
                    "session_id": session.id(),
                    "metaagent_id": metaagent.id(),
                    "artifact": artifact_name,
                    "error": error.to_string(),
                }),
            ),
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

    fn meta_session_overview(
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

    fn meta_coherent_session_snapshot(
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

    fn meta_subscribe_trace(
        &self,
        session: &crate::session::RuntimeSession,
        metaagent: &crate::agent::AgentInstance,
        args: MetaSubscribeTraceArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let Some(mode) =
            crate::runtime::metaagent_trace::MetaagentTraceMode::parse(args.mode.as_deref())
        else {
            return Ok(RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": "trace mode must be `compact` or `verbose`",
                    "mode": args.mode,
                }),
            });
        };
        let target = match self.meta_owned_regular_agent(session.id(), metaagent, &args.agent_ref) {
            Ok(agent) => agent,
            Err(error) => {
                return Ok(RuntimeToolResult {
                    ok: false,
                    payload: serde_json::json!({ "error": error.to_string() }),
                });
            }
        };
        let subscription = self.owned.metaagent_trace_subscriptions.subscribe(
            session.id(),
            metaagent.id(),
            target.id(),
            mode,
        );
        Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "subscription": subscription,
                "agent": meta_agent_ref_json(&target),
                "message": "subscribed to live worker trace; prompt the worker after subscribing, then call wait_trace for normal supervision or poll_trace for a nonblocking snapshot",
            }),
        })
    }

    async fn meta_poll_trace(
        &self,
        session: &crate::session::RuntimeSession,
        metaagent: &crate::agent::AgentInstance,
        args: MetaPollTraceArgs,
        wait: bool,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let subscription = if let Some(subscription_id) = args.subscription_id.as_deref() {
            self.owned
                .metaagent_trace_subscriptions
                .get_for_metaagent(metaagent.id(), subscription_id)
        } else if let Some(agent_ref) = args.agent_ref.as_deref() {
            let target = match self.meta_owned_regular_agent(session.id(), metaagent, agent_ref) {
                Ok(agent) => agent,
                Err(error) => {
                    return Ok(RuntimeToolResult {
                        ok: false,
                        payload: serde_json::json!({ "error": error.to_string() }),
                    });
                }
            };
            self.owned.metaagent_trace_subscriptions.get_for_target(
                metaagent.id(),
                session.id(),
                target.id(),
            )
        } else {
            return Ok(RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": "poll_trace requires subscription_id or agent_ref",
                }),
            });
        };
        let Some(subscription) = subscription else {
            return Ok(RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": "no live trace subscription matched; call subscribe_trace before prompting the worker",
                }),
            });
        };
        if subscription.session_id != session.id() {
            return Ok(RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": "trace subscription belongs to a different session",
                    "subscription_id": subscription.subscription_id,
                }),
            });
        }
        let Some(mode) =
            crate::runtime::metaagent_trace::MetaagentTraceMode::parse(args.mode.as_deref())
        else {
            return Ok(RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": "trace mode must be `compact` or `verbose`",
                    "mode": args.mode,
                }),
            });
        };
        let mode = if args.mode.is_some() {
            mode
        } else {
            subscription.mode
        };
        let limit = args.limit.unwrap_or(50).clamp(1, 100);
        let until = MetaTraceWaitUntil::parse(args.until.as_deref());
        let wait_ms = if wait {
            args.wait_ms
                .unwrap_or(META_TRACE_WAIT_DEFAULT_MS)
                .clamp(1, META_TRACE_WAIT_MAX_MS)
        } else {
            0
        };
        let started_at = std::time::Instant::now();
        let mut items = Vec::new();
        let mut drained_count = 0usize;
        let mut suppressed_count = 0usize;
        let mut matched = false;
        loop {
            let batch = self.meta_drain_trace_batch(session.id(), &subscription, mode, limit);
            drained_count += batch.drained_count;
            suppressed_count += batch.suppressed_count;
            matched = matched || batch.matches_until(until);
            extend_meta_trace_items(&mut items, batch.items, limit);
            if !wait || matched || started_at.elapsed().as_millis() >= wait_ms as u128 {
                break;
            }
            let Some(remaining) = meta_trace_wait_remaining(started_at, wait_ms) else {
                break;
            };
            let (observed_sequence, notify) = self
                .owned
                .metaagent_trace_subscriptions
                .watch_target_activity(session.id(), &subscription.target_agent_id);

            let batch = self.meta_drain_trace_batch(session.id(), &subscription, mode, limit);
            drained_count += batch.drained_count;
            suppressed_count += batch.suppressed_count;
            matched = matched || batch.matches_until(until);
            extend_meta_trace_items(&mut items, batch.items, limit);
            if matched || started_at.elapsed().as_millis() >= wait_ms as u128 {
                break;
            }

            let notified = notify.notified();
            tokio::pin!(notified);
            let latest_sequence = self
                .owned
                .metaagent_trace_subscriptions
                .target_activity_sequence(session.id(), &subscription.target_agent_id);
            if latest_sequence > observed_sequence {
                continue;
            }
            if tokio::time::timeout(remaining, notified).await.is_err() {
                break;
            }
        }
        let agent_activity = self.agent_activity_for_session(session);
        let worker_activity = agent_activity.get(&subscription.target_agent_id).cloned();
        let supervision = self.meta_trace_supervision_summary(
            session,
            metaagent,
            &subscription,
            mode,
            until,
            wait,
            matched,
            &items,
            worker_activity.as_ref(),
            drained_count,
            suppressed_count,
        );
        Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "subscription": subscription,
                "mode": mode,
                "until": until.as_str(),
                "wait_ms": wait_ms,
                "timed_out": wait && !matched,
                "matched": matched,
                "drained_count": drained_count,
                "suppressed_count": suppressed_count,
                "items": items,
                "empty": items.is_empty(),
                "worker_activity": worker_activity,
                "supervision": supervision,
            }),
        })
    }

    fn meta_trace_supervision_summary(
        &self,
        session: &crate::session::RuntimeSession,
        metaagent: &crate::agent::AgentInstance,
        subscription: &crate::runtime::metaagent_trace::MetaagentTraceSubscription,
        mode: crate::runtime::metaagent_trace::MetaagentTraceMode,
        until: MetaTraceWaitUntil,
        wait: bool,
        matched: bool,
        items: &[serde_json::Value],
        worker_activity: Option<&crate::runtime::projection::AgentRuntimeActivity>,
        drained_count: usize,
        suppressed_count: usize,
    ) -> serde_json::Value {
        let agent_activity = self.agent_activity_for_session(session);
        let active_owned_workers = self
            .meta_owned_regular_agents(session.id(), metaagent)
            .into_iter()
            .filter_map(|agent| {
                let activity = agent_activity.get(agent.id())?;
                if activity.busy {
                    Some(serde_json::json!({
                        "agent": meta_owned_agent_ref_json(&agent),
                        "activity": activity,
                    }))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        let target_agent = self
            .owned
            .agent_store
            .get_agent(&subscription.target_agent_id)
            .ok()
            .map(|agent| meta_owned_agent_ref_json(&agent));
        let last_meaningful_output = items.iter().rev().find_map(|item| {
            let kind = item_kind(item)?;
            if kind == "prompt_echo" {
                return None;
            }
            if !item
                .get("worker_generated")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                return None;
            }
            Some(serde_json::json!({
                "kind": kind,
                "title": item.get("title").cloned().unwrap_or(serde_json::Value::Null),
                "summary": item.get("summary").cloned().unwrap_or(serde_json::Value::Null),
                "excerpt": item.get("excerpt").cloned().unwrap_or(serde_json::Value::Null),
            }))
        });
        let completion_events = items
            .iter()
            .filter(|item| item_kind(item) == Some("assistant_message_completed"))
            .cloned()
            .collect::<Vec<_>>();
        let failure_events = items
            .iter()
            .filter(|item| {
                matches!(
                    item_kind(item),
                    Some("provider_error") | Some("runtime_notice")
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        let worker_busy = worker_activity.is_some_and(|activity| activity.busy);
        let suggested_next_action = if !failure_events.is_empty() {
            "review the failure event, inspect the worker turn if needed, then either steer the owned worker or mark the task blocked"
        } else if matched && !completion_events.is_empty() {
            "review the completed worker output; if it proves the task goal, call complete_task"
        } else if matched {
            "review the worker output; continue supervision, prompt the owned worker, or complete if the task goal is proven"
        } else if items.is_empty() && wait {
            "no meaningful worker output arrived before the wait ended; wait again, inspect turn_overview, or yield for kernel continuation"
        } else if worker_busy {
            "the worker is still active; call wait_trace with a clear until condition or yield for kernel continuation"
        } else if items.is_empty() {
            "no buffered meaningful worker output is available yet; subscribe before prompting workers and use wait_trace for live supervision"
        } else {
            "inspect the compact trace items; use verbose mode only if the summary is insufficient"
        };
        serde_json::json!({
            "mode": mode,
            "until": until.as_str(),
            "matched": matched,
            "target_agent": target_agent,
            "active_owned_workers": active_owned_workers,
            "last_meaningful_output": last_meaningful_output,
            "completion_events": completion_events,
            "failure_events": failure_events,
            "drained_count": drained_count,
            "suppressed_count": suppressed_count,
            "message": if items.is_empty() {
                "no meaningful worker output yet"
            } else {
                "worker trace activity available"
            },
            "suggested_next_action": suggested_next_action,
        })
    }

    fn meta_drain_trace_batch(
        &self,
        session_id: &str,
        subscription: &crate::runtime::metaagent_trace::MetaagentTraceSubscription,
        mode: crate::runtime::metaagent_trace::MetaagentTraceMode,
        limit: usize,
    ) -> MetaTraceBatch {
        let records = self
            .owned
            .terminal_stream
            .drain_output_records(session_id, &subscription.recipient_attachment_id);
        let completions = self
            .owned
            .terminal_stream
            .drain_completion_records(session_id, &subscription.recipient_attachment_id);
        let notices = self
            .owned
            .terminal_stream
            .drain_notice_records(session_id, &subscription.recipient_attachment_id);
        let mut items = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        let mut drained_count = 0usize;
        let mut suppressed_count = 0usize;
        for record in records {
            drained_count += 1;
            let item = meta_trace_output_item(record, mode);
            if self.meta_trace_should_emit_item(&subscription.subscription_id, mode, &item)
                && seen.insert(meta_trace_item_key(&item))
            {
                extend_meta_trace_items(&mut items, vec![item], limit);
            } else {
                suppressed_count += 1;
            }
        }
        for completion in completions {
            drained_count += 1;
            let item = serde_json::json!({
                "kind": "assistant_message_completed",
                "provider_run_id": completion.provider_run_id,
                "agent_id": completion.agent_id,
                "message_id": completion.message_id,
                "completed_at_ms": completion.completed_at_ms,
                "worker_generated": true,
            });
            if self.meta_trace_should_emit_item(&subscription.subscription_id, mode, &item)
                && seen.insert(meta_trace_item_key(&item))
            {
                extend_meta_trace_items(&mut items, vec![item], limit);
            } else {
                suppressed_count += 1;
            }
        }
        for notice in notices {
            drained_count += 1;
            let item = serde_json::json!({
                "kind": "runtime_notice",
                "provider_run_id": notice.provider_run_id,
                "agent_id": notice.agent_id,
                "summary": truncate_single_line(&notice.message, 240),
                "text": if mode == crate::runtime::metaagent_trace::MetaagentTraceMode::Verbose {
                    Some(truncate_text(&notice.message, 8_000))
                } else {
                    None
                },
                "worker_generated": false,
            });
            if self.meta_trace_should_emit_item(&subscription.subscription_id, mode, &item)
                && seen.insert(meta_trace_item_key(&item))
            {
                extend_meta_trace_items(&mut items, vec![item], limit);
            } else {
                suppressed_count += 1;
            }
        }
        MetaTraceBatch {
            items,
            drained_count,
            suppressed_count,
        }
    }

    fn meta_trace_should_emit_item(
        &self,
        subscription_id: &str,
        mode: crate::runtime::metaagent_trace::MetaagentTraceMode,
        item: &serde_json::Value,
    ) -> bool {
        if mode == crate::runtime::metaagent_trace::MetaagentTraceMode::Verbose {
            return true;
        }
        self.owned
            .metaagent_trace_subscriptions
            .remember_compact_item_key(subscription_id, meta_trace_item_key(item))
    }

    fn meta_unsubscribe_trace(
        &self,
        session: &crate::session::RuntimeSession,
        metaagent: &crate::agent::AgentInstance,
        args: MetaUnsubscribeTraceArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let removed = self
            .owned
            .metaagent_trace_subscriptions
            .unsubscribe(metaagent.id(), &args.subscription_id);
        if let Some(subscription) = removed.as_ref() {
            let _ = self
                .owned
                .terminal_stream
                .drain_output_records(session.id(), &subscription.recipient_attachment_id);
            let _ = self
                .owned
                .terminal_stream
                .drain_completion_records(session.id(), &subscription.recipient_attachment_id);
            let _ = self
                .owned
                .terminal_stream
                .drain_notice_records(session.id(), &subscription.recipient_attachment_id);
        }
        Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "subscription_id": args.subscription_id,
                "status": if removed.is_some() { "removed" } else { "not_found" },
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
                !agent.is_metaagent() && agent.controlled_by_metaagent_id() == Some(metaagent.id())
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
                message: owned_regular_agent_error_message(
                    reference,
                    &self.meta_owned_regular_agents(session_id, metaagent),
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

fn meta_workflow_code_node_path(
    node_path: Option<String>,
) -> Result<std::path::PathBuf, DaemonError> {
    node_path
        .map(std::path::PathBuf::from)
        .map(Ok)
        .unwrap_or_else(crate::workflow_code::discover_workflow_code_node_path)
}

fn meta_workflow_code_source(
    session: &crate::session::RuntimeSession,
    name: Option<String>,
    source: Option<String>,
) -> Result<String, DaemonError> {
    match (name, source) {
        (None, Some(source)) => Ok(source),
        (Some(name), None) => meta_workflow_code_artifact_registry(session)?
            .get(&name)?
            .map(|artifact| artifact.source)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "meta.workflow_code",
                message: format!("workflow-code artifact `{name}` is not saved"),
            }),
        (Some(_), Some(_)) => Err(DaemonError::LocalTransport {
            operation: "meta.workflow_code",
            message: "pass either name or source, not both".to_string(),
        }),
        (None, None) => Err(DaemonError::LocalTransport {
            operation: "meta.workflow_code",
            message: "pass either name or source".to_string(),
        }),
    }
}

fn meta_workflow_code_artifact(
    session: &crate::session::RuntimeSession,
    name: &str,
) -> Result<crate::workflow_code::WorkflowCodeArtifact, DaemonError> {
    meta_workflow_code_artifact_registry(session)?
        .get(name)?
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "meta.workflow_code",
            message: format!("workflow-code artifact `{name}` is not saved"),
        })
}

fn meta_workflow_code_run_audit_payload(
    result: &crate::workflow_code::WorkflowCodeRunResult,
) -> serde_json::Value {
    match &result.invocation {
        crate::workflow_code::WorkflowCodeRunInvocation::Started {
            workflow_run,
            workflow,
            endpoint,
        } => serde_json::json!({
            "outcome": "invoked",
            "apply": &result.apply.apply,
            "workflow_id": workflow.id(),
            "endpoint_id": endpoint.id(),
            "workflow_run_id": workflow_run.id(),
        }),
        crate::workflow_code::WorkflowCodeRunInvocation::Enqueued {
            queued_prompt,
            workflow,
            endpoint,
        } => serde_json::json!({
            "outcome": "enqueued",
            "apply": &result.apply.apply,
            "workflow_id": workflow.id(),
            "endpoint_id": endpoint.id(),
            "queued_prompt_id": queued_prompt.id(),
            "queue_id": queued_prompt.queue_id(),
        }),
    }
}

fn meta_workflow_code_artifact_registry(
    session: &crate::session::RuntimeSession,
) -> Result<crate::workflow_code::WorkflowCodeArtifactRegistry, DaemonError> {
    let mut roots = Vec::new();
    if !session.workspace_id().trim().is_empty() {
        roots.push(
            crate::workflow_code::WorkflowCodeArtifactRegistry::project_root(
                session.workspace_id(),
            ),
        );
    }
    if let Some(root) = crate::workflow_code::WorkflowCodeArtifactRegistry::user_root() {
        roots.push(root);
    }
    Ok(crate::workflow_code::WorkflowCodeArtifactRegistry::new(
        roots,
    ))
}

fn runtime_tool_result_from_local_response(
    response: crate::local::LocalDaemonResponse,
) -> Result<RuntimeToolResult, DaemonError> {
    Ok(RuntimeToolResult {
        ok: true,
        payload: local_response_to_value(&response)?,
    })
}

fn local_response_to_value(
    response: &crate::local::LocalDaemonResponse,
) -> Result<serde_json::Value, DaemonError> {
    serde_json::to_value(response).map_err(|error| DaemonError::LocalTransport {
        operation: "runtime_tool_meta",
        message: format!("failed to serialize workflow-code response: {error}"),
    })
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

fn meta_trace_output_item(
    record: crate::terminal::TerminalOutputRecord,
    mode: crate::runtime::metaagent_trace::MetaagentTraceMode,
) -> serde_json::Value {
    let text = String::from_utf8_lossy(&record.bytes).into_owned();
    let worker_generated = record.kind != crate::terminal::TerminalOutputKind::PromptEcho;
    let (title, summary) = match record.kind {
        crate::terminal::TerminalOutputKind::ProviderTool => {
            let (title, summary) = summarize_tool_trace(&text);
            (title, summary)
        }
        crate::terminal::TerminalOutputKind::ProviderOutput => {
            ("assistant".to_string(), truncate_single_line(&text, 240))
        }
        crate::terminal::TerminalOutputKind::ProviderReasoning => {
            ("thinking".to_string(), truncate_single_line(&text, 240))
        }
        crate::terminal::TerminalOutputKind::ProviderError => {
            ("error".to_string(), truncate_single_line(&text, 240))
        }
        crate::terminal::TerminalOutputKind::ProviderStatus => {
            ("status".to_string(), truncate_single_line(&text, 240))
        }
        crate::terminal::TerminalOutputKind::PromptEcho => {
            ("prompt".to_string(), truncate_single_line(&text, 240))
        }
    };
    let mut item = serde_json::json!({
        "kind": &record.kind,
        "provider_run_id": record.provider_run_id,
        "agent_id": record.agent_id,
        "merge_key": record.merge_key,
        "title": title,
        "summary": summary,
        "byte_len": record.bytes.len(),
        "worker_generated": worker_generated,
    });
    if mode == crate::runtime::metaagent_trace::MetaagentTraceMode::Verbose {
        item["text"] = serde_json::json!(truncate_text(&text, 8_000));
    } else if record.kind != crate::terminal::TerminalOutputKind::ProviderTool {
        item["excerpt"] = serde_json::json!(truncate_text(&text, 1_000));
    }
    item
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MetaTraceWaitUntil {
    Any,
    Activity,
    WorkerOutput,
    Completion,
    Error,
}

impl MetaTraceWaitUntil {
    fn parse(value: Option<&str>) -> Self {
        match value.unwrap_or("any") {
            "activity" => Self::Activity,
            "worker_output" => Self::WorkerOutput,
            "completion" => Self::Completion,
            "error" => Self::Error,
            _ => Self::Any,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::Activity => "activity",
            Self::WorkerOutput => "worker_output",
            Self::Completion => "completion",
            Self::Error => "error",
        }
    }
}

struct MetaTraceBatch {
    items: Vec<serde_json::Value>,
    drained_count: usize,
    suppressed_count: usize,
}

impl MetaTraceBatch {
    fn matches_until(&self, until: MetaTraceWaitUntil) -> bool {
        self.items.iter().any(|item| match until {
            MetaTraceWaitUntil::Any => true,
            MetaTraceWaitUntil::Activity => item
                .get("worker_generated")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            MetaTraceWaitUntil::WorkerOutput => item_kind(item) == Some("provider_output"),
            MetaTraceWaitUntil::Completion => {
                item_kind(item) == Some("assistant_message_completed")
                    || item_kind(item) == Some("provider_output")
            }
            MetaTraceWaitUntil::Error => {
                item_kind(item) == Some("runtime_notice")
                    || item_kind(item) == Some("provider_error")
            }
        })
    }
}

fn meta_trace_wait_remaining(
    started_at: std::time::Instant,
    wait_ms: u64,
) -> Option<std::time::Duration> {
    let elapsed_ms = u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
    if elapsed_ms >= wait_ms {
        return None;
    }
    Some(std::time::Duration::from_millis(wait_ms - elapsed_ms))
}

fn item_kind(item: &serde_json::Value) -> Option<&str> {
    item.get("kind").and_then(serde_json::Value::as_str)
}

fn meta_trace_item_key(item: &serde_json::Value) -> String {
    serde_json::json!({
        "kind": item.get("kind"),
        "provider_run_id": item.get("provider_run_id"),
        "agent_id": item.get("agent_id"),
        "merge_key": item.get("merge_key"),
        "title": item.get("title"),
        "summary": item.get("summary"),
    })
    .to_string()
}

fn extend_meta_trace_items(
    items: &mut Vec<serde_json::Value>,
    new_items: Vec<serde_json::Value>,
    limit: usize,
) {
    for item in new_items {
        if items.len() >= limit {
            break;
        }
        items.push(item);
    }
}

fn summarize_tool_trace(text: &str) -> (String, String) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return ("tool".to_string(), truncate_single_line(text, 240));
    };
    let tool = value
        .get("tool")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("tool");
    let status = value
        .get("status")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let title = match status {
        Some(status) => format!("{tool} · {}", status.to_ascii_uppercase()),
        None => tool.to_string(),
    };
    let summary = value
        .pointer("/input/command")
        .and_then(serde_json::Value::as_str)
        .map(|command| format!("$ {command}"))
        .or_else(|| {
            value
                .get("description")
                .or_else(|| value.get("title"))
                .or_else(|| value.get("output"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| truncate_single_line(text, 240));
    (title, truncate_single_line(&summary, 240))
}

fn truncate_single_line(text: &str, max_chars: usize) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let line = normalized
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    truncate_text(line, max_chars)
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let truncated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

fn suggest_metaagent_event_kinds(input: &str) -> Vec<&'static str> {
    let normalized_input = normalize_event_kind_for_suggestion(input);
    let mut scored = META_EVENT_KINDS
        .iter()
        .filter_map(|kind| {
            let normalized_kind = normalize_event_kind_for_suggestion(kind);
            let score = event_kind_suggestion_score(&normalized_input, &normalized_kind);
            (score > 0).then_some((*kind, score))
        })
        .collect::<Vec<_>>();
    scored.sort_by(|(left_kind, left_score), (right_kind, right_score)| {
        right_score
            .cmp(left_score)
            .then_with(|| left_kind.cmp(right_kind))
    });
    scored.into_iter().take(3).map(|(kind, _)| kind).collect()
}

fn normalize_event_kind_for_suggestion(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect()
}

fn event_kind_suggestion_score(input: &str, candidate: &str) -> u8 {
    if input.is_empty() {
        return 0;
    }
    if input == candidate {
        return 100;
    }
    if candidate.contains(input) || input.contains(candidate) {
        return 80;
    }
    let input_tokens = event_kind_tokens(input);
    let candidate_tokens = event_kind_tokens(candidate);
    let overlap = input_tokens
        .iter()
        .filter(|token| candidate_tokens.iter().any(|candidate| candidate == *token))
        .count();
    if overlap > 0 {
        return 40 + overlap.min(4) as u8 * 10;
    }
    let common_prefix = input
        .chars()
        .zip(candidate.chars())
        .take_while(|(left, right)| left == right)
        .count();
    (common_prefix >= 5).then_some(20).unwrap_or(0)
}

fn event_kind_tokens(normalized: &str) -> Vec<&str> {
    const TOKENS: &[&str] = &[
        "agent",
        "turn",
        "completed",
        "failed",
        "runtime",
        "interaction",
        "workflow",
        "run",
        "started",
        "updated",
        "cancelled",
        "output",
        "final",
        "intermediate",
    ];
    TOKENS
        .iter()
        .copied()
        .filter(|token| normalized.contains(token))
        .collect()
}

fn meta_agent_ref_json(agent: &crate::agent::AgentInstance) -> serde_json::Value {
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

fn meta_owned_agent_ref_json(agent: &crate::agent::AgentInstance) -> serde_json::Value {
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
