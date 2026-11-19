//! The owner's setting "Agents may acquire machines without asking" and its limits. This is the one
//! place that decides whether an agent's acquisition goes ahead without the owner's card, and it
//! keeps the ledger of what went ahead: class, rate, start, end and cost, told in the agent's Room.
use super::*;
use crate::durable_state::acquired_machines::{
    AcquiredMachine, AcquiredMachineWrite, AcquiredState,
};
use crate::local::{ControlPlaneMachineRow, JobMachineRate};

const DAY_MS: u64 = 86_400_000;
const MINUTE_MS: u64 = 60_000;

/// Whether an agent's acquisition needs the owner's card.
pub(crate) enum Admission {
    /// The owner has not allowed acquisitions without asking.
    Off,
    /// Allowed, but this request is outside the owner's limits, for this reason.
    Outside(String),
    /// Within the owner's limits: it holds one place in the ledger until the service answers.
    Within(AcquisitionHold),
}

/// One held place in the ledger. The service's answer starts or drops it.
#[derive(Debug)]
pub(crate) struct AcquisitionHold {
    id: String,
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
        _ => Err("it names no class_id from chariox.machines.options".into()),
    }
}

/// What `machine` costs between `from_ms` and `until_ms`, by started minute.
fn cost_micros(machine: &AcquiredMachine, from_ms: u64, until_ms: u64) -> u64 {
    let end = match &machine.state {
        AcquiredState::Ended { ended_ms, .. } => (*ended_ms).min(until_ms),
        AcquiredState::Held | AcquiredState::Running { .. } => until_ms,
    };
    let start = machine.started_ms.max(from_ms);
    if end <= start {
        return 0;
    }
    (end - start).div_ceil(MINUTE_MS) * machine.rate_micros
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
    /// holds its place in the ledger. Only the owner's settings decide; nothing an agent sends does.
    pub(super) fn admit_without_asking(
        &self,
        id: &str,
        room: &str,
        agent: &str,
        request: &ControlPlaneRequest,
        capability: Option<&MachineAllocationCapability>,
    ) -> Result<Admission, DaemonError> {
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
        let acquired = self.owned.durable_state_store.acquired_machines(day)?;
        if acquired
            .iter()
            .any(|machine| machine.id == id && machine.state == AcquiredState::Held)
        {
            // A retry of a request whose first try is still unanswered keeps its place.
            return Ok(Admission::Within(AcquisitionHold { id: id.into() }));
        }
        let max = settings
            .offload_max_machines
            .unwrap_or(crate::config::DEFAULT_OFFLOAD_MAX_MACHINES);
        if let Some(cap) = settings.offload_daily_cap_micros {
            let spent: u64 = acquired
                .iter()
                .map(|machine| cost_micros(machine, day, now))
                .sum();
            if spent >= cap {
                return Ok(Admission::Outside(format!(
                    "machines acquired without asking cost {} today, at the daily cap of {}",
                    amount(spent, &requested.rate.currency),
                    amount(cap, &requested.rate.currency)
                )));
            }
        }
        let held =
            self.owned
                .durable_state_store
                .write_acquired_machine(AcquiredMachineWrite::Hold {
                    machine: AcquiredMachine {
                        id: id.into(),
                        room_id: room.into(),
                        agent_id: agent.into(),
                        class_id: requested.class_id.into(),
                        rate_micros: requested.rate.amount_micros,
                        currency: requested.rate.currency.clone(),
                        started_ms: now,
                        state: AcquiredState::Held,
                    },
                    max_live: max,
                })?;
        if !held {
            return Ok(Admission::Outside(format!(
                "the most machines acquired without asking at the same time ({max}) are already running"
            )));
        }
        Ok(Admission::Within(AcquisitionHold { id: id.into() }))
    }

    /// The service answered a held acquisition: accepted, it runs from now and the Room is told;
    /// refused, its place is free again.
    pub(crate) fn settle_acquisition_hold(
        &self,
        hold: AcquisitionHold,
        allocation_ref: Option<&str>,
    ) {
        let now = crate::session::unix_epoch_ms();
        let write = match allocation_ref {
            Some(allocation_ref) => AcquiredMachineWrite::Start {
                id: hold.id.clone(),
                allocation_ref: allocation_ref.into(),
                started_ms: now,
            },
            None => AcquiredMachineWrite::Drop {
                id: hold.id.clone(),
            },
        };
        let store = &self.owned.durable_state_store;
        // A replayed request whose machine is already recorded frees this place.
        let started = store.write_acquired_machine(write).and_then(|started| {
            if !started && allocation_ref.is_some() {
                store.write_acquired_machine(AcquiredMachineWrite::Drop {
                    id: hold.id.clone(),
                })?;
            }
            Ok(started)
        });
        match started {
            Ok(true) if allocation_ref.is_some() => {}
            Ok(_) => return,
            Err(error) => {
                crate::logging::error_with_fields(
                    "kernel.machines",
                    "a machine acquired without asking could not be recorded",
                    serde_json::json!({"machine": hold.id, "error": error.to_string()}),
                );
                return;
            }
        }
        let Some(machine) = self.acquired_machine(|machine| machine.id == hold.id) else {
            return;
        };
        let day = now - now % DAY_MS;
        let acquired = self
            .owned
            .durable_state_store
            .acquired_machines(day)
            .unwrap_or_default();
        let running = acquired
            .iter()
            .filter(|machine| !matches!(machine.state, AcquiredState::Ended { .. }))
            .count();
        let spent: u64 = acquired
            .iter()
            .map(|machine| cost_micros(machine, day, now))
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
    pub(crate) fn end_acquired_machine(&self, allocation_ref: &str, ended_ms: u64) {
        let ended =
            self.owned
                .durable_state_store
                .write_acquired_machine(AcquiredMachineWrite::End {
                    allocation_ref: allocation_ref.into(),
                    ended_ms,
                });
        match ended {
            Ok(true) => {}
            Ok(false) => return,
            Err(error) => {
                crate::logging::error_with_fields(
                    "kernel.machines",
                    "the end of a machine acquired without asking could not be recorded",
                    serde_json::json!({"allocation_ref": allocation_ref, "error": error.to_string()}),
                );
                return;
            }
        }
        let Some(machine) = self.acquired_machine(|machine| {
            matches!(&machine.state, AcquiredState::Ended { allocation_ref: ended, .. } if ended == allocation_ref)
        }) else {
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
                amount(cost_micros(&machine, 0, ended_ms), &machine.currency),
            ),
        );
    }

    /// Closes every running machine the service no longer lists as live; `machines` is its whole list.
    pub(crate) fn settle_ended_acquired_machines(&self, machines: &[ControlPlaneMachineRow]) {
        let now = crate::session::unix_epoch_ms();
        let Ok(acquired) = self.owned.durable_state_store.acquired_machines(now) else {
            return;
        };
        for machine in acquired {
            let AcquiredState::Running { allocation_ref } = &machine.state else {
                continue;
            };
            let listed = machines
                .iter()
                .find(|row| &row.allocation_ref == allocation_ref);
            let ended_ms = match listed {
                Some(row) if row.state == "deleted" => {
                    chrono::DateTime::parse_from_rfc3339(&row.since)
                        .ok()
                        .and_then(|since| u64::try_from(since.timestamp_millis()).ok())
                        .unwrap_or(now)
                }
                Some(_) => continue,
                None => now,
            };
            self.end_acquired_machine(allocation_ref, ended_ms);
        }
    }

    /// Whether any machine acquired without asking is running, so its end needs the service's list.
    pub(crate) fn has_running_acquired_machines(&self) -> bool {
        self.acquired_machine(|machine| matches!(machine.state, AcquiredState::Running { .. }))
            .is_some()
    }

    fn acquired_machine(
        &self,
        matches: impl Fn(&AcquiredMachine) -> bool,
    ) -> Option<AcquiredMachine> {
        self.owned
            .durable_state_store
            .acquired_machines(0)
            .ok()?
            .into_iter()
            .find(matches)
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
            id: "r".into(),
            room_id: "room".into(),
            agent_id: "agent".into(),
            class_id: "small".into(),
            rate_micros: 1_000,
            currency: "EUR".into(),
            started_ms: DAY_MS - MINUTE_MS / 2,
            state,
        };
        let running = machine(AcquiredState::Running {
            allocation_ref: "a".into(),
        });
        assert_eq!(cost_micros(&running, 0, DAY_MS), 1_000);
        // Only today's part counts against today's cap.
        assert_eq!(cost_micros(&running, DAY_MS, DAY_MS + 3 * MINUTE_MS), 3_000);
        let ended = machine(AcquiredState::Ended {
            allocation_ref: "a".into(),
            ended_ms: DAY_MS + 1,
        });
        assert_eq!(cost_micros(&ended, 0, DAY_MS * 2), 1_000);
        assert_eq!(cost_micros(&ended, DAY_MS + 2, DAY_MS * 2), 0);
    }
}
