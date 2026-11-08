//! MP-08 / MP-09 / MP-10 / MP-11 reach: the `chariox-reach` tools of a kernel owner's
//! Room agent. Each call resolves the agent's own live run again, so ending the run
//! ends the reach. Another kernel is asked through the owner's gateway, naming the agent.
use super::CommandRouter;
use crate::error::DaemonError;
use crate::local::LocalDaemonRequest;
use crate::runtime::kernel_gateway::{forward_with_retry, Origin};
use crate::runtime::state::is_outside_event_request;
use crate::runtime::state::reach::{acquires_machine, reach_need, ReachCaller, ReachNeed};
use crate::transport::runtime_tools::{
    reach_events_ack_request, reach_events_subscribe_request, reach_events_wait_request,
    reach_guide, reach_sessions_request, ReachEventsAckArgs, ReachEventsSubscribeArgs,
    ReachEventsWaitArgs, ReachRequestArgs, ReachSessionsArgs, RuntimeToolResult,
    REACH_EVENTS_ACK_TOOL, REACH_EVENTS_SUBSCRIBE_TOOL, REACH_EVENTS_WAIT_TOOL, REACH_GUIDE_TOOL,
    REACH_KERNELS_TOOL, REACH_REQUEST_TOOL, REACH_SESSIONS_TOOL,
};

/// A refused call is a tool result with a stable code, not a transport failure.
fn refused(code: &str, message: impl Into<String>) -> RuntimeToolResult {
    RuntimeToolResult {
        ok: false,
        payload: serde_json::json!({"code": code, "message": message.into()}),
    }
}

fn tool_arguments<T: serde::de::DeserializeOwned>(
    tool_name: &str,
    arguments: serde_json::Value,
) -> Result<T, DaemonError> {
    serde_json::from_value(arguments).map_err(|error| {
        crate::runtime::kernel_access::error(format!("invalid {tool_name} tool arguments: {error}"))
    })
}

fn kernel_request(request: serde_json::Value) -> Result<LocalDaemonRequest, DaemonError> {
    serde_json::from_value(request).map_err(|error| {
        crate::runtime::kernel_access::error(format!("invalid kernel request: {error}; use externally tagged JSON in request, for example {{\"ListSessions\":null}}"))
    })
}

/// A reach caller is an agent, and an agent answers a decision only as its holder, through
/// `agent answer`; no room, kernel or sudo window changes that.
fn answers_a_decision(request: &LocalDaemonRequest) -> bool {
    matches!(request, LocalDaemonRequest::RespondToInteraction(_))
}

impl CommandRouter {
    pub(crate) fn reach_tool_specs_for_auth_token(
        &self,
        auth_token: &str,
    ) -> Vec<crate::transport::runtime_tools::RuntimeToolSpec> {
        match self.runtime_state.reach_listing_for_auth_token(auth_token) {
            Ok(_) => crate::transport::runtime_tools::reach_tool_specs(),
            Err(_) => Vec::new(),
        }
    }

    pub(crate) async fn dispatch_reach_tool_call(
        &self,
        auth_token: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let started = std::time::Instant::now();
        let result = self
            .dispatch_reach_tool(auth_token, tool_name, arguments)
            .await;
        crate::logging::info_with_fields(
            "agent.reach",
            "reach tool call finished",
            serde_json::json!({
                "tool": tool_name,
                "ok": result.as_ref().is_ok_and(|result| result.ok),
                "elapsed_ms": u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            }),
        );
        result
    }

