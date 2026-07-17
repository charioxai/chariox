use super::*;

impl KernelRuntimeState {
    pub(super) async fn settle_owned_provider_prompt(
        &self,
        session_id: &str,
        provider_run_id: &str,
        prompt_completed: bool,
        saw_settlement_blocking_activity: bool,
        force: bool,
    ) -> Result<crate::app::ProviderRunExitSessionSummary, DaemonError> {
        let owned = &self.owned;
        let provider_run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        let agent_id = provider_run
            .agent_instance_id()
            .map(str::to_string)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: "provider run has no agent".to_string(),
            })?;
        let active_prompt = owned
            .prompt_state_owner
            .active_prompt_for_agent(&owned.session_store.get_session(session_id)?, &agent_id);
        let Some(active_prompt) = active_prompt else {
            if !force && !prompt_completed {
                crate::logging::debug_with_fields(
                    "daemon.provider",
                    "settle provider prompt skipped without completion signal",
                    serde_json::json!({
                        "session_id": session_id,
                        "provider_run_id": provider_run_id,
                        "agent_id": agent_id,
                        "prompt_completed": prompt_completed,
                        "force": force,
                    }),
                );
                return Ok(crate::app::ProviderRunExitSessionSummary {
                    had_active_prompt: false,
                    started_next_prompt: false,
                });
            }
            crate::logging::debug_with_fields(
                "daemon.provider",
                "settle provider prompt found no active prompt",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                    "agent_id": agent_id,
                    "prompt_completed": prompt_completed,
                    "force": force,
                }),
            );
            if owned.clear_prompt_activity(provider_run_id) {
                self.spawn_workflow_prompt_dispatches(owned.workflow_retry_blocked_claims());
            }
            self.observe_git_after_provider_activity_if_pending(provider_run_id)
                .await;
            let _ = owned.sync_focused_provider_run_if_idle(session_id);
            let _ = owned.session_snapshot(session_id);
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: false,
                started_next_prompt: false,
            });
        };
        if active_prompt.is_external() {
            crate::logging::debug_with_fields(
                "daemon.provider",
                "settle provider prompt ignored external active prompt",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                    "agent_id": agent_id,
                    "prompt_id": active_prompt.id(),
                    "prompt_completed": prompt_completed,
                    "force": force,
                }),
            );
            if owned.clear_prompt_activity(provider_run_id) {
                self.spawn_workflow_prompt_dispatches(owned.workflow_retry_blocked_claims());
            }
            self.observe_git_after_provider_activity_if_pending(provider_run_id)
                .await;
            let _ = owned.sync_focused_provider_run_if_idle(session_id);
            let _ = owned.session_snapshot(session_id);
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: false,
                started_next_prompt: false,
            });
        }

        let completion_recorded = owned.prompt_completion_recorded(provider_run_id);
        let settlement_pending = owned.prompt_completion_settlement_pending(provider_run_id);
        let is_workflow_prompt = active_prompt.workflow_run_id().is_some();
        if is_workflow_prompt && !force && !prompt_completed && !settlement_pending {
            if completion_recorded {
                owned.note_prompt_settlement_requested(provider_run_id);
                let _ = owned.session_snapshot(session_id);
            }
            crate::logging::debug_with_fields(
                "daemon.provider",
                "workflow provider prompt settlement skipped until provider completion",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                    "agent_id": agent_id,
                    "prompt_id": active_prompt.id(),
                    "settlement_pending": settlement_pending,
                    "saw_settlement_blocking_activity": saw_settlement_blocking_activity,
                    "completion_recorded": completion_recorded,
                }),
            );
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: true,
                started_next_prompt: false,
            });
        }
        if !force && !prompt_completed && !settlement_pending && completion_recorded {
            owned.note_prompt_settlement_requested(provider_run_id);
            let _ = owned.session_snapshot(session_id);
            if saw_settlement_blocking_activity {
                crate::logging::debug_with_fields(
                    "daemon.provider",
                    "provider completion is draining final output",
                    serde_json::json!({
                        "session_id": session_id,
                        "provider_run_id": provider_run_id,
                        "agent_id": agent_id,
                        "prompt_id": active_prompt.id(),
                    }),
                );
                return Ok(crate::app::ProviderRunExitSessionSummary {
                    had_active_prompt: true,
                    started_next_prompt: false,
                });
            }
        }
        if !force && !prompt_completed && !settlement_pending && !completion_recorded {
            crate::logging::debug_with_fields(
                "daemon.provider",
                "settle provider prompt skipped until provider completion",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                    "agent_id": agent_id,
                    "active_prompt_status": active_prompt.status(),
                }),
            );
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: true,
                started_next_prompt: false,
            });
        }

        if !force && !prompt_completed && completion_recorded && saw_settlement_blocking_activity {
            owned.note_prompt_settlement_requested(provider_run_id);
            let _ = owned.session_snapshot(session_id);
            crate::logging::debug_with_fields(
                "daemon.provider",
                "provider completion is draining final output",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                    "agent_id": agent_id,
                    "prompt_id": active_prompt.id(),
                }),
            );
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: true,
                started_next_prompt: false,
            });
        }

        if active_prompt.status() == crate::session::PromptStatus::Cancelling {
            if !force && completion_recorded && saw_settlement_blocking_activity {
                owned.note_prompt_settlement_requested(provider_run_id);
                let _ = owned.session_snapshot(session_id);
                return Ok(crate::app::ProviderRunExitSessionSummary {
                    had_active_prompt: true,
                    started_next_prompt: false,
                });
            }
            let cancellation = owned.finalize_local_prompt_cancellation_with_queued_advance(
                session_id,
                &agent_id,
                Some(provider_run_id),
            )?;
            owned.workflow_cancel_prompt(session_id, &cancellation.cancellation.prompt)?;
            if cancellation.released_claim {
                self.spawn_workflow_prompt_dispatches(owned.workflow_retry_blocked_claims());
            }
            if let Some(dispatch) = cancellation.dispatch {
                if let Err(error) = self
                    .enqueue_prompt_dispatch_after_liveness(&dispatch, owned)
                    .await
                {
                    let _ = self.fail_prompt_dispatch(dispatch, error).await;
                }
            }
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: true,
                started_next_prompt: cancellation.cancellation.started_next.is_some(),
            });
        }

        let provider_run_state = provider_run.state();
        let next_queued_prompt = if provider_run_state == crate::provider::ProviderRunState::Running
        {
            owned
                .prompt_state_owner
                .peek_next_queued_prompt(&owned.session_store.get_session(session_id)?, &agent_id)
        } else {
            None
        };
        let completion = if let Some(next_queued_prompt) = next_queued_prompt.as_ref() {
            owned.complete_local_prompt_with_queued_advance(
                session_id,
                &agent_id,
                Some(provider_run_id),
                next_queued_prompt,
            )?
        } else {
            owned.complete_local_prompt_without_advance(
                session_id,
                &agent_id,
                Some(provider_run_id),
            )?
        }
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "settle provider prompt",
            message: "owned prompt runtime could not settle provider prompt".to_string(),
        })?;
        self.observe_git_after_prompt_completion(provider_run_id, &completion.completion.completed)
            .await;
        crate::logging::debug_with_fields(
            "daemon.provider",
            "settled provider prompt",
            serde_json::json!({
                "session_id": session_id,
                "provider_run_id": provider_run_id,
                "agent_id": agent_id,
                "prompt_completed": prompt_completed,
                "force": force,
                "started_next": completion.completion.started_next.is_some(),
                "released_claim": completion.released_claim,
            }),
        );
        if completion.completion.completed.workflow_run_id().is_some() {
            let mut dispatches = owned.workflow_complete_prompt(
                session_id,
                &completion.completion.completed,
                Some(provider_run_id),
            )?;
            if completion.released_claim {
                dispatches.extend(owned.workflow_retry_blocked_claims());
            }
            self.spawn_workflow_prompt_dispatches(dispatches);
        }
        if let Some(started_next) = completion.completion.started_next.as_ref() {
            if crate::scheduler::runtime::is_workflow_prompt_attachment(
                started_next.source_attachment_id(),
            ) {
                owned.workflow_start_prompt(session_id, started_next)?;
            }
        }
        if completion.released_claim && completion.completion.completed.workflow_run_id().is_none()
        {
            self.spawn_workflow_prompt_dispatches(owned.workflow_retry_blocked_claims());
        }
        self.inject_metaagent_turn_completion_event(session_id, &agent_id, &completion.completion)?;
        if prompt_completed || force {
            self.inject_orphaned_metaagent_task_event_after_turn(
                session_id,
                &agent_id,
                &completion.completion,
            )?;
        }
        if let Some(dispatch) = completion.dispatch {
            if let Err(error) = self
                .enqueue_prompt_dispatch_after_liveness(&dispatch, owned)
                .await
            {
                let _ = self.fail_prompt_dispatch(dispatch, error).await;
            }
        }
        let state = self.clone();
        let session_id_for_continuation = session_id.to_string();
        let agent_id_for_continuation = agent_id.clone();
        let continuation: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> =
            Box::pin(async move {
                if let Err(error) = state
                    .run_pending_mcp_continuation_after_completion(
                        &session_id_for_continuation,
                        &agent_id_for_continuation,
                    )
                    .await
                {
                    crate::logging::warn_with_fields(
                        "daemon.provider",
                        "pending MCP continuation failed",
                        serde_json::json!({
                            "session_id": session_id_for_continuation,
                            "agent_id": agent_id_for_continuation,
                            "error": error.to_string(),
                        }),
                    );
                }
            });
        tokio::spawn(continuation);
        Ok(crate::app::ProviderRunExitSessionSummary {
            had_active_prompt: true,
            started_next_prompt: completion.completion.started_next.is_some(),
        })
    }
}
