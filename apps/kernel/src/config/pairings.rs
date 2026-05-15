use super::{
    normalized_optional,
    persisted_daemon::{
        load_persisted_daemon_config, normalized_terminal_type, persist_daemon_config,
        upsert_client_pairing, upsert_machine_registration,
    },
    validate_non_empty, DaemonConfig, PersistedClientPairing, PersistedMachineRegistration,
};
use crate::error::DaemonError;

impl DaemonConfig {
    pub fn machine_registry_entries() -> Vec<PersistedMachineRegistration> {
        load_persisted_daemon_config().machines
    }

    pub fn client_pairing_entries() -> Vec<PersistedClientPairing> {
        load_persisted_daemon_config().clients
    }

    pub fn approve_remote_machine(
        machine_id: impl Into<String>,
        alias: Option<String>,
    ) -> Result<PersistedMachineRegistration, DaemonError> {
        let machine_id = machine_id.into();
        let alias = normalized_optional(alias);
        validate_non_empty("machine_id", &machine_id)?;
        let mut persisted = load_persisted_daemon_config();
        let entry = upsert_machine_registration(&mut persisted.machines, &machine_id);
        entry.alias = alias.or_else(|| entry.alias.clone());
        entry.approved = true;
        entry.forgotten = false;
        let saved = entry.clone();
        persist_daemon_config(&persisted, "persist machine approval")?;
        Ok(saved)
    }

    pub fn pair_remote_machine(
        machine_id: impl Into<String>,
        public_key_thumbprint: impl Into<String>,
        paired_at_ms: u64,
    ) -> Result<PersistedMachineRegistration, DaemonError> {
        let machine_id = machine_id.into();
        let public_key_thumbprint = public_key_thumbprint.into().trim().to_string();
        validate_non_empty("machine_id", &machine_id)?;
        validate_non_empty("public_key_thumbprint", &public_key_thumbprint)?;
        let mut persisted = load_persisted_daemon_config();
        let entry = upsert_machine_registration(&mut persisted.machines, &machine_id);
        entry.public_key_thumbprint = Some(public_key_thumbprint);
        entry.paired_at_ms = Some(paired_at_ms);
        entry.forgotten = false;
        let saved = entry.clone();
        persist_daemon_config(&persisted, "persist machine pairing")?;
        Ok(saved)
    }

    pub fn record_paired_client(
        client_id: impl Into<String>,
        public_key_thumbprint: impl Into<String>,
        alias: Option<String>,
        paired_at_ms: u64,
    ) -> Result<PersistedClientPairing, DaemonError> {
        Self::record_paired_terminal(client_id, public_key_thumbprint, alias, paired_at_ms, "cli")
    }

    pub fn record_paired_terminal(
        client_id: impl Into<String>,
        public_key_thumbprint: impl Into<String>,
        alias: Option<String>,
        paired_at_ms: u64,
        terminal_type: impl Into<String>,
    ) -> Result<PersistedClientPairing, DaemonError> {
        let client_id = client_id.into();
        let public_key_thumbprint = public_key_thumbprint.into().trim().to_string();
        let terminal_type = normalized_terminal_type(&terminal_type.into());
        validate_non_empty("client_id", &client_id)?;
        validate_non_empty("public_key_thumbprint", &public_key_thumbprint)?;
        let mut persisted = load_persisted_daemon_config();
        let entry = upsert_client_pairing(&mut persisted.clients, &client_id);
        entry.alias = normalized_optional(alias).or_else(|| entry.alias.clone());
        entry.public_key_thumbprint = public_key_thumbprint;
        entry.terminal_type = terminal_type;
        entry.paired_at_ms = paired_at_ms;
        entry.revoked = false;
        let saved = entry.clone();
        persist_daemon_config(&persisted, "persist client pairing")?;
        Ok(saved)
    }

    pub fn revoke_paired_client(
        client_id: impl Into<String>,
    ) -> Result<PersistedClientPairing, DaemonError> {
        let client_id = client_id.into();
        validate_non_empty("client_id", &client_id)?;
        let mut persisted = load_persisted_daemon_config();
        let entry = upsert_client_pairing(&mut persisted.clients, &client_id);
        entry.revoked = true;
        let saved = entry.clone();
        persist_daemon_config(&persisted, "persist client revocation")?;
        Ok(saved)
    }

    pub fn forget_remote_machine(
        machine_id: impl Into<String>,
    ) -> Result<PersistedMachineRegistration, DaemonError> {
        let machine_id = machine_id.into();
        validate_non_empty("machine_id", &machine_id)?;
        let mut persisted = load_persisted_daemon_config();
        let entry = upsert_machine_registration(&mut persisted.machines, &machine_id);
        entry.approved = false;
        entry.forgotten = true;
        let saved = entry.clone();
        persist_daemon_config(&persisted, "persist forgotten machine")?;
        Ok(saved)
    }

    pub fn rename_remote_machine(
        machine_id: impl Into<String>,
        alias: impl Into<String>,
    ) -> Result<PersistedMachineRegistration, DaemonError> {
        let machine_id = machine_id.into();
        let alias = alias.into().trim().to_string();
        validate_non_empty("machine_id", &machine_id)?;
        validate_non_empty("machine_alias", &alias)?;
        let mut persisted = load_persisted_daemon_config();
        let entry = upsert_machine_registration(&mut persisted.machines, &machine_id);
        entry.alias = Some(alias);
        entry.approved = true;
        entry.forgotten = false;
        let saved = entry.clone();
        persist_daemon_config(&persisted, "persist machine rename")?;
        Ok(saved)
    }

    pub fn resolve_registered_machine_ref(machine_ref: &str) -> Option<String> {
        let machine_ref = machine_ref.trim();
        if machine_ref.is_empty() {
            return None;
        }
        Self::machine_registry_entries()
            .into_iter()
            .filter(|entry| !entry.forgotten)
            .find(|entry| {
                entry.machine_id == machine_ref || entry.alias.as_deref() == Some(machine_ref)
            })
            .map(|entry| entry.machine_id)
    }
}
