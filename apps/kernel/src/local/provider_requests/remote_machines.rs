use crate::config::{DaemonConfig, PersistedMachineRegistration};
use crate::error::DaemonError;
use chariox_relay::protocol::RelayMachinePresence;

use super::super::api::{RemoteMachinePresence, RemoteMachineRecord, RemoteMachineTrustStatus};

/// The machines the relay lists now, all online.
pub(crate) fn remote_machine_records(
    live_machines: Vec<RelayMachinePresence>,
    local_machine_id: &str,
) -> Vec<RemoteMachineRecord> {
    let registry = DaemonConfig::machine_registry_entries();
    let mut records: Vec<RemoteMachineRecord> = live_machines
        .into_iter()
        .map(|machine| {
            let entry = registry
                .iter()
                .find(|entry| entry.machine_id == machine.machine_id);
            remote_machine_record(machine, entry, local_machine_id)
        })
        .collect();
    sort_remote_machines(&mut records);
    records
}

/// Listed machines first by how reachable they are, then approved before pending, then by name.
pub(crate) fn sort_remote_machines(records: &mut [RemoteMachineRecord]) {
    records.sort_by_key(|record| {
        (
            record.presence != RemoteMachinePresence::Online,
            record.pending,
            record.display_name.to_ascii_lowercase(),
            record.machine_id.clone(),
        )
    });
}

fn remote_machine_record(
    machine: RelayMachinePresence,
    entry: Option<&PersistedMachineRegistration>,
    local_machine_id: &str,
) -> RemoteMachineRecord {
    let approved = machine.machine_id == local_machine_id
        || entry.map(|entry| entry.approved).unwrap_or(false);
    let display_name = machine
        .machine_alias
        .clone()
        .unwrap_or_else(|| machine.machine_id.clone());
    RemoteMachineRecord {
        machine_id: machine.machine_id,
        machine_alias: machine.machine_alias,
        display_name,
        trust_status: if approved {
            RemoteMachineTrustStatus::Approved
        } else {
            RemoteMachineTrustStatus::Pending
        },
        presence: RemoteMachinePresence::Online,
        pending: !approved,
        kernel_count: machine.kernel_count,
        available_providers: machine.available_providers,
        provider_accounts: machine.provider_accounts,
    }
}

pub(crate) fn resolve_registered_or_raw_machine_ref(machine_ref: &str) -> String {
    DaemonConfig::resolve_registered_machine_ref(machine_ref)
        .unwrap_or_else(|| machine_ref.trim().to_string())
}

pub(crate) fn resolve_machine_for_registry(
    machine_ref: &str,
    live_machines: &[RelayMachinePresence],
) -> Result<RelayMachinePresence, DaemonError> {
    let machine_ref = machine_ref.trim();
    live_machines
        .iter()
        .find(|machine| {
            machine.machine_id == machine_ref
                || machine.machine_alias.as_deref() == Some(machine_ref)
        })
        .cloned()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "resolve remote machine",
            message: format!("no live remote machine found for `{machine_ref}`"),
        })
}

pub(crate) fn resolve_machine_id_for_registry(
    machine_ref: &str,
    live_machines: &[RelayMachinePresence],
) -> Result<String, DaemonError> {
    if let Some(machine_id) = DaemonConfig::resolve_registered_machine_ref(machine_ref) {
        return Ok(machine_id);
    }
    resolve_machine_for_registry(machine_ref, live_machines).map(|machine| machine.machine_id)
}

pub(crate) fn record_for_machine_id(
    machine_id: String,
    live_machines: Vec<RelayMachinePresence>,
    local_machine_id: &str,
) -> Result<RemoteMachineRecord, DaemonError> {
    remote_machine_records(live_machines, local_machine_id)
        .into_iter()
        .find(|machine| machine.machine_id == machine_id)
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "load remote machine record",
            message: format!("remote machine `{machine_id}` is not visible"),
        })
}

/// The name to answer a removed machine with: the one it had when last seen, else the live
/// alias, else its id.
pub(crate) fn forgotten_machine_name(
    machine_id: &str,
    removed: Option<PersistedMachineRegistration>,
    live_machines: &[RelayMachinePresence],
) -> String {
    removed
        .and_then(|entry| entry.display_name)
        .or_else(|| {
            live_machines
                .iter()
                .find(|machine| machine.machine_id == machine_id)
                .and_then(|machine| machine.machine_alias.clone())
        })
        .unwrap_or_else(|| machine_id.to_string())
}
