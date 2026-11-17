//! Which machines are online, reconnecting or offline. The relay says who is connected now and
//! forgets everyone else; this keeps when each machine was last seen, so a dropped connection
//! reads as Reconnecting for a while (unknown is not offline), then Offline, then is forgotten.
use crate::config::PersistedMachineRegistration;
use crate::local::{
    OfflineMachineRecord, RemoteMachineInventory, RemoteMachinePresence, RemoteMachineRecord,
    RemoteMachineTrustStatus,
};

pub(crate) const RECONNECTING_WINDOW_MS: u64 = 10 * 60 * 1000;
pub(crate) const FORGET_AFTER_MS: u64 = 30 * 24 * 60 * 60 * 1000;
/// A machine that stays connected is written to disk this often, which bounds how early its
/// reconnecting window can end after it drops.
const SIGHTING_WRITE_INTERVAL_MS: u64 = 60_000;

/// What a machine that is not connected now is, by how long ago it was last seen.
#[derive(Debug, PartialEq, Eq)]
enum Absence {
    Reconnecting,
    Offline,
    Forgotten,
}

fn absence(last_seen_ms: u64, now_ms: u64) -> Absence {
    let gone_ms = now_ms.saturating_sub(last_seen_ms);
    if gone_ms < RECONNECTING_WINDOW_MS {
        Absence::Reconnecting
    } else if gone_ms <= FORGET_AFTER_MS {
        Absence::Offline
    } else {
        Absence::Forgotten
    }
}

/// Whether a machine or kernel that is not connected now still belongs in a list: it dropped
/// less than the reconnecting window ago.
pub(crate) fn still_reconnecting(last_seen_ms: u64, now_ms: u64) -> bool {
    absence(last_seen_ms, now_ms) == Absence::Reconnecting
}

