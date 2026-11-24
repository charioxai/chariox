//! The owner's setting "Agents may acquire machines without asking" and its limits. This is the one
//! place that decides whether an agent's acquisition goes ahead without the owner's card, and it
//! keeps the ledger of what went ahead: class, rate, start, end and cost, told in the agent's Room.
//! Each answer the service gives moves the request's ledger row along `AcquiredState`.
use super::*;
use crate::durable_state::acquired_machines::{
    AcquiredMachine, AcquiredMachineWrite, AcquiredState, AcquisitionKey, Admitted, Registered,
    MINUTE_MS,
};
use crate::local::{ControlPlaneMachineRow, ControlPlaneResponse, JobMachineRate};

const DAY_MS: u64 = 86_400_000;

/// Whether an agent's acquisition needs the owner's card.
pub(crate) enum Admission {
    /// The owner has not allowed acquisitions without asking.
    Off,
    /// Allowed, but this request is outside the owner's limits, for this reason.
    Outside(String),
    /// Within the owner's limits, or a retry of a request admitted before: its ledger row holds its
    /// place until the service's answer settles it.
    Within(AcquisitionHold),
}

/// One admitted attempt on its way to the service; `settle_acquisition` applies its answer. Its
/// count in the ledger was taken in the same step that found or created its request's row (see
/// `AcquiredState`), so a sibling attempt's refusal frees the row only once no attempt is in
/// flight. The hold is consumed by the transition that settles it, so it settles once.
#[derive(Debug)]
pub(crate) struct AcquisitionHold {
    agent_id: String,
    request_id: String,
    registered: Registered,
}

impl AcquisitionHold {
    fn counted(&self) -> bool {
        self.registered == Registered::Counted
    }

    fn key(&self) -> AcquisitionKey {
        AcquisitionKey::Request {
            agent_id: self.agent_id.clone(),
            request_id: self.request_id.clone(),
        }
    }
}

/// What an agent's request would acquire, as the service's capability lists it.
struct Requested<'a> {
    class_id: &'a str,
    rate: &'a JobMachineRate,
    /// The smallest class of the same list: the only one allowed when the owner named none.
    smallest: &'a str,
}

