//! Which machine option an acquire asks for, and how every try of one request keeps it.
//!
//! An agent's request_id names one acquisition. The kernel sends the service a request id derived
//! from the kernel, the agent and that request_id, and keeps the option it asked for in the
//! durable row of (agent, request_id). The service binds an accepted request to its option and
//! replays it before any stock or catalogue check, so every try of a request sends the option its
//! first try chose, whatever the service answered before. Each arrow below is one compare-and-set
//! on that row; no transition writes another option. The row's lifetime is the store's: it
//! stays for the replay window after its last write (`durable_state::acquire_choices`).
//!
//! An option P is a listed class with its region, or the service's own default when it lists no
//! options; both are remembered and replayed the same way, whatever the service lists later.
//!
//! | row \ outcome of a try | accepted (new or replay) | closed refusal | anything else: refusal, failure before or after the post, lost response, timeout | restart |
//! |-----------------|-----------------------------|----------------|---------------------|---------|
//! | none            | chose P → pending P → accepted P, ref | chose P → pending P, window restarts | chose P → pending P | none |
//! | pending P       | accepted P, ref             | pending P, window restarts | pending P | pending P |
//! | accepted P, ref | accepted P, ref             | accepted P, ref | accepted P, ref     | accepted P, ref |
//!
//! Every write restarts the replay window, so a request that is accepted, closed or abandoned
//! keeps its answer for a retry of the same request_id for 24 hours and is then forgotten: a
//! request_id used again after that chooses its option anew. Releasing an allocation never needs
//! the row, only the allocation_ref.
//!
//! Tries of one request overlap: a try may still wait for its capability call or the owner's
//! decision when another is refused. So only the service can say that a request holds nothing:
//! a refusal that closes the request (`details.request_id` is `"closed"`). The service closes a
//! request under the same account lock its allocations take, only when no allocation holds it,
//! and refuses every later try of it as closed, so a try still in flight never allocates under a
//! closed request. Only that refusal says nothing was allocated or charged and sends the caller to
//! a new request_id. Every other refusal never frees the option and sends the caller back to the
//! same request_id, which is safe to retry: the service never allocates twice under it, returns
//! the allocation it accepted and, when it refuses again, closes it. No other answer, and no list
//! of machines, proves that a try still in flight will not allocate.
//!
//! - Refusals are a closed request (`AcquireFailure::Closed`), and `capacity_exceeded`,
//!   `budget_exceeded` and `invalid_request` (`AcquireFailure::Refused`) answering the acquire
//!   itself; the refusal names them. Every other
//!   failure passes on as it is. The connected-service boundary passes the capability call's
//!   failure before the acquire on without its Cloud code (`control_plane_offer_unavailable`), so
//!   no code it carries is read as a refusal of the acquire.
//! - Accepted writes the row with the option the try sent, whatever the row was and even when it
//!   is gone: a try may wait for the service or the owner's decision past the replay window, and
//!   the allocation it was given must not be lost to a sweep. The service's acceptance is the
//!   truth, even after a refusal. Only a row accepted under another allocation or option stays.
//! - Choosing writes only a missing row; a try that loses that race takes the option the winner
//!   wrote. An explicit class_id or region that differs from the row's is refused before the
//!   service is asked.
//! - A restart keeps the row, so a retry after a restart resends what the first try sent.
//! - Nothing writes another option into a row, so within its window a request_id never forgets
//!   the option under which the service may hold an allocation; releasing it is the owner's
//!   `chariox.machines.release` of the allocation_ref.
use super::*;
use crate::durable_state::acquire_choices::{AcquireChoiceWrite, AcquireOption};
use crate::runtime::cloud_api_client::{
    acquire_failure, cloud_error_code, cloud_error_reason, AcquireFailure,
};
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
            "No machine option is in stock right now, so this try was not sent. Try again later: chariox.machines.options shows the options and their stock",
        )),
    }
}

