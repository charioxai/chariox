//! Read the original agent-scoped request without contacting the machine service.
use super::approval::{decision_error, direct_approval_id, MachineDecision};
use super::*;
use crate::durable_state::acquire_choices::{AcquireChoice, AcquireChoiceState, AcquireSubmission};
use crate::transport::runtime_tools::RuntimeToolResult;
use serde_json::json;

impl KernelRuntimeState {
    pub(super) fn machine_request_status(
        &self,
        room: &str,
        agent: &str,
        request_id: &str,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let store = &self.owned.durable_state_store;
        let choice = store.acquire_choice(agent, request_id, crate::session::unix_epoch_ms())?;
        let service_id = choice::service_request_id(
            &self.owned.config_projection.snapshot().daemon_id,
            agent,
            request_id,
        );
        let interaction_id = direct_approval_id(room, agent, &service_id);
        let session = self.owned.session_store.get_session(room)?;
        let wake = store.agent_wake(&interaction_id)?;
        let decision = self
            .owned
            .machine_decision(&interaction_id, session.owner_user_id())?;
        let response =
            |status, message| json!({"status":status,"request_id":request_id,"message":message});
        let unconfirmed =
            "The kernel saved the request but has no confirmed allocation. Retry acquire with the same request_id";
        let mut payload = match (choice.as_ref(), decision, wake) {
            (Some(AcquireChoice { state: AcquireChoiceState::Accepted { allocation_ref }, .. }), _, _) => {
                let mut value = response(
                    "allocated",
                    "The service accepted this request. Use allocation_ref to read the machine's current state",
                );
                value["allocation_ref"] = json!(allocation_ref);
                value
            }
            (Some(AcquireChoice { submission: AcquireSubmission::Closed, .. }), _, _) => response(
                "closed",
                "The service closed the request without an allocation. Use a new request_id to try again",
            ),
            (Some(AcquireChoice { submission: AcquireSubmission::Submitted, .. }), _, _) => response(
                "unconfirmed",
                unconfirmed,
            ),
            (_, Some(MachineDecision::Approved { .. }), _) => response(
                "approved",
                "The approval is saved. Retry acquire with the same request_id to complete it or recover the allocation",
            ),
            (_, Some(decision @ MachineDecision::Denied { .. }), _) => {
                json!({"status":"denied","request_id":request_id,"message":decision_error(&decision).to_string()})
            }
            (_, Some(decision @ MachineDecision::Expired { .. }), _) => {
                json!({"status":"expired","request_id":request_id,"message":decision_error(&decision).to_string()})
            }
            (_, None, Some(wake)) => match wake.state.as_str() {
                "running" => {
                    let mut value = response(
                        "pending",
                        "The approval is waiting for an answer. Yield on registration_id",
                    );
                    value["interaction_id"] = json!(interaction_id);
                    value["registration_id"] = json!(wake.registration_id);
                    value["expires_at_ms"] = json!(wake.next_due_ms);
                    value
                }
                "starting" => response(
                    "opening",
                    "The kernel is opening the approval. Retry status shortly with the same request_id",
                ),
                "cancelled" | "cancelling" => response(
                    "cancelled", "The approval was cancelled. Use a new request_id to ask again",
                ),
                _ => response(
                    "unavailable",
                    "The approval wait ended without a saved decision. Use a new request_id to ask again",
                ),
            },
            (Some(_), None, None) => response("unconfirmed", unconfirmed),
            (None, None, None) => response(
                "unknown",
                "This kernel has no receipt for this request_id from your agent. If a reply was lost, retry acquire with the same request_id",
            ),
        };
        if let Some(choice) = choice {
            payload["class_id"] = json!(choice.option.class_id());
            payload["region"] = json!(choice.option.region());
        }
        Ok(RuntimeToolResult { ok: true, payload })
    }
}
