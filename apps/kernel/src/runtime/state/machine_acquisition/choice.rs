//! Which machine option an acquire asks for, and how a retry of the same request keeps it.
use super::*;
use sha2::Digest;

const CHOSEN: &str = "compute.acquire_choice";
const REFUSED: &str = "compute.acquire_choice_refused";

/// The class_id and region an acquire asks the service for; both absent asks for its default.
pub(super) type Choice = (Option<String>, Option<String>);

/// The option an acquire asks for: the one the agent named, else the first option the service
/// lists in stock. A service that lists no options is asked for its own default.
pub(super) fn acquire_choice(
    capability: &MachineAllocationCapability,
    class_id: Option<String>,
    region: Option<String>,
) -> Result<Choice, DaemonError> {
    if class_id.is_some() || capability.offers.is_empty() {
        if class_id.is_none() && region.is_some() {
            return Err(acquisition_error(
                "invalid_request",
                "Pass class_id with region, from chariox.machines.options",
            ));
        }
        return Ok((class_id, region));
    }
    match capability.offers.iter().find(|option| {
        option.available && region.as_ref().is_none_or(|region| &option.region == region)
    }) {
        Some(option) => Ok((Some(option.class_id.clone()), Some(option.region.clone()))),
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
    chosen: Option<&str>,
    chosen_region: Option<&str>,
) -> DaemonError {
    if crate::runtime::cloud_api_client::cloud_error_code(&error) != Some("capacity_exceeded") {
        return error;
    }
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
    let subject = match (chosen, chosen_region) {
        (Some(chosen), Some(region)) => format!("{chosen} in {region}"),
        (Some(chosen), None) => chosen.to_owned(),
        (None, _) => "The default option".to_owned(),
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

/// The durable record of the option an agent's request_id took, so a retry asks the service
/// for the same one even when stock has changed since.
pub(super) fn choice_subject(agent_id: &str, request_id: &str) -> String {
    format!(
        "machine-acquire-choice-{:x}",
        sha2::Sha256::digest(format!("{agent_id}:{request_id}").as_bytes())
    )
}

impl KernelRuntimeState {
    pub(super) fn remembered_acquire_choice(
        &self,
        subject: &str,
    ) -> Result<Option<Choice>, DaemonError> {
        let latest = self
            .owned
            .durable_state_store
            .load_subject_events_by_kind_prefixes(subject, &[CHOSEN], 1)?;
        Ok(latest
            .into_iter()
            .find(|event| event.kind == CHOSEN)
            .map(|event| {
                let field = |name: &str| event.payload[name].as_str().map(str::to_owned);
                (field("class_id"), field("region"))
            }))
    }

    pub(super) fn remember_acquire_choice(
        &self,
        subject: &str,
        class_id: Option<&str>,
        region: Option<&str>,
    ) -> Result<(), DaemonError> {
        self.owned.durable_state_store.append_event(
            CHOSEN,
            Some(subject.to_owned()),
            serde_json::json!({"class_id":class_id,"region":region}),
        )?;
        Ok(())
    }

    /// The service refused the request, so nothing was accepted under this id: the next try chooses afresh.
    pub(super) fn forget_acquire_choice(&self, subject: &str) -> Result<(), DaemonError> {
        self.owned.durable_state_store.append_event(
            REFUSED,
            Some(subject.to_owned()),
            serde_json::json!({}),
        )?;
        Ok(())
    }
}
