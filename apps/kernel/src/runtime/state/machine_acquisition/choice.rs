//! Which machine option an acquire asks for, and how every try of one request keeps it.
//!
//! An agent's request_id names one acquisition. The kernel sends the service a request id derived
//! from the kernel, the agent and that request_id, and keeps the option it asked for in the
//! durable row of (agent, request_id). The service binds an accepted request to its option and
//! replays it, so every try must send the same option until the service has said that it holds
//! nothing under the request. Each arrow below is one compare-and-set on that row.
//!
//! An option P or Q is a listed class with its region, or the service's own default when it lists
//! no options; both are remembered and replayed the same way, whatever the service lists later.
//!
//! | row \ outcome of a try | accepted (new or replay) | failed before the acquire was sent | lost response, timeout, transient or other code | definitive refusal | restart |
//! |-----------------|---------------------------|-------------|-------------|--------------------|---------|
//! | none            | chose P → pending P → …   | chose P → pending P | chose P → pending P | chose P → pending P → refused P | none |
//! | pending P       | accepted P, ref           | pending P   | pending P   | refused P          | pending P |
//! | accepted P, ref | accepted P, ref           | accepted P, ref | accepted P, ref | accepted P, ref | accepted P, ref |
//! | refused P       | chose Q → pending Q → …   | chose Q → pending Q | chose Q → pending Q | chose Q → pending Q → refused Q | refused P |
//!
//! - Definitive refusals are `capacity_exceeded`, `budget_exceeded` and `invalid_request`
//!   (`AcquireFailure::Refused`) answering the acquire itself: the service sends them only before
//!   it commits a new request and replays an accepted one before any check that sends them.
//!   Everything else is ambiguous: `internal_error`, `dependency_unavailable`, `rate_limited`,
//!   `admission_denied`, any 5xx, a lost or undecodable response, a timeout, and the other codes
//!   (`authorization_denied`, `identity_conflict`, `subscription_required`, ...). An ambiguous try
//!   keeps the row, so the retry resends the same option and the service replays what it accepted.
//! - A try that fails before the acquire is sent keeps the row whatever the failure says: the
//!   capability call before it, the owner's denial and a local refusal. The connected-service
//!   boundary passes the capability call's failure on without its Cloud code
//!   (`control_plane_offer_unavailable`), so no code it carries is read as a refusal.
//! - Refused moves only a pending row that still holds the option the refused try sent; a
//!   concurrent try that was accepted keeps its row. Accepted moves any row but one accepted under
//!   another allocation: the service's acceptance is the truth, even after a concurrent refusal.
//! - Choosing writes only a missing or refused row; a try that loses that race takes the option
//!   the winner wrote. A pending or accepted row is never chosen again, and an explicit class_id or
//!   region that differs from it is refused before the service is asked.
//! - A restart keeps the row, so a retry after a restart resends what the first try sent.
use super::*;
use crate::durable_state::acquire_choices::{
    AcquireChoice, AcquireChoiceState, AcquireChoiceWrite, AcquireOption,
};
use crate::runtime::cloud_api_client::{acquire_failure, AcquireFailure};
use sha2::Digest;

/// The option an acquire asks for: the one the agent named, else the first option the service
/// lists in stock. A service that lists no options is asked for its own default.
pub(super) fn acquire_choice(
    capability: &MachineAllocationCapability,
    class_id: Option<String>,
    region: Option<String>,
) -> Result<AcquireOption, DaemonError> {
    if class_id.is_some() || capability.offers.is_empty() {
        if class_id.is_none() && region.is_some() {
            return Err(acquisition_error(
                "invalid_request",
                "Pass class_id with region, from chariox.machines.options",
            ));
        }
        return Ok(match class_id {
            Some(class_id) => AcquireOption::Named { class_id, region },
            None => AcquireOption::ServiceDefault,
        });
    }
    match capability.offers.iter().find(|option| {
        option.available && region.as_ref().is_none_or(|region| &option.region == region)
    }) {
        Some(option) => Ok(AcquireOption::Named {
            class_id: option.class_id.clone(),
            region: Some(option.region.clone()),
        }),
        None => Err(acquisition_error(
            "capacity_unavailable",
            "No machine option is in stock right now; nothing was allocated or charged. Try again later: chariox.machines.options shows the options and their stock",
        )),
    }
}