/// A refused acquire names what the service refused. A closed request says nothing was allocated
/// under it and sends the caller to a new request_id; any other refusal sends the caller back to
/// the same request_id, which is safe to retry, because another try of it may still be in flight.
pub(super) fn acquire_refusal(
    error: DaemonError,
    capability: &MachineAllocationCapability,
    option: &AcquireOption,
    request_id: &str,
) -> DaemonError {
    let (out_of_stock, closed) = match acquire_failure(&error) {
        AcquireFailure::Closed { out_of_stock } => (out_of_stock, true),
        AcquireFailure::Refused { out_of_stock } => (out_of_stock, false),
        AcquireFailure::Ambiguous => return error,
    };
    let subject = match option {
        AcquireOption::Named { .. } => option_name(option),
        AcquireOption::ServiceDefault => "The default option".to_owned(),
    };
    let (code, refused) = if out_of_stock {
        (
            "capacity_unavailable".to_owned(),
            format!("{subject} is out of stock"),
        )
    } else {
        (
            cloud_error_code(&error)
                .unwrap_or("invalid_request")
                .to_owned(),
            format!(
                "The service refused {}: {}",
                option_name(option),
                cloud_error_reason(&error).unwrap_or("no reason given")
            ),
        )
    };
    if !closed {
        return acquisition_error(
            &code,
            &format!(
                "{refused}. Retrying chariox.machines.acquire with request_id {request_id} is safe: it keeps this option, the service never allocates twice under it and returns the machine if a try of it was accepted. Keep this request_id until the service accepts it or closes it."
            ),
        );
    }
    // Only the refused option is left out: the same class in another region may be in stock.
    let (chosen, chosen_region) = (option.class_id(), option.region());
    let in_stock: Vec<String> = capability
        .offers
        .iter()
        .filter(|offer| {
            out_of_stock
                && offer.available
                && !(Some(offer.class_id.as_str()) == chosen
                    && chosen_region.is_none_or(|region| offer.region == region))
        })
        .map(|offer| format!("{} in {}", offer.class_id, offer.region))
        .collect();
    let in_stock = if in_stock.is_empty() {
        String::new()
    } else {
        format!(" In stock now: {}.", in_stock.join(", "))
    };
    acquisition_error(
        &code,
        &format!(
            "{refused}. The service closed request_id {request_id}: nothing was allocated or charged under it, and no try of it ever will be. Use a new request_id for this option or another.{in_stock}"
        ),
    )
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
    /// The option this try of `request_id` asks for: the one an earlier try took, else a new one.
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
            match store.acquire_choice(agent_id, request_id, crate::session::unix_epoch_ms())? {
                Some(choice) => {
                    let option = choice.option.clone();
                    check_acquire_option(
                        request_id,
                        &option,
                        class_id.as_deref(),
                        region.as_deref(),
                    )?;
                    return Ok(option);
                }
                None => {
                    let option = acquire_choice(capability, class_id.clone(), region.clone())?;
                    if store.write_acquire_choice(
                        agent_id,
                        request_id,
                        AcquireChoiceWrite::Choose {
                            option: option.clone(),
                        },
                        crate::session::unix_epoch_ms(),
                    )? {
                        return Ok(option);
                    }
                }
            }
        }
    }

    /// Keeps the row of a request the service closed for another replay window, so a retry of it
    /// gets the same refusal. Any other failure leaves the row as it is.
    pub(super) fn close_acquire_option(
        &self,
        agent_id: &str,
        request_id: &str,
        option: &AcquireOption,
        refusal: &DaemonError,
    ) -> Result<(), DaemonError> {
        if matches!(acquire_failure(refusal), AcquireFailure::Closed { .. }) {
            self.owned.durable_state_store.write_acquire_choice(
                agent_id,
                request_id,
                AcquireChoiceWrite::Closed {
                    option: option.clone(),
                },
                crate::session::unix_epoch_ms(),
            )?;
        }
        Ok(())
    }

    /// Records the allocation the service accepted for the option this try sent.
    pub(super) fn accept_acquire_option(
        &self,
        agent_id: &str,
        request_id: &str,
        option: &AcquireOption,
        allocation_ref: &str,
    ) -> Result<(), DaemonError> {
        if self.owned.durable_state_store.write_acquire_choice(
            agent_id,
            request_id,
            AcquireChoiceWrite::Accept {
                option: option.clone(),
                allocation_ref: allocation_ref.to_owned(),
            },
            crate::session::unix_epoch_ms(),
        )? {
            return Ok(());
        }
        Err(acquisition_error(
            "invalid_response",
            &format!("The service returned another allocation for request_id {request_id} than it accepted before; check chariox.machines.status for both"),
        ))
    }
}

pub(super) fn check_acquire_option(
    request_id: &str,
    option: &AcquireOption,
    class: Option<&str>,
    region: Option<&str>,
) -> Result<(), DaemonError> {
    if class.is_some_and(|class| Some(class) != option.class_id())
        || region.is_some_and(|region| Some(region) != option.region())
    {
        return Err(acquisition_error(
            "invalid_request",
            &format!(
                "request_id {request_id} already asked for {}; retry it without class_id and region. Use a new request_id only when the service closes this one.",
                option_name(option),
            ),
        ));
    }
    Ok(())
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
