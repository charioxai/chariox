//! MP-08 / MP-11: settle legacy remote Meta execution before ordinary follow-ups.
use super::*;
use crate::app::RemoteMetaRetirementIntent as Intent;
use crate::session::{DurablePromptDeliveryPhase, PromptQueueItem};
use crate::transport::relay_peer::LeasedPromptReceiptPhase;

impl KernelRuntimeState {
    pub(super) async fn recover_remote_meta_retirement(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt: &PromptQueueItem,
    ) -> Result<bool, DaemonError> {
        let intent = Intent::read(prompt)?;
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        let binding = agent
            .remote_execution()
            .cloned()
            .filter(|binding| intent.matches(binding) && binding.relay_peer_protocol_compatible())
            .ok_or_else(|| Intent::error("retirement worker binding is missing or changed"))?;
        let Some(_claim) = self.try_claim_remote_prompt_cancellation_send(session_id, agent_id)
        else {
            return Ok(false);
        };
        let config = self
            .with_app_side_effect(|app| app.relay_config_for_remote_execution(&binding))
            .await;
        if prompt.durable_delivery_phase() != Some(DurablePromptDeliveryPhase::Accepted) {
            // Dispatching/legacy prompts have no reliable ACK. A read-only receipt
            // must establish the exact run and lease; never replay or cancel by guess.
            let response = self
                .send_remote_meta_retirement_request(
                    &config,
                    &binding,
                    RelayPeerRequest::GetLeasedPromptReceipt {
                        leased_agent_id: binding.leased_agent_id.clone(),
                        home_prompt_id: prompt.id().into(),
                    },
                )
                .await?;
            let RelayPeerResponse::LeasedPromptReceiptQueried {
                receipt: Some(receipt),
            } = response
            else {
                return Err(Intent::error(
                    "worker has not proved the retired prompt receipt",
                ));
            };
            if receipt.home_prompt_id != prompt.id()
                || receipt.execution_lease_id.as_deref()
                    != Some(binding.execution_lease_id.as_str())
                || prompt
                    .durable_delivery_provider_run_id()
                    .is_some_and(|id| id != receipt.worker_provider_run_id)
                || binding
                    .active_worker_provider_run_id
                    .as_deref()
                    .is_some_and(|id| id != receipt.worker_provider_run_id)
            {
                return Err(Intent::error(
                    "worker receipt does not match the retirement identity",
                ));
            }
            match receipt.phase {
                LeasedPromptReceiptPhase::Active | LeasedPromptReceiptPhase::Completed => {
                    if receipt.worker_provider_run_id.is_empty() {
                        return Err(Intent::error("worker receipt is missing its provider run"));
                    }
                    if receipt.phase == LeasedPromptReceiptPhase::Active {
                        let response = self
                            .send_remote_meta_retirement_request(
                                &config,
                                &binding,
                                RelayPeerRequest::CancelLeasedPrompt {
                                    leased_agent_id: binding.leased_agent_id.clone(),
                                    home_prompt_id: prompt.id().into(),
                                    worker_provider_run_id: receipt.worker_provider_run_id.clone(),
                                },
                            )
                            .await?;
                        if !matches!(response, RelayPeerResponse::LeasedPromptCancelled { .. }) {
                            return Err(Intent::error(
                                "worker did not acknowledge exact-turn cancellation",
                            ));
                        }
                    }
                    // Pump the existing leased projection path so cancellation can
                    // settle on the worker. Do not complete/promote the home queue yet.
                    let response = self
                        .send_remote_meta_retirement_request(
                            &config,
                            &binding,
                            RelayPeerRequest::DrainLeasedRuntimeProjection {
                                leased_agent_id: binding.leased_agent_id.clone(),
                                provider_run_id: receipt.worker_provider_run_id,
                                pump_output: true,
                            },
                        )
                        .await?;
                    if !matches!(
                        response,
                        RelayPeerResponse::LeasedRuntimeProjectionDrained { .. }
                    ) {
                        return Err(Intent::error(
                            "worker did not acknowledge retirement projection drain",
                        ));
                    }
                }
                LeasedPromptReceiptPhase::SteerRejected
                    if receipt.target_home_prompt_id.is_none() => {}
                _ => {
                    return Err(Intent::error(
                        "worker retirement receipt is still uncertain",
                    ))
                }
            }
        }
        // This existing worker operation refuses an active backing prompt. Its
        // exact-lease ACK proves both turn settlement and removal of Meta policy.
        let response = self
            .send_remote_meta_retirement_request(
                &config,
                &binding,
                RelayPeerRequest::UpdateLeasedAgentMetaMode {
                    leased_agent_id: binding.leased_agent_id.clone(),
                    active: false,
                },
            )
            .await?;
        let RelayPeerResponse::LeasedAgentMetaModeUpdated { leased_agent } = response else {
            return Err(Intent::error(
                "worker has not acknowledged settled Meta-mode retirement",
            ));
        };
        if leased_agent.id != binding.leased_agent_id
            || leased_agent.lease_id != binding.execution_lease_id
        {
            return Err(Intent::error(
                "worker Meta-mode acknowledgement belongs to another lease",
            ));
        }
        let committed = self
            .owned
            .commit_remote_meta_retirement(session_id, agent_id, prompt, &binding)?;
        // Release before dispatch: a successor must own its own claim rather
        // than enqueue a wakeup on the retirement claim that is about to drop.
        drop(_claim);
        if committed {
            self.spawn_next_queued_remote_prompt(session_id, agent_id)
                .await?;
        }
        Ok(committed)
    }