/// A refused acquire names what is in stock and how to choose it, so the agent can pick again.
/// Only the refused option is left out: the same class in another region may be in stock.
pub(super) fn out_of_stock_refusal(
    error: DaemonError,
    capability: &MachineAllocationCapability,
    option: &AcquireOption,
) -> DaemonError {
    if acquire_failure(&error) != (AcquireFailure::Refused { out_of_stock: true }) {
        return error;
    }
    let (chosen, chosen_region) = (option.class_id(), option.region());
    let in_stock: Vec<String> = capability
        .offers
        .iter()
        .filter(|option| {
            option.available
                && !(Some(option.class_id.as_str()) == chosen
                    && chosen_region.is_none_or(|region| option.region == region))
        })
        .map(|option| format!("{} in {}", option.class_id, option.region))
        .collect();
    let subject = match option {
        AcquireOption::Named { .. } => option_name(option),
        AcquireOption::ServiceDefault => "The default option".to_owned(),
    };
    let message = if in_stock.is_empty() {
        format!("{subject} is out of stock and no other option is in stock; nothing was allocated or charged. Try again later")
    } else {
        format!(
            "{subject} is out of stock; nothing was allocated or charged. In stock now: {}. Choose one by passing its class_id and region to chariox.machines.acquire, or pass neither to take the first in stock",
            in_stock.join(", ")
        )
    };
    acquisition_error("capacity_unavailable", &message)
}

/// The service's request id: request_ids are the agent's own, so they are scoped to this kernel
/// and agent before they reach the account-wide service.
pub(super) fn service_request_id(kernel_id: &str, agent_id: &str, request_id: &str) -> String {
    format!(
        "machine-{:x}",
        sha2::Sha256::digest(format!("{kernel_id}\0{agent_id}\0{request_id}").as_bytes())
    )
}

impl KernelRuntimeState {
    /// The option this try of `request_id` asks for: the one an earlier try took, unless the
    /// service refused it, else a new one.
    pub(super) fn acquire_option(
        &self,
        agent_id: &str,
        request_id: &str,
        capability: &MachineAllocationCapability,
        class_id: Option<String>,
        region: Option<String>,
    ) -> Result<AcquireOption, DaemonError> {
        let store = &self.owned.durable_state_store;
        loop {
            match store.acquire_choice(agent_id, request_id)? {
                Some(AcquireChoice { option, state }) if state != AcquireChoiceState::Refused => {
                    let differs = |named: &Option<String>, kept: Option<&str>| {
                        named.is_some() && named.as_deref() != kept
                    };
                    if differs(&class_id, option.class_id()) || differs(&region, option.region()) {
                        return Err(acquisition_error(
                            "invalid_request",
                            &format!(
                                "request_id {request_id} already asked for {}; retry it without class_id and region, or pass a new request_id to choose another option",
                                option_name(&option)
                            ),
                        ));
                    }
                    return Ok(option);
                }
                _ => {
                    let option = acquire_choice(capability, class_id.clone(), region.clone())?;
                    if store.write_acquire_choice(
                        agent_id,
                        request_id,
                        AcquireChoiceWrite::Choose {
                            option: option.clone(),
                        },
                    )? {
                        return Ok(option);
                    }
                }
            }
        }
    }

    /// Records what the service said about the option this try sent.
    pub(super) fn settle_acquire_option(
        &self,
        agent_id: &str,
        request_id: &str,
        option: &AcquireOption,
        outcome: Result<&str, &DaemonError>,
    ) -> Result<(), DaemonError> {
        let write = match outcome {
            Ok(allocation_ref) => AcquireChoiceWrite::Accept {
                option: option.clone(),
                allocation_ref: allocation_ref.to_owned(),
            },
            Err(error) => match acquire_failure(error) {
                AcquireFailure::Refused { .. } => AcquireChoiceWrite::Refuse {
                    option: option.clone(),
                },
                AcquireFailure::Ambiguous => return Ok(()),
            },
        };
        let accepted = matches!(write, AcquireChoiceWrite::Accept { .. });
        if !self
            .owned
            .durable_state_store
            .write_acquire_choice(agent_id, request_id, write)?
            && accepted
        {
            return Err(acquisition_error(
                "invalid_response",
                &format!("The service returned another allocation for request_id {request_id} than it accepted before; check chariox.machines.status for both"),
            ));
        }
        Ok(())
    }
}

fn option_name(option: &AcquireOption) -> String {
    match option {
        AcquireOption::Named {
            class_id,
            region: Some(region),
        } => format!("{class_id} in {region}"),
        AcquireOption::Named {
            class_id,
            region: None,
        } => class_id.clone(),
        AcquireOption::ServiceDefault => "the default option".to_owned(),
    }
}