    async fn dispatch_reach_tool(
        &self,
        auth_token: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let caller = match self.runtime_state.reach_for_auth_token(auth_token) {
            Ok(caller) => caller,
            Err(error) => return Ok(refused("reach_not_available", error.to_string())),
        };
        match tool_name {
            REACH_KERNELS_TOOL => Ok(self.reach_kernels().await),
            REACH_GUIDE_TOOL => Ok(RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({"guide": reach_guide()}),
            }),
            REACH_SESSIONS_TOOL => {
                let args: ReachSessionsArgs = tool_arguments(tool_name, arguments)?;
                let request = reach_sessions_request(&args);
                self.reach_request(&caller, args.kernel, kernel_request(request)?)
                    .await
            }
            REACH_REQUEST_TOOL => {
                let args: ReachRequestArgs = tool_arguments(tool_name, arguments)?;
                self.reach_request(&caller, args.kernel, kernel_request(args.request)?)
                    .await
            }
            // The inbox of events lives on the agent's own kernel, whichever kernel is followed.
            REACH_EVENTS_SUBSCRIBE_TOOL => {
                let args: ReachEventsSubscribeArgs = tool_arguments(tool_name, arguments)?;
                let request = reach_events_subscribe_request(args);
                self.reach_request(&caller, None, kernel_request(request)?)
                    .await
            }
            REACH_EVENTS_WAIT_TOOL => {
                let args: ReachEventsWaitArgs = tool_arguments(tool_name, arguments)?;
                let request = reach_events_wait_request(args);
                self.reach_request(&caller, None, kernel_request(request)?)
                    .await
            }
            REACH_EVENTS_ACK_TOOL => {
                let args: ReachEventsAckArgs = tool_arguments(tool_name, arguments)?;
                let request = reach_events_ack_request(args);
                self.reach_request(&caller, None, kernel_request(request)?)
                    .await
            }
            _ => Ok(refused("reach_unknown_tool", "no such chariox-reach tool")),
        }
    }

    async fn reach_kernels(&self) -> RuntimeToolResult {
        match self.kernel_gateway().kernels().await {
            Ok(kernels) => RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({"kernels": kernels}),
            },
            Err(failure) => refused(failure.code(), failure.transport_error().message),
        }
    }

    async fn reach_request(
        &self,
        caller: &ReachCaller,
        kernel: Option<String>,
        request: LocalDaemonRequest,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let state = &self.runtime_state;
        // Without a /sudo window of this agent's own turn, only what a Room agent
        // may do in its own room; the window adds what /sudo adds at home.
        let sudo =
            match reach_need(&request) {
                ReachNeed::Room if acquires_machine(&request) => {
                    state.reach_sudo_turn(caller).filter(|turn| {
                        state
                            .authorize_sudo_request(&turn.entry_id, &request)
                            .is_ok()
                    })
                }
                ReachNeed::Room => None,
                ReachNeed::HumanOnly => {
                    return Ok(refused(
                        "human_only",
                        "this request belongs to the owner and no agent may send it",
                    ))
                }
                ReachNeed::Excluded => {
                    return Ok(refused(
                        "excluded_for_agents",
                        "agents may not send this request, not even with /sudo",
                    ))
                }
                ReachNeed::Sudo => match state.reach_sudo_turn(caller) {
                    Some(turn) => Some(turn),
                    None => return Ok(refused(
                        "reach_requires_sudo",
                        "this needs /sudo in your own room too; ask the owner to /sudo your task",
                    )),
                },
            };
        if is_outside_event_request(&request) {
            // The inbox of events lives on the agent's own kernel, whichever kernel is followed.
            if kernel.is_some() {
                return Ok(refused(
                    "events_local_only",
                    "event requests run on your own kernel; name the kernel to follow in events_subscribe",
                ));
            }
            return self.reach_events(caller, request).await;
        }
        if answers_a_decision(&request) {
            return Ok(refused(
                "excluded_for_agents",
                "agents do not answer decisions; the person answers them in Chariox",
            ));
        }
        if acquires_machine(&request) && sudo.is_none() {
            let offer = if let LocalDaemonRequest::ControlPlane(
                crate::local::ControlPlaneRequest::Acquire {
                    capability_revision,
                    ..
                },
            ) = &request
            {
                // Resolve through the same target and owner authority as the acquisition.
                // Cloud binds this descriptor to the revision it validates again on Accept.
                let capabilities = Box::pin(self.reach_request(
                    caller,
                    kernel.clone(),
                    LocalDaemonRequest::ControlPlane(
                        crate::local::ControlPlaneRequest::Capabilities {},
                    ),
                ))
                .await?;
                if !capabilities.ok {
                    return Ok(capabilities);
                }
                let Some(capability) = serde_json::from_value::<crate::local::LocalDaemonResponse>(
                    capabilities.payload,
                )
                .ok()
                .and_then(|response| match response {
                    crate::local::LocalDaemonResponse::ControlPlane {
                        result:
                            crate::local::ControlPlaneResponse::Capabilities { machine_allocation },
                    } => machine_allocation,
                    _ => None,
                }) else {
                    return Ok(refused(
                        "acquisition_unavailable",
                        "the connected service has no machine offer",
                    ));
                };
                if capability.revision != *capability_revision
                    || !capability.actions.iter().any(|action| action == "acquire")
                    || capability.offer.is_none()
                {
                    return Ok(refused(
                        "acquisition_offer_changed",
                        "refresh machine capabilities before acquiring a machine",
                    ));
                }
                capability.offer
            } else {
                None
            };
            if !state
                .confirm_machine_acquisition(caller, offer.as_ref())
                .await?
            {
                return Ok(refused(
                    "acquisition_declined",
                    "the owner did not accept acquiring a machine",
                ));
            }
        }
        let command_id = format!("{}:{}", caller.id, rand::random::<u64>());
        let Some(kernel) = kernel else {
            return self.reach_local(caller, command_id, request, sudo).await;
        };
        let follow_origin = Origin {
            grant_id: caller.id.clone(),
            harness: caller.harness.clone(),
        };
        let origin = match &sudo {
            Some(turn) => Origin {
                grant_id: turn.entry_id.clone(),
                harness: caller.harness.clone(),
            },
            None => Origin {
                grant_id: caller.id.clone(),
                harness: caller.harness.clone(),
            },
        };
        if let Err(error) = state.authorize_gateway_request(&origin.grant_id, &request) {
            return Ok(refused("not_allowed", error.to_string()));
        }
        Ok(
            match forward_with_retry(command_id, |id| {
                let (origin, follow_origin, kernel, request) =
                    (&origin, &follow_origin, &kernel, &request);
                async move {
                    crate::runtime::kernel_gateway::forward_audited(
                        self,
                        origin,
                        follow_origin,
                        &id,
                        kernel,
                        request,
                    )
                    .await
                }
            })
            .await
            {
                Ok(response) => RuntimeToolResult {
                    ok: true,
                    payload: response,
                },
                Err(failure) => refused(failure.code(), failure.transport_error().message),
            },
        )
    }

    /// Follow, wait and acknowledge on the agent's own inbox, which is named after its run.
    async fn reach_events(
        &self,
        caller: &ReachCaller,
        request: LocalDaemonRequest,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let origin = Origin {
            grant_id: caller.id.clone(),
            harness: caller.harness.clone(),
        };
        if matches!(request, LocalDaemonRequest::SubscribeEvents(_)) {
            self.runtime_state.end_dead_reach_subscriptions()?;
        }
        match self
            .kernel_gateway()
            .event_request(&origin, &request, None)
            .await
        {
            Ok(response) => Ok(RuntimeToolResult {
                ok: true,
                payload: serde_json::to_value(response).map_err(|_| {
                    crate::runtime::kernel_access::error("kernel response serialization failed")
                })?,
            }),
            Err(DaemonError::MachineRequest { code, message }) => Ok(refused(&code, message)),
            Err(error) => Ok(refused("request_failed", error.to_string())),
        }
    }

    /// The agent's own kernel: the owner's request, named after the run, with none of
    /// the room fences a Room tool call carries.
    async fn reach_local(
        &self,
        caller: &ReachCaller,
        command_id: String,
        request: LocalDaemonRequest,
        sudo: Option<crate::local::KernelSudoTurn>,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let dispatched = match sudo {
            Some(turn) => Ok(self.dispatch_sudo_request(turn, request.clone()).await?),
            None => {
                let mut command = crate::runtime::command::KernelCommand::from_local_request(
                    command_id,
                    None,
                    Some(caller.id.clone()),
                    &request,
                );
                command.caller = crate::runtime::command::KernelCaller::for_source(
                    &crate::runtime::command::KernelCommandSource::LocalIpc,
                )
                .with_connection_class(crate::local::KernelConnectionClass::KernelAgent);
                command.provider_run_id = Some(caller.run_id.clone());
                command.caller.caller_id = caller.id.clone();
                command.caller.user_id = Some(caller.owner_user_id.clone());
                Box::pin(self.dispatch(command, request.clone())).await
            }
        };
        let mut response = match dispatched {
            Ok(response) => response,
            Err(error) => return Ok(refused("request_failed", error.to_string())),
        };
        let origin = Origin {
            grant_id: caller.id.clone(),
            harness: caller.harness.clone(),
        };
        if self
            .kernel_gateway()
            .follow_created(
                &origin,
                &self.config_projection.snapshot().daemon_id,
                &request,
                &response,
            )
            .is_err()
        {
            response.record_event_subscription_failure();
        }
        Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::to_value(response).map_err(|_| {
                crate::runtime::kernel_access::error("kernel response serialization failed")
            })?,
        })
    }
}

