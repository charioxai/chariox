//! One owner decision before an agent can start charged work.
use super::*;
use crate::local::{ControlPlaneRequest, MachineAllocationCapability};
use crate::runtime::command::KernelCommand;
use crate::session::{RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionChoiceStyle};
use sha2::Digest;

impl KernelRuntimeState {
    pub(crate) async fn approve_machine_acquisition(
        &self,
        command: &KernelCommand,
        request: &ControlPlaneRequest,
        capability: Option<&MachineAllocationCapability>,
    ) -> Result<(), DaemonError> {
        if !matches!(
            request,
            ControlPlaneRequest::Acquire { .. } | ControlPlaneRequest::AcquireJob { .. }
        ) {
            return Ok(());
        }
        let agent_request = command.external_grant_id().is_some()
            || command.reach_caller_id().is_some()
            || command.caller.metaagent_id.is_some()
            || command.caller.connection_class
                == Some(crate::local::KernelConnectionClass::KernelAgent);
        if !agent_request
            || self
                .owned
                .config_projection
                .snapshot()
                .user_config
                .agents
                .offload_without_asking
                == Some(true)
        {
            return Ok(());
        }
        let reach = command
            .reach_caller_id()
            .and_then(|id| self.reach_for_id(&id));
        let agent = reach
            .as_ref()
            .map(|r| r.agent_id.as_str())
            .or(command.agent_id.as_deref())
            .or(command.caller.metaagent_id.as_deref());
        let room = agent.and_then(|agent| self.owned.agent_store.get_agent(agent).ok()).map(|a| a.session_id().to_owned())
            .or_else(|| command.session_id.clone())
            .ok_or_else(|| acquisition_error("approval_required", "An agent allocation needs an active Room for the owner's approval; request it from your Room"))?;
        // The owner authorized this agent's task with /sudo; its remote work needs no second decision.
        if let Some(agent) = agent {
            if self.sudo_task_authorizes_agent(&room, agent).await {
                return Ok(());
            }
        }
        let session = self.owned.session_store.get_session(&room)?;
        let request_id = match request {
            ControlPlaneRequest::Acquire { request_id, .. }
            | ControlPlaneRequest::AcquireJob { request_id, .. } => request_id,
            _ => unreachable!(),
        };
        let digest = crate::runtime::command::KernelCommand::from_local_request(
            request_id,
            None,
            None,
            &LocalDaemonRequest::ControlPlane(request.clone()),
        )
        .durable_request_fingerprint();
        let id = format!(
            "machine-acquisition-{:x}",
            sha2::Sha256::digest(format!("{digest}:{}", agent.unwrap_or_default()).as_bytes())
        );
        if self
            .owned
            .durable_state_store
            .load_subject_events_by_kind(&id, "compute.acquisition_approved", 1)?
            .iter()
            .any(|event| event.payload["owner"] == session.owner_user_id())
        {
            return Ok(());
        }
        let description = match (request, capability) {
            (
                ControlPlaneRequest::AcquireJob {
                    needs, timeout_s, ..
                },
                Some(capability),
            ) => {
                let class = super::job_compute::select_job_class(capability, needs)?;
                let minutes = timeout_s.saturating_add(59) / 60;
                format!("Run this job on a VM ({}, {} CPUs, {} MiB) at {} {} per minute? The command limit is {minutes} minutes; charges continue through startup and until deletion.", class.class_id, class.vcpu, class.usable_memory_mib, money(class.rate_per_minute.amount_micros), class.rate_per_minute.currency)
            }
            (_, Some(capability)) => capability
                .offer
                .as_ref()
                .map(|offer| {
                    if let Some(rate) = &offer.rate_per_minute {
                        return format!(
                            "Acquire {} ({} CPUs, {} GB) at {} {} per minute? The total depends on how long you keep it.",
                            offer.machine_type, offer.vcpu, offer.ram_gb,
                            money(rate.amount_micros), rate.currency
                        );
                    }
                    format!(
                        "Acquire {} ({} CPUs, {} GB) for {} {} per {:?}?",
                        offer.machine_type,
                        offer.vcpu,
                        offer.ram_gb,
                        offer.terms.amount,
                        offer.terms.currency,
                        offer.terms.period
                    )
                })
                .unwrap_or_else(|| {
                    "Acquire a machine at the connected service's quoted rate?".into()
                }),
            _ => {
                return Err(acquisition_error(
                    "capacity_unavailable",
                    "The connected service has no machine offer; nothing was allocated",
                ))
            }
        };
        let mut interaction = RuntimeInteraction::for_kernel_operation(
            &id,
            &id,
            "",
            description,
            vec![
                RuntimeInteractionChoice::new(
                    "approve",
                    "Approve",
                    "approve",
                    Some(RuntimeInteractionChoiceStyle::Primary),
                ),
                RuntimeInteractionChoice::new(
                    "deny",
                    "Deny",
                    "deny",
                    Some(RuntimeInteractionChoiceStyle::Danger),
                ),
            ],
        );
        if let Some(agent) = agent {
            interaction = interaction.with_related_agent_id(agent.into());
        }
        let receiver = self
            .create_kernel_operation_interaction(&room, session.owner_user_id(), interaction)
            .await?;
        let mut pending = PendingAcquisition(Some((self.clone(), room.clone(), id.clone())));
        let answer = match tokio::time::timeout(Duration::from_secs(300), receiver).await {
            Ok(Ok(answer)) => answer,
            _ => {
                self.timeout_runtime_interaction(&room, &id).await?;
                return Err(acquisition_error(
                    "acquisition_denied",
                    "The owner did not approve this request; nothing was allocated or charged",
                ));
            }
        };
        pending.0 = None;
        if answer.status != "answered" || answer.choice_id.as_deref() != Some("approve") {
            return Err(acquisition_error(
                "acquisition_denied",
                "The owner denied this request; nothing was allocated or charged",
            ));
        }
        self.owned.durable_state_store.append_event("compute.acquisition_approved", Some(id), serde_json::json!({"owner":session.owner_user_id(),"approved_at_ms":crate::session::unix_epoch_ms()}))?;
        Ok(())
    }
}
/// Dropping a waiting job withdraws its decision before an agent can leave a stale stripe.
struct PendingAcquisition(Option<(KernelRuntimeState, String, String)>);
impl Drop for PendingAcquisition {
    fn drop(&mut self) {
        if let Some((state, room, id)) = self.0.take() {
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = state.timeout_runtime_interaction(&room, &id).await;
                });
            }
        }
    }
}
fn money(amount_micros: u64) -> String {
    format!("{:.6}", amount_micros as f64 / 1_000_000.0)
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}
pub(super) fn acquisition_error(code: &str, message: &str) -> DaemonError {
    DaemonError::MachineRequest {
        code: code.into(),
        message: message.into(),
    }
}

