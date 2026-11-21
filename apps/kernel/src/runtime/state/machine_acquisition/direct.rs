//! The native agent's acquisition request and its trace across the approval wait.
use super::approval::{decision_error, direct_approval_id, MachineDecision};
use super::*;
use crate::durable_state::acquire_choices::AcquireOption;
use crate::durable_state::agent_lifecycle::AgentWake;
use crate::local::ControlPlaneResponse;
use crate::runtime::kernel_gateway::Origin;
use crate::transport::runtime_tools::RuntimeToolResult;
use serde_json::{json, Value};

const WAKE_TRACE: &str = "machine_acquisition_trace";

pub(crate) struct AcquireTrace<'a> {
    pub(crate) request_id: &'a str,
    room: &'a str,
    agent: &'a str,
    class: Option<String>,
    interaction_id: &'a str,
}

impl AcquireTrace<'_> {
    pub(crate) fn detail(&self) -> Value {
        json!({
            "request_id": crate::secret_redaction::redact_secrets(self.request_id),
            "class_id": self.class.as_deref().map(crate::secret_redaction::redact_secrets),
        })
    }

    fn record(&self, outcome: &str, code: Option<&str>, allocation: Option<&str>) {
        record(
            self.detail(),
            self.room,
            self.agent,
            self.interaction_id,
            outcome,
            code,
            allocation,
        );
    }
}

fn record(
    mut fields: Value,
    room: &str,
    agent: &str,
    interaction_id: &str,
    outcome: &str,
    code: Option<&str>,
    allocation: Option<&str>,
) {
    fields["session_id"] = json!(room);
    fields["agent_id"] = json!(agent);
    fields["interaction_id"] = json!(interaction_id);
    fields["outcome"] = json!(outcome);
    if let Some(code) = code {
        fields["code"] = json!(code);
    }
    if let Some(allocation) = allocation {
        fields["allocation_ref"] = json!(allocation);
    }
    crate::logging::info_with_fields("runtime.machine_acquisition", "machine acquire", fields);
}

pub(in crate::runtime::state) fn set_wake_trace(detail: &mut Value, trace: Value) {
    detail[WAKE_TRACE] = trace;
}

pub(in crate::runtime::state) fn record_wake(wake: &AgentWake, outcome: &str) {
    if let Some(fields) = wake
        .detail
        .get(WAKE_TRACE)
        .filter(|fields| fields.is_object())
    {
        record(
            fields.clone(),
            &wake.room_id,
            &wake.agent_id,
            &wake.id,
            outcome,
            None,
            None,
        );
    }
}

pub(crate) fn has_wake_trace(wake: &AgentWake) -> bool {
    wake.detail.get(WAKE_TRACE).is_some()
}

impl KernelRuntimeState {
    pub(super) async fn direct_machine_acquire(
        &self,
        origin: &Origin,
        room: &str,
        agent: &str,
        args: Args,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let request_id = args
            .request_id
            .as_ref()
            .cloned()
            .unwrap_or_else(|| format!("machine-{:032x}", rand::random::<u128>()));
        let service_id = choice::service_request_id(
            &self.owned.config_projection.snapshot().daemon_id,
            agent,
            &request_id,
        );
        let interaction_id = direct_approval_id(room, agent, &service_id);
        let mut trace = AcquireTrace {
            request_id: &request_id,
            room,
            agent,
            class: args.class_id.clone(),
            interaction_id: &interaction_id,
        };
        trace.record("requested", None, None);
        match self
            .acquire_machine(origin, &args, service_id, &mut trace)
            .await
        {
            Ok(payload) => {
                if payload["status"] == "pending" {
                    trace.record("pending", None, None);
                }
                Ok(RuntimeToolResult { ok: true, payload })
            }
            Err(error) => {
                let error = match error {
                    DaemonError::MachineRequest { .. } | DaemonError::RelayTransport { .. } => error,
                    _ => acquisition_error(
                        "acquisition_unavailable",
                        &format!("The kernel could not finish the acquisition. Retry with request_id {request_id}: {error}"),
                    ),
                };
                let code = match &error {
                    DaemonError::MachineRequest { code, .. }
                    | DaemonError::RelayTransport { code, .. } => Some(code.as_str()),
                    _ => None,
                };
                trace.record("refused", code, None);
                Err(error)
            }
        }
    }