fn requested<'a>(
    request: &'a ControlPlaneRequest,
    capability: &'a MachineAllocationCapability,
) -> Result<Requested<'a>, String> {
    match request {
        ControlPlaneRequest::AcquireJob { needs, .. } => {
            let class = super::super::job_compute::select_job_class(capability, needs)
                .map_err(|error| error.to_string())?;
            let smallest = capability
                .job_offers
                .iter()
                .min_by_key(|offer| {
                    (
                        offer.vcpu,
                        offer.usable_memory_mib,
                        offer.rate_per_minute.amount_micros,
                    )
                })
                .map_or(class.class_id.as_str(), |offer| offer.class_id.as_str());
            Ok(Requested {
                class_id: &class.class_id,
                rate: &class.rate_per_minute,
                smallest,
            })
        }
        ControlPlaneRequest::Acquire {
            class_id: Some(class_id),
            region,
            ..
        } => {
            let option = capability
                .offers
                .iter()
                .find(|option| {
                    &option.class_id == class_id
                        && region
                            .as_ref()
                            .is_none_or(|region| &option.region == region)
                })
                .ok_or_else(|| format!("{class_id} is not in the machine options"))?;
            let smallest = capability
                .offers
                .iter()
                .min_by(|a, b| {
                    (a.vcpu, a.ram_gb, a.rate_per_minute.amount_micros)
                        .partial_cmp(&(b.vcpu, b.ram_gb, b.rate_per_minute.amount_micros))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map_or(class_id.as_str(), |option| option.class_id.as_str());
            Ok(Requested {
                class_id,
                rate: &option.rate_per_minute,
                smallest,
            })
        }
        // A request that names no class takes the service's own default, which is its only option.
        ControlPlaneRequest::Acquire { class_id: None, .. } => {
            let offer = capability
                .offer
                .as_ref()
                .ok_or_else(|| "the service lists no default machine".to_owned())?;
            let rate = offer
                .rate_per_minute
                .as_ref()
                .ok_or_else(|| "the default machine has no rate per minute".to_owned())?;
            Ok(Requested {
                class_id: &offer.machine_type,
                rate,
                smallest: &offer.machine_type,
            })
        }
        _ => Err("it names no class_id from chariox.machines.options".into()),
    }
}

/// An uncertain acquisition names the request_id its ledger row has, which is the service's derived
/// one; the agent retries with the request_id it sent (or the one it was generated), so say that one.
pub(super) fn name_agent_request(
    error: DaemonError,
    service_request_id: &str,
    agent_request_id: &str,
) -> DaemonError {
    match error {
        DaemonError::MachineRequest { code, message } if code == "acquisition_uncertain" => {
            DaemonError::MachineRequest {
                message: message.replace(service_request_id, agent_request_id),
                code,
            }
        }
        error => error,
    }
}

#[cfg(test)]
type Seam = (String, Box<dyn FnOnce() + Send>);

/// Test seam: runs once, for the request it names, where an attempt found no row for its request.
#[cfg(test)]
pub(super) static UNRECORDED_ATTEMPT: std::sync::Mutex<Option<Seam>> = std::sync::Mutex::new(None);

#[cfg(test)]
fn unrecorded_attempt(request_id: &str) {
    let seam = {
        let mut seam = UNRECORDED_ATTEMPT.lock().unwrap();
        match &*seam {
            Some((named, _)) if named == request_id => seam.take(),
            _ => None,
        }
    };
    if let Some((_, run)) = seam {
        run();
    }
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

fn amount(micros: u64, currency: &str) -> String {
    format!("{} {currency}", super::money(micros))
}

impl KernelRuntimeState {
    /// Decides whether the agent's acquisition goes ahead without the owner's card and, when it does,
    /// admits it to the ledger. Only the owner's settings decide; nothing an agent sends does.
    pub(super) fn admit_without_asking(
        &self,
        room: &str,
        agent: &str,
        request: &ControlPlaneRequest,
        capability: Option<&MachineAllocationCapability>,
    ) -> Result<Admission, DaemonError> {
        let (request_id, job_ref) = match request {
            ControlPlaneRequest::Acquire { request_id, .. } => (request_id, None),
            ControlPlaneRequest::AcquireJob {
                request_id,
                job_ref,
                ..
            } => (request_id, Some(job_ref)),
            _ => return Ok(Admission::Off),
        };
        let hold = |registered| {
            Admission::Within(AcquisitionHold {
                agent_id: agent.into(),
                request_id: request_id.clone(),
                registered,
            })
        };
        let store = &self.owned.durable_state_store;
        // A retry of a request admitted before takes no new place and is never asked again: the
        // service replays its answer, which settles the same row.
        if let Some(registered) = store.count_recorded_attempt(agent, request_id)? {
            return Ok(hold(registered));
        }
        #[cfg(test)]
        unrecorded_attempt(request_id);
        let settings = self.owned.config_projection.snapshot().user_config.agents;
        if settings.offload_without_asking != Some(true) {
            return Ok(Admission::Off);
        }
        let Some(capability) = capability else {
            return Ok(Admission::Outside(
                "the connected service lists no machine options".into(),
            ));
        };
        let requested = match requested(request, capability) {
            Ok(requested) => requested,
            Err(reason) => return Ok(Admission::Outside(reason)),
        };
        let allowed = match &settings.offload_classes {
            Some(classes) => classes.iter().any(|class| class == requested.class_id),
            None => requested.class_id == requested.smallest,
        };
        if !allowed {
            let classes = settings.offload_classes.as_ref().map_or_else(
                || requested.smallest.to_owned(),
                |classes| classes.join(", "),
            );
            return Ok(Admission::Outside(format!(
                "{} is not a class agents may acquire without asking ({classes})",
                requested.class_id
            )));
        }
        let now = crate::session::unix_epoch_ms();
        let day = now - now % DAY_MS;
        let max = settings
            .offload_max_machines
            .unwrap_or(crate::config::DEFAULT_OFFLOAD_MAX_MACHINES);
        let machine = AcquiredMachine {
            agent_id: agent.into(),
            request_id: request_id.clone(),
            job_ref: job_ref.cloned(),
            room_id: room.into(),
            class_id: requested.class_id.into(),
            rate_micros: requested.rate.amount_micros,
            currency: requested.rate.currency.clone(),
            started_ms: now,
            state: AcquiredState::Pending,
        };
        let cap = settings.offload_daily_cap_micros;
        match store.admit_attempt(machine, max, cap, day)? {
            Admitted::Registered(registered) => Ok(hold(registered)),
            Admitted::AtMaximum => Ok(Admission::Outside(format!(
                "the most machines acquired without asking at the same time ({max}) are already running"
            ))),
            Admitted::AtCap => {
                let spent: u64 = store
                    .acquired_machines(day)?
                    .iter()
                    .map(|machine| machine.cost_micros(day, now))
                    .sum();
                Ok(Admission::Outside(format!(
                    "machines acquired without asking cost {} today, at the daily cap of {}",
                    amount(spent, &requested.rate.currency),
                    amount(cap.unwrap_or_default(), &requested.rate.currency)
                )))
            }
        }
    }

    /// Applies the service's answer to one attempt at an admitted request. Accepted, the machine
    /// runs and the Room is told. Refused, the attempt releases its count, and the place is free
    /// again once every attempt was refused and no answer was lost. Any other end makes it
    /// uncertain, so a machine the service may have created keeps its place and its cost, and the
    /// agent is told the request_id to retry with.
    pub(crate) fn settle_acquisition(
        &self,
        hold: AcquisitionHold,
        sent: Result<ControlPlaneResponse, DaemonError>,
    ) -> Result<ControlPlaneResponse, DaemonError> {
        match &sent {
            Ok(ControlPlaneResponse::Machine { allocation_ref, .. }) => {
                self.accept_acquired_machine(hold.key(), allocation_ref);
            }
            Err(error) if crate::runtime::cloud_api_client::cloud_error_is_refusal(error) => {
                self.withdraw_acquisition(hold);
            }
            Ok(_) => self.lose_acquisition_answer(&hold),
            Err(error) => {
                self.lose_acquisition_answer(&hold);
                return Err(acquisition_error(
                    "acquisition_uncertain",
                    &format!("{error}. The service's answer was lost, so the machine may exist: it counts against the owner's limits for acquiring without asking until you retry with request_id \"{}\", which returns the machine the service made for it, or makes it now", hold.request_id),
                ));
            }
        }
        sent
    }

    /// An attempt that was refused or never sent releases its count: the request leaves the ledger
    /// when it was the last attempt and no answer was lost.
    pub(crate) fn withdraw_acquisition(&self, hold: AcquisitionHold) {
        if hold.counted() {
            self.record_acquired_machine(AcquiredMachineWrite::Release { key: hold.key() });
        }
    }

    /// An attempt whose answer was lost: the machine may exist, so its request stays counted until
    /// a settled outcome.
    fn lose_acquisition_answer(&self, hold: &AcquisitionHold) {
        if hold.counted() {
            self.record_acquired_machine(AcquiredMachineWrite::Lost { key: hold.key() });
        }
    }

    /// A job's cleanup, after the service fenced the job, learned whether its machine exists.
    pub(crate) fn settle_job_acquisition(&self, job_ref: &str, found: Option<(&str, &str)>) {
        let key = AcquisitionKey::Job(job_ref.into());
        match found {
            Some((allocation_ref, machine_state)) => {
                self.accept_acquired_machine(key, allocation_ref);
                self.observe_acquired_machine(
                    allocation_ref,
                    machine_state,
                    crate::session::unix_epoch_ms(),
                );
            }
            None => {
                self.record_acquired_machine(AcquiredMachineWrite::Remove { key });
            }
        }
    }

    /// The service said `allocation_ref` is in `machine_state`: deletion moves a machine acquired
    /// without asking on, and only a confirmed deletion ends it, at `at_ms`.
    pub(crate) fn observe_acquired_machine(
        &self,
        allocation_ref: &str,
        machine_state: &str,
        at_ms: u64,
    ) {
        match machine_state {
            "deleting" => {
                self.record_acquired_machine(AcquiredMachineWrite::Deleting {
                    allocation_ref: allocation_ref.into(),
                });
            }
            "deleted" => self.end_acquired_machine(allocation_ref, at_ms),
            _ => {}
        }
    }

    /// The machines a list sent now must account for: every one accepted before it was sent. A
    /// machine accepted later may be missing from that list and is never ended by it.
    pub(crate) fn acquired_machines_to_reconcile(&self) -> Vec<String> {
        self.owned
            .durable_state_store
            .acquired_machines(i64::MAX as u64)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|machine| match machine.state {
                AcquiredState::Running { allocation_ref }
                | AcquiredState::Deleting { allocation_ref } => Some(allocation_ref),
                AcquiredState::Pending | AcquiredState::Uncertain | AcquiredState::Ended { .. } => {
                    None
                }
            })
            .collect()
    }

    /// Settles each of `expected` against the service's whole list `machines`, sent after they
    /// were read: one missing from it has ended.
    pub(crate) fn reconcile_acquired_machines(
        &self,
        expected: &[String],
        machines: &[ControlPlaneMachineRow],
    ) {
        let now = crate::session::unix_epoch_ms();
        for allocation_ref in expected {
            match machines
                .iter()
                .find(|row| &row.allocation_ref == allocation_ref)
            {
                Some(row) => {
                    let since = chrono::DateTime::parse_from_rfc3339(&row.since)
                        .ok()
                        .and_then(|since| u64::try_from(since.timestamp_millis()).ok())
                        .unwrap_or(now);
                    self.observe_acquired_machine(allocation_ref, &row.state, since);
                }
                None => self.end_acquired_machine(allocation_ref, now),
            }
        }
    }

    fn accept_acquired_machine(&self, key: AcquisitionKey, allocation_ref: &str) {
        if !self.record_acquired_machine(AcquiredMachineWrite::Accept {
            key,
            allocation_ref: allocation_ref.into(),
        }) {
            return;
        }
        let Some(machine) = self.acquired_machine_by_allocation(allocation_ref) else {
            return;
        };
        let now = crate::session::unix_epoch_ms();
        let day = now - now % DAY_MS;
        let acquired = self
            .owned
            .durable_state_store
            .acquired_machines(day)
            .unwrap_or_default();
        let running = acquired
            .iter()
            .filter(|machine| machine.state.is_live())
            .count();
        let spent: u64 = acquired
            .iter()
            .map(|machine| machine.cost_micros(day, now))
            .sum();
        self.tell_room(
            &machine,
            format!(
                "{} acquired {} without asking at {} per minute. Running without asking: {running}; spent today: {}.",
                capitalized(&self.agent_name(&machine.agent_id)),
                machine.class_id,
                amount(machine.rate_micros, &machine.currency),
                amount(spent, &machine.currency),
            ),
        );
    }

    /// A machine acquired without asking ended at `ended_ms`: the ledger closes it and the Room is
    /// told what it cost. Any other allocation is not in the ledger and changes nothing.
    fn end_acquired_machine(&self, allocation_ref: &str, ended_ms: u64) {
        if !self.record_acquired_machine(AcquiredMachineWrite::End {
            allocation_ref: allocation_ref.into(),
            ended_ms,
        }) {
            return;
        }
        let Some(machine) = self.acquired_machine_by_allocation(allocation_ref) else {
            return;
        };
        let AcquiredState::Ended { ended_ms, .. } = machine.state else {
            return;
        };
        self.tell_room(
            &machine,
            format!(
                "The {} machine that {} acquired without asking ended after {} min; it cost {}.",
                machine.class_id,
                self.agent_name(&machine.agent_id),
                ended_ms
                    .saturating_sub(machine.started_ms)
                    .div_ceil(MINUTE_MS),
                amount(machine.cost_micros(0, ended_ms), &machine.currency),
            ),
        );
    }

    /// Applies one ledger transition; false when the row was not in the state it expects.
    fn record_acquired_machine(&self, write: AcquiredMachineWrite) -> bool {
        self.owned
            .durable_state_store
            .write_acquired_machine(write)
            .unwrap_or_else(|error| {
                crate::logging::error_with_fields(
                    "kernel.machines",
                    "a machine acquired without asking could not be recorded",
                    serde_json::json!({"error": error.to_string()}),
                );
                false
            })
    }

    fn acquired_machine_by_allocation(&self, allocation_ref: &str) -> Option<AcquiredMachine> {
        self.owned
            .durable_state_store
            .acquired_machine(allocation_ref)
            .ok()
            .flatten()
    }

    fn agent_name(&self, agent_id: &str) -> String {
        self.owned
            .agent_store
            .get_agent(agent_id)
            .ok()
            .and_then(|agent| agent.alias().map(|alias| format!("@{alias}")))
            .unwrap_or_else(|| "an agent".into())
    }

    fn tell_room(&self, machine: &AcquiredMachine, message: String) {
        self.owned.record_notice_for_agent(
            &machine.room_id,
            None,
            Some(&machine.agent_id),
            self.owned
                .attachment_store
                .list_session_attachment_ids(&machine.room_id),
            message,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_acquired_machine_costs_each_started_minute_within_the_window() {
        let machine = |state| AcquiredMachine {
            agent_id: "agent".into(),
            request_id: "r".into(),
            job_ref: None,
            room_id: "room".into(),
            class_id: "small".into(),
            rate_micros: 1_000,
            currency: "EUR".into(),
            started_ms: DAY_MS - MINUTE_MS / 2,
            state,
        };
        let running = machine(AcquiredState::Running {
            allocation_ref: "a".into(),
        });
        assert_eq!(running.cost_micros(0, DAY_MS), 1_000);
        // Only today's part counts against today's cap.
        assert_eq!(running.cost_micros(DAY_MS, DAY_MS + 3 * MINUTE_MS), 3_000);
        let ended = machine(AcquiredState::Ended {
            allocation_ref: "a".into(),
            ended_ms: DAY_MS + 1,
        });
        assert_eq!(ended.cost_micros(0, DAY_MS * 2), 1_000);
        assert_eq!(ended.cost_micros(DAY_MS + 2, DAY_MS * 2), 0);
    }
}