pub(crate) fn machine_tool_specs() -> Vec<crate::transport::runtime_tools::RuntimeToolSpec> {
    use crate::transport::runtime_tools::RuntimeToolSpec;
    let schema = |properties, required| serde_json::json!({"type":"object","properties":properties,"required":required,"additionalProperties":false});
    [
        ("chariox.machines.acquire", "Acquire a machine through Chariox for work you need to run elsewhere. The kernel asks the person first unless they enabled automatic offload and allocation. Service limits and the daily spend cap always apply. Returns allocation_ref, the machine type and service terms; poll status until access is available, then reach the returned kernel with Room commands. Release it when finished. Reuse request_id for a retry.", schema(serde_json::json!({"request_id":{"type":"string"}}),serde_json::json!([]))),
        ("chariox.machines.status", "Read a allocation's state and its verified target kernel. Use the allocation_ref returned by acquire. The machine remains charged until deletion is confirmed.", schema(serde_json::json!({"allocation_ref":{"type":"string"}}),serde_json::json!(["allocation_ref"]))),
        ("chariox.machines.release", "Delete an acquired machine when its work is finished. Returns the allocation state; poll status until deleted to confirm the allocation has ended.", schema(serde_json::json!({"allocation_ref":{"type":"string"}}),serde_json::json!(["allocation_ref"]))),
    ].into_iter().map(|(name,description,input_schema)|RuntimeToolSpec{name:name.into(),description:description.into(),input_schema}).collect()
}
impl KernelRuntimeState {
    pub(crate) async fn machine_tool(
        &self,
        token: &str,
        tool: &str,
        args: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            request_id: Option<String>,
            allocation_ref: Option<String>,
        }
        let args: Args = serde_json::from_value(args).map_err(|_| {
            acquisition_error(
                "invalid_request",
                "Use request_id for acquire or allocation_ref for status and release",
            )
        })?;
        let runs = self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(token);
        let [run] = runs.as_slice() else {
            return Err(acquisition_error(
                "not_allowed",
                "Machine tools need one active agent",
            ));
        };
        let reach = self
            .reach_for_id(&format!("{}{}", super::reach::REACH_PREFIX, run.id()))
            .ok_or_else(|| {
                acquisition_error(
                    "not_allowed",
                    "Only a running agent of the kernel owner may acquire machines",
                )
            })?;
        let origin = reach.origin();
        let caps = self
            .job_control_plane(
                &origin,
                &ControlPlaneRequest::Capabilities {},
                "machines:capabilities",
            )
            .await?;
        let crate::local::ControlPlaneResponse::Capabilities {
            machine_allocation: Some(capability),
        } = caps
        else {
            return Err(acquisition_error(
                "capacity_unavailable",
                "This kernel has no connected machine service",
            ));
        };
        let result = match tool {
            "chariox.machines.acquire" => {
                let request_id = args
                    .request_id
                    .unwrap_or_else(|| format!("machine-{:032x}", rand::random::<u128>()));
                let result = self
                    .job_control_plane(
                        &origin,
                        &ControlPlaneRequest::Acquire {
                            capability_revision: capability.revision.clone(),
                            request_id: request_id.clone(),
                        },
                        &format!("machine:{request_id}:acquire"),
                    )
                    .await?;
                serde_json::json!({"allocation":result,"offer":capability.offer,"estimate":{"kind":"unknown","reason":"No duration was specified; the total depends on when the machine is deleted"}})
            }
            "chariox.machines.status" => {
                let allocation_ref = args.allocation_ref.ok_or_else(|| {
                    acquisition_error("invalid_request", "allocation_ref is required")
                })?;
                serde_json::to_value(
                    self.job_control_plane(
                        &origin,
                        &ControlPlaneRequest::Get { allocation_ref },
                        "machines:status",
                    )
                    .await?,
                )
                .map_err(|e| acquisition_error("invalid_response", &e.to_string()))?
            }
            "chariox.machines.release" => {
                let allocation_ref = args.allocation_ref.ok_or_else(|| {
                    acquisition_error("invalid_request", "allocation_ref is required")
                })?;
                let current = self
                    .job_control_plane(
                        &origin,
                        &ControlPlaneRequest::Get {
                            allocation_ref: allocation_ref.clone(),
                        },
                        "machines:release:get",
                    )
                    .await?;
                let crate::local::ControlPlaneResponse::Machine { revision, .. } = current else {
                    return Err(acquisition_error(
                        "invalid_response",
                        "The service returned another allocation",
                    ));
                };
                serde_json::to_value(
                    self.job_control_plane(
                        &origin,
                        &ControlPlaneRequest::Release {
                            allocation_ref: allocation_ref.clone(),
                            capability_revision: capability.revision,
                            request_id: format!("release:{allocation_ref}"),
                            expected_revision: revision,
                        },
                        "machines:release",
                    )
                    .await?,
                )
                .map_err(|e| acquisition_error("invalid_response", &e.to_string()))?
            }
            _ => {
                return Err(acquisition_error(
                    "invalid_request",
                    "Unknown machine action",
                ))
            }
        };
        Ok(crate::transport::runtime_tools::RuntimeToolResult {
            ok: true,
            payload: result,
        })
    }
}

#[cfg(test)]
mod tests;