    async fn acquire_machine(
        &self,
        origin: &Origin,
        args: &Args,
        service_id: String,
        trace: &mut AcquireTrace<'_>,
    ) -> Result<Value, DaemonError> {
        let reason = args
            .reason
            .as_deref()
            .map(str::trim)
            .filter(|reason| {
                !reason.is_empty()
                    && reason.chars().count() <= ACQUISITION_REASON_MAX_CHARS
                    && !reason.chars().any(char::is_control)
            })
            .ok_or_else(|| {
                acquisition_error(
            "invalid_request",
            "Give a reason in one line of up to 160 characters explaining why you need the machine",
        )
            })?;
        let session = self.owned.session_store.get_session(trace.room)?;
        let decision = self
            .owned
            .machine_decision(trace.interaction_id, session.owner_user_id())?;
        match decision {
            Some(MachineDecision::Approved { .. }) => {}
            Some(decision) => return Err(decision_error(&decision)),
            None => {
                if let Some(wake) = self.machine_approval_wake(trace.interaction_id).await? {
                    if let Some(choice) = self.owned.durable_state_store.acquire_choice(
                        trace.agent,
                        trace.request_id,
                        crate::session::unix_epoch_ms(),
                    )? {
                        choice::check_acquire_option(
                            trace.request_id,
                            &choice.option,
                            args.class_id.as_deref(),
                            args.region.as_deref(),
                        )?;
                        trace.class = choice.option.class_id().map(str::to_owned);
                    }
                    // A settlement can commit while the lookup above was reading the wake.
                    match self
                        .owned
                        .machine_decision(trace.interaction_id, session.owner_user_id())?
                    {
                        Some(MachineDecision::Approved { .. }) => {}
                        Some(decision) => return Err(decision_error(&decision)),
                        None => {
                            let waiting = self.pending_machine_acquisition(
                                &wake,
                                trace.room,
                                Some(trace.agent),
                            )?;
                            return pending_payload(trace, waiting);
                        }
                    }
                }
            }
        }

        let caps = self
            .machine_control_plane(
                origin,
                &ControlPlaneRequest::Capabilities {},
                "machines:capabilities",
                Some(trace),
            )
            .await?;
        let ControlPlaneResponse::Capabilities {
            machine_allocation: Some(capability),
        } = caps
        else {
            return Err(acquisition_error(
                "capacity_unavailable",
                "This kernel has no connected machine service. Use a kernel connected to one, or ask the owner to connect this kernel",
            ));
        };
        let option = self.acquire_option(
            trace.agent,
            trace.request_id,
            &capability,
            args.class_id.clone(),
            args.region.clone(),
        )?;
        trace.class = option.class_id().map(str::to_owned);
        let result = self
            .machine_control_plane(
                origin,
                &ControlPlaneRequest::Acquire {
                    capability_revision: capability.revision.clone(),
                    request_id: service_id.clone(),
                    class_id: option.class_id().map(str::to_owned),
                    region: option.region().map(str::to_owned),
                    reason: Some(crate::secret_redaction::redact_secrets(reason).into_owned()),
                },
                &format!("machine:{service_id}:acquire"),
                Some(trace),
            )
            .await;
        let result = match result {
            Ok(waiting @ ControlPlaneResponse::AcquisitionWaiting { .. }) => {
                return pending_payload(trace, waiting);
            }
            Ok(result) => result,
            Err(error) => {
                self.close_acquire_option(trace.agent, trace.request_id, &option, &error)?;
                return Err(choice::acquire_refusal(
                    error,
                    &capability,
                    &option,
                    trace.request_id,
                ));
            }
        };
        let ControlPlaneResponse::Machine { allocation_ref, .. } = &result else {
            return Err(acquisition_error(
                "invalid_response",
                "The service returned no allocation. Retry with the same request_id",
            ));
        };
        trace.record("allocated", None, Some(allocation_ref));
        self.accept_acquire_option(trace.agent, trace.request_id, &option, allocation_ref)?;
        let estimate = json!({"kind":"unknown","reason":"No duration was specified; the total depends on when the machine is deleted"});
        Ok(match option {
            AcquireOption::Named { class_id, region } => {
                let option = capability.offers.iter().find(|option| {
                    option.class_id == class_id
                        && region
                            .as_ref()
                            .is_none_or(|region| &option.region == region)
                });
                json!({"allocation":result,"option":option,"estimate":estimate})
            }
            AcquireOption::ServiceDefault => {
                json!({"allocation":result,"offer":capability.offer,"estimate":estimate})
            }
        })
    }
}

fn pending_payload(
    trace: &AcquireTrace<'_>,
    waiting: ControlPlaneResponse,
) -> Result<Value, DaemonError> {
    let ControlPlaneResponse::AcquisitionWaiting {
        registration_id,
        expires_at_ms,
    } = waiting
    else {
        return Err(acquisition_error(
            "invalid_response",
            "The kernel returned no approval wait. Retry with the same request_id",
        ));
    };
    Ok(json!({
        "status":"pending",
        "message":"Yield on registration_id for the decision. If approved, acquire again with this request_id.",
        "request_id":trace.request_id,
        "interaction_id":trace.interaction_id,
        "registration_id":registration_id,
        "expires_at_ms":expires_at_ms,
    }))
}
