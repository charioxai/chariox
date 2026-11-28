//! A redacted explanation of the reserve transaction, never a second admission calculation.
use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{super::pool::MemoryRoom, Pool, Reservation};

const DOMAIN_LIMIT: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct MemoryAccounting {
    sampled_at_ms: u64,
    pool_ceiling_bytes: u64,
    margin_bytes: u64,
    room_bytes: u64,
    host_bound: bool,
    declared_bytes: u64,
    learned_peak_bytes: u64,
    need_bytes: u64,
    free_bytes: u64,
    predicted_growth_bytes: u64,
    launch_remaining_bytes: u64,
    hold_cause: &'static str,
    waiting_unit_id: String,
    domains: Vec<DomainMemory>,
    omitted_domains: usize,
    omitted_launch_remaining_bytes: u64,
    omitted_predicted_growth_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct DomainMemory {
    /// Digest of an internal unit key, never a process command, PID, path or user identity.
    unit_id: String,
    has_launch_reservation: bool,
    observed_domain: bool,
    declared_bytes: u64,
    launch_reserved_bytes: u64,
    observed_bytes: u64,
    launch_remaining_bytes: u64,
    predicted_growth_bytes: u64,
}

pub(super) struct Decision<'a> {
    pub id: &'a str,
    pub declared: u64,
    pub learned: u64,
    pub need: u64,
    pub free: u64,
}

impl MemoryAccounting {
    pub(super) fn capture(
        pool: &Pool,
        room: &MemoryRoom,
        launches: &BTreeMap<String, Reservation>,
        remaining: &BTreeMap<String, u64>,
        growth: &BTreeMap<String, u64>,
        decision: Decision<'_>,
    ) -> Self {
        let units: BTreeSet<_> = launches
            .keys()
            .chain(room.observed.keys())
            .chain(growth.keys())
            .collect();
        let mut domains: Vec<_> = units
            .into_iter()
            .map(|unit| {
                let launch = launches.get(unit);
                DomainMemory {
                    unit_id: opaque(unit),
                    has_launch_reservation: launch.is_some(),
                    observed_domain: room.observed.contains_key(unit),
                    declared_bytes: launch.map_or(0, |launch| launch.declared_memory),
                    launch_reserved_bytes: launch.map_or(0, |launch| launch.memory),
                    observed_bytes: room.observed.get(unit).copied().unwrap_or(0),
                    launch_remaining_bytes: remaining.get(unit).copied().unwrap_or(0),
                    predicted_growth_bytes: growth.get(unit).copied().unwrap_or(0),
                }
            })
            .collect();
        domains.sort_by(|a, b| {
            let weight = |domain: &DomainMemory| {
                domain
                    .launch_remaining_bytes
                    .saturating_add(domain.predicted_growth_bytes)
            };
            weight(b)
                .cmp(&weight(a))
                .then_with(|| a.unit_id.cmp(&b.unit_id))
        });
        let omitted = domains.get(DOMAIN_LIMIT..).unwrap_or_default();
        let omitted_domains = omitted.len();
        let omitted_launch_remaining_bytes =
            sum(omitted.iter().map(|domain| domain.launch_remaining_bytes));
        let omitted_predicted_growth_bytes =
            sum(omitted.iter().map(|domain| domain.predicted_growth_bytes));
        domains.truncate(DOMAIN_LIMIT);
        Self {
            sampled_at_ms: crate::session::unix_epoch_ms(),
            pool_ceiling_bytes: pool.ceiling,
            margin_bytes: pool.ceiling.saturating_sub(pool.capacity()),
            room_bytes: room.memory,
            host_bound: room.host_bound,
            declared_bytes: decision.declared,
            learned_peak_bytes: decision.learned,
            need_bytes: decision.need,
            free_bytes: decision.free,
            predicted_growth_bytes: sum(growth.values().copied()),
            launch_remaining_bytes: sum(remaining.values().copied()),
            // "Headroom" does not assert OS pressure: a large request can exceed it on a healthy host.
            hold_cause: if room.memory >= decision.need {
                "reserved_capacity"
            } else {
                "physical_headroom"
            },
            waiting_unit_id: opaque(decision.id),
            domains,
            omitted_domains,
            omitted_launch_remaining_bytes,
            omitted_predicted_growth_bytes,
        }
    }
}

fn sum(values: impl Iterator<Item = u64>) -> u64 {
    values.fold(0, u64::saturating_add)
}

fn opaque(unit: &str) -> String {
    format!("unit-{:x}", Sha256::digest(unit.as_bytes()))
}