/// Records the machines connected now (`live`, all online) in `registry`, and sorts every other
/// machine the registry knows into reconnecting, offline or forgotten (removed from `registry`).
/// The host machine is never tracked: this kernel runs on it.
pub(crate) fn reconcile(
    registry: &mut Vec<PersistedMachineRegistration>,
    live: Vec<RemoteMachineRecord>,
    host_machine_id: &str,
    now_ms: u64,
) -> RemoteMachineInventory {
    for machine in live
        .iter()
        .filter(|machine| machine.machine_id != host_machine_id)
    {
        let index = registry
            .iter()
            .position(|entry| entry.machine_id == machine.machine_id)
            .unwrap_or_else(|| {
                registry.push(PersistedMachineRegistration {
                    machine_id: machine.machine_id.clone(),
                    ..PersistedMachineRegistration::default()
                });
                registry.len() - 1
            });
        let entry = &mut registry[index];
        let named = entry.display_name.as_deref() == Some(machine.display_name.as_str());
        if !named || now_ms.saturating_sub(entry.last_seen_ms) >= SIGHTING_WRITE_INTERVAL_MS {
            entry.display_name = Some(machine.display_name.clone());
            entry.last_seen_ms = now_ms;
        }
    }

    let mut machines = live;
    let mut offline_machines = Vec::new();
    registry.retain_mut(|entry| {
        if entry.machine_id == host_machine_id
            || machines
                .iter()
                .any(|machine| machine.machine_id == entry.machine_id)
        {
            return true;
        }
        if entry.last_seen_ms == 0 {
            // Approved or paired before it was ever listed: the clock starts when first missed.
            entry.last_seen_ms = now_ms;
        }
        let display_name = entry
            .display_name
            .clone()
            .unwrap_or_else(|| entry.machine_id.clone());
        match absence(entry.last_seen_ms, now_ms) {
            Absence::Reconnecting => machines.push(RemoteMachineRecord {
                machine_id: entry.machine_id.clone(),
                machine_alias: None,
                display_name,
                trust_status: if entry.approved {
                    RemoteMachineTrustStatus::Approved
                } else {
                    RemoteMachineTrustStatus::Pending
                },
                presence: RemoteMachinePresence::Reconnecting,
                pending: !entry.approved,
                kernel_count: 0,
                available_providers: Vec::new(),
                provider_accounts: Vec::new(),
            }),
            Absence::Offline => offline_machines.push(OfflineMachineRecord {
                machine_id: entry.machine_id.clone(),
                display_name,
                last_seen_ms: entry.last_seen_ms,
            }),
            Absence::Forgotten => return false,
        }
        true
    });
    crate::local::provider_requests::sort_remote_machines(&mut machines);
    offline_machines.sort_by_key(|machine| {
        (
            std::cmp::Reverse(machine.last_seen_ms),
            machine.display_name.to_ascii_lowercase(),
            machine.machine_id.clone(),
        )
    });
    RemoteMachineInventory {
        machines,
        offline_machines,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOST: &str = "host-machine";
    const NOW: u64 = 100 * 24 * 60 * 60 * 1000;

    fn entry(machine_id: &str, last_seen_ms: u64) -> PersistedMachineRegistration {
        PersistedMachineRegistration {
            machine_id: machine_id.into(),
            approved: true,
            last_seen_ms,
            display_name: Some(format!("{machine_id} name")),
            ..PersistedMachineRegistration::default()
        }
    }

    fn live(machine_id: &str) -> RemoteMachineRecord {
        RemoteMachineRecord {
            machine_id: machine_id.into(),
            machine_alias: None,
            display_name: format!("{machine_id} name"),
            trust_status: RemoteMachineTrustStatus::Approved,
            presence: RemoteMachinePresence::Online,
            pending: false,
            kernel_count: 1,
            available_providers: Vec::new(),
            provider_accounts: Vec::new(),
        }
    }

    fn ids(machines: &[RemoteMachineRecord]) -> Vec<(&str, RemoteMachinePresence)> {
        machines
            .iter()
            .map(|machine| (machine.machine_id.as_str(), machine.presence))
            .collect()
    }

    #[test]
    fn a_machine_gone_for_less_than_ten_minutes_is_reconnecting() {
        let mut registry = vec![entry("b", NOW - RECONNECTING_WINDOW_MS + 1)];
        let inventory = reconcile(&mut registry, Vec::new(), HOST, NOW);
        assert_eq!(
            ids(&inventory.machines),
            vec![("b", RemoteMachinePresence::Reconnecting)]
        );
        assert!(inventory.offline_machines.is_empty());
    }

    #[test]
    fn a_machine_gone_for_ten_minutes_is_offline_and_not_listed() {
        let mut registry = vec![entry("b", NOW - RECONNECTING_WINDOW_MS)];
        let inventory = reconcile(&mut registry, Vec::new(), HOST, NOW);
        assert!(inventory.machines.is_empty());
        assert_eq!(
            inventory.offline_machines,
            vec![OfflineMachineRecord {
                machine_id: "b".into(),
                display_name: "b name".into(),
                last_seen_ms: NOW - RECONNECTING_WINDOW_MS,
            }]
        );
    }

    #[test]
    fn a_machine_offline_for_thirty_days_is_forgotten() {
        let mut registry = vec![
            entry("old", NOW - FORGET_AFTER_MS),
            entry("older", NOW - FORGET_AFTER_MS - 1),
        ];
        let inventory = reconcile(&mut registry, Vec::new(), HOST, NOW);
        assert_eq!(
            inventory
                .offline_machines
                .iter()
                .map(|machine| machine.machine_id.as_str())
                .collect::<Vec<_>>(),
            vec!["old"]
        );
        assert_eq!(
            registry
                .iter()
                .map(|entry| entry.machine_id.as_str())
                .collect::<Vec<_>>(),
            vec!["old"]
        );
    }

    #[test]
    fn a_machine_that_reconnects_is_online_again_by_itself() {
        let mut registry = vec![entry("b", NOW - FORGET_AFTER_MS / 2)];
        let inventory = reconcile(&mut registry, vec![live("b")], HOST, NOW);
        assert_eq!(
            ids(&inventory.machines),
            vec![("b", RemoteMachinePresence::Online)]
        );
        assert!(inventory.offline_machines.is_empty());
        assert_eq!(registry[0].last_seen_ms, NOW);
    }

    #[test]
    fn a_machine_seen_for_the_first_time_is_remembered_with_its_name() {
        let mut registry = Vec::new();
        let _ = reconcile(&mut registry, vec![live("new")], HOST, NOW);
        assert_eq!(registry.len(), 1);
        assert_eq!(registry[0].display_name.as_deref(), Some("new name"));
        assert_eq!(registry[0].last_seen_ms, NOW);
        assert!(!registry[0].approved);
    }

    #[test]
    fn an_approved_machine_never_seen_starts_its_clock_when_first_missed() {
        let mut registry = vec![entry("b", 0)];
        let inventory = reconcile(&mut registry, Vec::new(), HOST, NOW);
        assert_eq!(
            ids(&inventory.machines),
            vec![("b", RemoteMachinePresence::Reconnecting)]
        );
        assert_eq!(registry[0].last_seen_ms, NOW);
    }

    #[test]
    fn the_host_machine_is_never_tracked() {
        let mut registry = Vec::new();
        let inventory = reconcile(&mut registry, vec![live(HOST)], HOST, NOW);
        assert!(registry.is_empty());
        assert_eq!(
            ids(&inventory.machines),
            vec![(HOST, RemoteMachinePresence::Online)]
        );
    }

    #[test]
    fn a_connected_machine_is_written_at_most_once_a_minute() {
        let mut registry = vec![entry("b", NOW - 30_000)];
        let _ = reconcile(&mut registry, vec![live("b")], HOST, NOW);
        assert_eq!(registry[0].last_seen_ms, NOW - 30_000);
        let _ = reconcile(&mut registry, vec![live("b")], HOST, NOW + 30_000);
        assert_eq!(registry[0].last_seen_ms, NOW + 30_000);
    }
}
