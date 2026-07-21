use crate::config::{DaemonConfig, PersistedMachineRegistration};
use crate::error::DaemonError;
use arroba_relay::protocol::RelayMachinePresence;

use super::super::api::{RemoteMachineRecord, RemoteMachineTrustStatus};

pub(crate) fn remote_machine_records(
    live_machines: Vec<RelayMachinePresence>,
    local_machine_id: &str,
) -> Vec<RemoteMachineRecord> {
    let registry = DaemonConfig::machine_registry_entries();
    let mut records: Vec<RemoteMachineRecord> = live_machines
        .into_iter()
        .filter_map(|machine| {
            let entry = registry
                .iter()
                .find(|entry| entry.machine_id == machine.machine_id);
            if entry.map(|entry| entry.forgotten).unwrap_or(false) {
                return None;
            }
            Some(remote_machine_record(
                machine,
                entry,
                local_machine_id,
                true,
            ))
        })
        .collect();

    let offline_entries = registry
        .iter()
        .filter(|entry| entry.approved && !entry.forgotten)
        .filter(|entry| {
            !records
                .iter()
                .any(|record| record.machine_id == entry.machine_id)
        })
        .cloned()
        .collect::<Vec<_>>();

    for entry in offline_entries {
        records.push(RemoteMachineRecord {
            machine_id: entry.machine_id.clone(),
            machine_alias: None,
            registry_alias: entry.alias.clone(),
            display_name: entry
                .alias
                .clone()
                .unwrap_or_else(|| entry.machine_id.clone()),
            trust_status: RemoteMachineTrustStatus::Approved,
            online: false,
            pending: false,
            kernel_count: 0,
            available_providers: Vec::new(),
            provider_accounts: Vec::new(),
        });
    }

    records.sort_by_key(|record| {
        (
            !record.online,
            record.pending,
            record.display_name.to_ascii_lowercase(),
            record.machine_id.clone(),
        )
    });
    records
}

fn remote_machine_record(
    machine: RelayMachinePresence,
    entry: Option<&PersistedMachineRegistration>,
    local_machine_id: &str,
    online: bool,
) -> RemoteMachineRecord {
    let approved = machine.machine_id == local_machine_id
        || entry
            .map(|entry| entry.approved && !entry.forgotten)
            .unwrap_or(false);
    let registry_alias = entry.and_then(|entry| entry.alias.clone());
    let display_name = registry_alias
        .clone()
        .or_else(|| machine.machine_alias.clone())
        .unwrap_or_else(|| machine.machine_id.clone());
    RemoteMachineRecord {
        machine_id: machine.machine_id,
        machine_alias: machine.machine_alias,
        registry_alias,
        display_name,
        trust_status: if approved {
            RemoteMachineTrustStatus::Approved
        } else {
            RemoteMachineTrustStatus::Pending
        },
        online,
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

pub(crate) fn forgotten_machine_record(
    machine_id: String,
    registry_alias: Option<String>,
    live_machines: Vec<RelayMachinePresence>,
    local_machine_id: &str,
) -> RemoteMachineRecord {
    let live = live_machines
        .into_iter()
        .find(|machine| machine.machine_id == machine_id && machine.machine_id != local_machine_id);
    let display_name = registry_alias
        .clone()
        .or_else(|| {
            live.as_ref()
                .and_then(|machine| machine.machine_alias.clone())
        })
        .unwrap_or_else(|| machine_id.clone());
    RemoteMachineRecord {
        machine_id,
        machine_alias: live
            .as_ref()
            .and_then(|machine| machine.machine_alias.clone()),
        registry_alias,
        display_name,
        trust_status: RemoteMachineTrustStatus::Forgotten,
        online: live.is_some(),
        pending: false,
        kernel_count: live
            .as_ref()
            .map(|machine| machine.kernel_count)
            .unwrap_or(0),
        available_providers: live
            .as_ref()
            .map(|machine| machine.available_providers.clone())
            .unwrap_or_default(),
        provider_accounts: live
            .map(|machine| machine.provider_accounts)
            .unwrap_or_default(),
    }
}