#[cfg(test)]
mod retry_tests {
    use super::*;
    use crate::runtime::kernel_gateway::{
        ledger::{GatewayLedger, Settled},
        GatewayError,
    };
    use crate::runtime_transport::command_cache::CommandFingerprint;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn reach_lost_response_retries_without_repeating_the_side_effect() {
        let dir = crate::test_support::TestWorktree::new("reach-retry");
        let ledger = GatewayLedger::open(dir.path().join("receipts.json"));
        let effects = AtomicU32::new(0);
        let attempts = AtomicU32::new(0);
        let request: LocalDaemonRequest = serde_json::from_value(serde_json::json!({"RoomTerminal": {
            "session_id": "s", "operation": {"operation": "exec_open", "command": ["uname", "-a"]}
        }})).unwrap();
        let command =
            crate::runtime::command::KernelCommand::from_local_request("one", None, None, &request);
        let fingerprint = CommandFingerprint::for_app_control(&command, &request);
        let answer = forward_with_retry("reach:run:one".into(), |id| {
            let (ledger, effects, attempts, fingerprint) =
                (&ledger, &effects, &attempts, fingerprint.clone());
            async move {
                let settled = ledger
                    .run(&id, fingerprint, async {
                        effects.fetch_add(1, Ordering::SeqCst);
                        Ok(serde_json::json!({"exec_id": "one"}))
                    })
                    .await;
                // The target completed the operation, but the first response was lost.
                if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    return Err(GatewayError::ConnectionLost {
                        name: "remote".into(),
                    });
                }
                match settled {
                    Settled::Done(Ok(value)) => Ok(value),
                    _ => panic!("the retry must recover the original receipt"),
                }
            }
        })
        .await
        .expect("a lost response must recover with the original command identity");
        assert_eq!(answer["exec_id"], "one");
        assert_eq!(effects.load(Ordering::SeqCst), 1);
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn reach_exhausted_retry_requires_reading_state_instead_of_replaying() {
        let attempts = AtomicU32::new(0);
        let failure = forward_with_retry("reach:one".into(), |_| {
            attempts.fetch_add(1, Ordering::SeqCst);
            async {
                Err(GatewayError::RequestTimeout {
                    name: "remote".into(),
                })
            }
        })
        .await
        .unwrap_err();
        assert_eq!(failure.code(), "kernel_outcome_unknown");
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }
}
