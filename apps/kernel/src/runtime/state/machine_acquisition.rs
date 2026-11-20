//! Machine options and the agent's allocation workflow.
use super::*;
use crate::durable_state::acquire_choices::AcquireOption;
use crate::local::{ControlPlaneRequest, MachineAllocationCapability};
use approval::{decision_error, direct_approval_id, MachineDecision};

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
        ("chariox.machines.options", "List the machines you can acquire now, as the connected service offers them: class_id, region, size, decimal rate per minute and per hour in the account's currency, and whether it is in stock. Pick one for chariox.machines.acquire. An option that is not available cannot be acquired.", schema(serde_json::json!({}),serde_json::json!([]))),
        ("chariox.machines.acquire", "Acquire a machine through Chariox for work you need to run elsewhere. Pass class_id (and region when the class is offered in several) from chariox.machines.options to acquire exactly that option; without class_id the first option in stock is acquired. A named option that is out of stock is refused with the options in stock, never replaced. When the person needs to approve, returns status waiting, the request_id and registration_id. Yield on the registration_id. After approval, call acquire with that same request_id to send the request. After denial or expiry, ask again with a new request_id. Service limits and the daily spend cap always apply. Returns allocation_ref, the machine type and service terms; poll status until access is available, then reach the returned kernel with Room commands. Release it when finished. Reuse request_id for a retry, after a lost response, a timeout or a refusal: it is safe, keeps the option the first try chose and returns the allocation the service accepted. Use a new request_id when the approval ended or the service closed the old one.", schema(serde_json::json!({"request_id":{"type":"string"},"class_id":{"type":"string"},"region":{"type":"string"}}),serde_json::json!([]))),
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
            class_id: Option<String>,
            region: Option<String>,
        }
        let args: Args = serde_json::from_value(args).map_err(|_| {
            acquisition_error(
                "invalid_request",
                "Use request_id, class_id and region for acquire, allocation_ref for status and release, and no arguments for options",
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
        if let ("chariox.machines.acquire", Some(request_id)) = (tool, args.request_id.as_deref()) {
            let session = self.owned.session_store.get_session(run.session_id())?;
            let id = direct_approval_id(
                session.id(),
                &reach.agent_id,
                &choice::service_request_id(
                    &self.owned.config_projection.snapshot().daemon_id,
                    &reach.agent_id,
                    request_id,
                ),
            );
            if let Some(
                decision @ (MachineDecision::Denied { .. } | MachineDecision::Expired { .. }),
            ) = self.owned.machine_decision(&id, session.owner_user_id())?
            {
                return Err(decision_error(&decision));
            }
        }
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
            "chariox.machines.options" => quote::machine_options(&capability),
            "chariox.machines.acquire" => {
                let request_id = args
                    .request_id
                    .clone()
                    .unwrap_or_else(|| format!("machine-{:032x}", rand::random::<u128>()));
                let option = self.acquire_option(
                    &reach.agent_id,
                    &request_id,
                    &capability,
                    args.class_id.clone(),
                    args.region.clone(),
                )?;
                let service_request_id = choice::service_request_id(
                    &self.owned.config_projection.snapshot().daemon_id,
                    &reach.agent_id,
                    &request_id,
                );
                let result = match self
                    .job_control_plane(
                        &origin,
                        &ControlPlaneRequest::Acquire {
                            capability_revision: capability.revision.clone(),
                            request_id: service_request_id.clone(),
                            class_id: option.class_id().map(str::to_owned),
                            region: option.region().map(str::to_owned),
                        },
                        &format!("machine:{service_request_id}:acquire"),
                    )
                    .await
                {
                    Ok(crate::local::ControlPlaneResponse::AcquisitionWaiting {
                        registration_id,
                        expires_at_ms,
                    }) => {
                        return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                            ok: true,
                            payload: serde_json::json!({
                                "status":"waiting",
                                "message":"No answer yet. Yield on registration_id for the decision. If approved, use this request_id to acquire the machine.",
                                "request_id":request_id,
                                "registration_id":registration_id,
                                "expires_at_ms":expires_at_ms,
                            }),
                        })
                    }
                    Ok(result) => result,
                    Err(error) => {
                        self.close_acquire_option(&reach.agent_id, &request_id, &option, &error)?;
                        return Err(choice::acquire_refusal(
                            error,
                            &capability,
                            &option,
                            &request_id,
                        ));
                    }
                };
                let crate::local::ControlPlaneResponse::Machine { allocation_ref, .. } = &result
                else {
                    return Err(acquisition_error(
                        "invalid_response",
                        "The service returned no allocation",
                    ));
                };
                self.accept_acquire_option(&reach.agent_id, &request_id, &option, allocation_ref)?;
                // The service decides whether the choice can be acquired, and replays a retry of one it accepted.
                match option {
                    AcquireOption::Named { class_id, region } => {
                        let option = capability.offers.iter().find(|option| {
                            option.class_id == class_id
                                && region
                                    .as_ref()
                                    .is_none_or(|region| &option.region == region)
                        });
                        serde_json::json!({"allocation":result,"option":option,"estimate":{"kind":"unknown","reason":"No duration was specified; the total depends on when the machine is deleted"}})
                    }
                    AcquireOption::ServiceDefault => {
                        serde_json::json!({"allocation":result,"offer":capability.offer,"estimate":{"kind":"unknown","reason":"No duration was specified; the total depends on when the machine is deleted"}})
                    }
                }
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

pub(super) mod approval;
mod choice;
mod quote;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tool_tests;