    async fn send_remote_meta_retirement_request(
        &self,
        config: &crate::config::DaemonConfig,
        binding: &crate::agent::RemoteAgentBinding,
        request: RelayPeerRequest,
    ) -> Result<RelayPeerResponse, DaemonError> {
        let target = ClientTarget {
            daemon_id: Some(binding.worker_kernel_id.clone()),
            daemon_alias: None,
        };
        let timeout = Duration::from_secs(5);
        if let Some(relay) = self.connected_relay_state_for_config(config).await {
            crate::transport::relay_client::send_peer_request_via_connected_relay_with_timeout(
                config, &relay, target, request, timeout,
            )
            .await
        } else {
            crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
                config, target, request, timeout,
            )
            .await
        }
    }
}

impl KernelRuntimeOwnedState {
    pub(super) fn ensure_remote_meta_retirement_not_pending(
        &self,
        session: &crate::session::RuntimeSession,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        if self
            .prompt_state_owner
            .active_prompt_for_agent(session, agent_id)
            .as_ref()
            .is_some_and(Intent::is_pending)
        {
            return Err(Intent::error(
                "retired Meta worker must settle before the home prompt is discarded",
            ));
        }
        Ok(())
    }

    fn commit_remote_meta_retirement(
        &self,
        session_id: &str,
        agent_id: &str,
        expected: &PromptQueueItem,
        binding: &crate::agent::RemoteAgentBinding,
    ) -> Result<bool, DaemonError> {
        let activity = self.begin_managed_activity_mutation();
        let mut sessions = self.session_store.write();
        let session = sessions.get_session(session_id)?;
        let cancelled = self
            .prompt_state_owner
            .finalize_active_prompt_cancellation_if_matches(
                &session,
                agent_id,
                expected,
                |queue| {
                    // Same lock order as ordinary remote dispatch settlement:
                    // activity -> sessions -> prompt owner -> agents -> durable write.
                    let mut agents = self.agent_store.write();
                    let mut agent = agents.get_agent(agent_id)?;
                    if agent.remote_execution() != Some(binding) {
                        return Err(Intent::error("worker binding changed during retirement"));
                    }
                    agent.set_remote_execution_active_worker_provider_run_id(None);
                    agent.set_processing(false);
                    let mut retired = session.clone();
                    retired.mirror_agent_prompt_state(agent_id, None, queue);
                    retired.set_agents(
                        agents
                            .get_session_agents(session_id)
                            .into_iter()
                            .map(|candidate| {
                                if candidate.id() == agent_id {
                                    agent.clone()
                                } else {
                                    candidate
                                }
                            })
                            .collect(),
                    );
                    // Persist first. A failed write leaves the owner and agent intact;
                    // retries may idempotently confirm the worker's already-cleared mode.
                    // Agent cleanup is monotonic and may survive a failed prompt
                    // write. The exact ACK remains in the still-held private prompt.
                    self.durable_state_store.append_event(
                        "agent.updated",
                        Some(agent_id.into()),
                        serde_json::json!({ "agent": &agent, "reason": "meta_remote_retired", "retired_prompt_id": expected.id() }),
                    )?;
                    self.persist_prompt_session_state(&retired, agent_id)?;
                    agents.restore_agent(agent);
                    sessions.restore_session(retired);
                    Ok(true)
                },
            )?;
        drop(sessions);
        if let Some(cancelled) = cancelled.as_ref() {
            self.record_cancelled_prompt_settlement(
                session_id,
                agent_id,
                cancelled,
                expected.durable_delivery_provider_run_id(),
            );
            activity.record();
            self.session_snapshot(session_id)?;
        }
        Ok(cancelled.is_some())
    }
}
