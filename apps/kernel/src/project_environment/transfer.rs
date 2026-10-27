//! MP-08: Selected Project values are sealed directly for a target kernel.
//! No full-Vault transfer or cross-kernel synchronization is required.
use super::resolver::environment_error;
use super::*;
use crate::error::DaemonError;
use crate::secret::CredentialVaultStore;
use crate::transport::relay_crypto;
use chariox_relay::protocol::EncryptedRelayPayload;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use zeroize::Zeroizing;

const PURPOSE: &[u8] = b"chariox-project-environment-v1";
const MAX_VALUE_BYTES: usize = 1024 * 1024;
const MAX_LAYER_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectEnvironmentTransferBinding {
    pub context_id: String,
    pub source_kernel_id: String,
    pub target_kernel_id: String,
    pub project_id: String,
    pub manifest_sha256: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SealedProjectEnvironment {
    pub binding: ProjectEnvironmentTransferBinding,
    pub manifest: ProjectEnvironmentManifest,
    pub values: EncryptedRelayPayload,
}

// Plaintext wire values exist only inside Zeroizing storage at the encryption boundary.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireValue {
    workspace_id: String,
    name: String,
    value: String,
}
impl Drop for WireValue {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.value.zeroize();
    }
}

fn manifest_sha256(manifest: &ProjectEnvironmentManifest) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(manifest).expect("manifest encodes"))
    )
}
fn validate_binding(
    binding: &ProjectEnvironmentTransferBinding,
    manifest: &ProjectEnvironmentManifest,
) -> Result<Vec<u8>, DaemonError> {
    manifest.validate().map_err(environment_error)?;
    if binding.project_id != manifest.project_id
        || binding.manifest_sha256 != manifest_sha256(manifest)
        || [
            &binding.context_id,
            &binding.source_kernel_id,
            &binding.target_kernel_id,
        ]
        .iter()
        .any(|id| id.is_empty() || id.len() > 256 || id.contains(['\0', '\n', '\r']))
    {
        return Err(environment_error(
            "Project environment transfer binding mismatch",
        ));
    }
    serde_json::to_vec(binding)
        .map_err(|_| environment_error("invalid environment transfer binding"))
}

pub fn seal_project_environment(
    context_id: &str,
    source_kernel_id: &str,
    target_kernel_id: &str,
    source_private_key: &str,
    target_public_key: &str,
    manifest: &ProjectEnvironmentManifest,
    resolved: &ResolvedProjectEnvironment,
) -> Result<SealedProjectEnvironment, DaemonError> {
    // MP-08 / MP-10: skipped inputs remain metadata; only resolved values are sealed.
    let mut manifest = manifest.clone();
    for entry in &mut manifest.entries {
        entry.status = if resolved
            .values
            .contains_key(&(entry.workspace_id.clone(), entry.name.clone()))
        {
            ProjectEnvironmentEntryStatus::Found
        } else {
            resolved
                .unresolved
                .iter()
                .find(|missing| {
                    missing.workspace_id == entry.workspace_id && missing.name == entry.name
                })
                .ok_or_else(|| environment_error("Project environment resolution incomplete"))?
                .status
        };
    }
    let binding = ProjectEnvironmentTransferBinding {
        context_id: context_id.into(),
        source_kernel_id: source_kernel_id.into(),
        target_kernel_id: target_kernel_id.into(),
        project_id: manifest.project_id.clone(),
        manifest_sha256: manifest_sha256(&manifest),
    };
    let aad = validate_binding(&binding, &manifest)?;
    let mut values = Vec::new();
    let mut total = 0;
    for entry in manifest
        .entries
        .iter()
        .filter(|entry| entry.status == ProjectEnvironmentEntryStatus::Found)
    {
        let value = resolved
            .values
            .get(&(entry.workspace_id.clone(), entry.name.clone()))
            .ok_or_else(|| environment_error("Project environment resolution incomplete"))?;
        total += value.len();
        if value.len() > MAX_VALUE_BYTES || total > MAX_LAYER_BYTES {
            return Err(environment_error(
                "Project environment transfer exceeds bounds",
            ));
        }
        values.push(WireValue {
            workspace_id: entry.workspace_id.clone(),
            name: entry.name.clone(),
            value: value.to_string(),
        });
    }
    let plaintext = Zeroizing::new(
        serde_json::to_vec(&values).map_err(|_| environment_error("environment sealing failed"))?,
    );
    let encrypted = relay_crypto::encrypt_payload_for_peer_bound(
        source_private_key,
        target_public_key,
        PURPOSE,
        &aad,
        &plaintext,
    )?;
    Ok(SealedProjectEnvironment {
        binding,
        manifest: manifest.clone(),
        values: encrypted,
    })
}

/// The M28 caller supplies authenticated identity and context bindings, never package claims.
pub fn unseal_project_environment(
    layer: &SealedProjectEnvironment,
    expected_binding: &ProjectEnvironmentTransferBinding,
    target_private_key: &str,
    expected_source_public_key: &str,
    vault: &dyn CredentialVaultStore,
) -> Result<ResolvedProjectEnvironment, DaemonError> {
    if &layer.binding != expected_binding
        || layer.values.sender_public_key != expected_source_public_key
    {
        return Err(environment_error(
            "Project environment source or target mismatch",
        ));
    }
    let aad = validate_binding(&layer.binding, &layer.manifest)?;
    relay_crypto::validate_encrypted_payload_shape(&layer.values, MAX_LAYER_BYTES * 2)?;
    let decrypted = relay_crypto::decrypt_payload_for_private_key_bound(
        target_private_key,
        &layer.values,
        PURPOSE,
        &aad,
    )?;
    if decrypted.plaintext.len() > MAX_LAYER_BYTES * 2 {
        return Err(environment_error("environment plaintext exceeds bounds"));
    }
    let values: Vec<WireValue> = serde_json::from_slice(&decrypted.plaintext)
        .map_err(|_| environment_error("invalid sealed Project environment"))?;
    let mut resolved = ResolvedProjectEnvironment {
        values: BTreeMap::new(),
        unresolved: Vec::new(),
    };
    for wire in &values {
        if wire.value.is_empty()
            || wire.value.len() > MAX_VALUE_BYTES
            || wire.value.contains('\0')
            || resolved
                .values
                .insert(
                    (wire.workspace_id.clone(), wire.name.clone()),
                    Zeroizing::new(wire.value.clone()),
                )
                .is_some()
        {
            return Err(environment_error(
                "invalid or duplicate sealed environment value",
            ));
        }
    }
    if resolved.values.len()
        != layer
            .manifest
            .entries
            .iter()
            .filter(|entry| entry.status == ProjectEnvironmentEntryStatus::Found)
            .count()
        || layer.manifest.entries.iter().any(|entry| {
            (entry.status == ProjectEnvironmentEntryStatus::Found)
                != resolved
                    .values
                    .contains_key(&(entry.workspace_id.clone(), entry.name.clone()))
        })
    {
        return Err(environment_error(
            "sealed environment entries do not match manifest",
        ));
    }
    resolved.unresolved = layer
        .manifest
        .entries
        .iter()
        .filter(|entry| entry.status != ProjectEnvironmentEntryStatus::Found)
        .cloned()
        .collect();
    // Validate the whole payload before mutating the target Vault.
    let service = project_environment_vault_service(&layer.manifest.project_id);
    for entry in layer
        .manifest
        .entries
        .iter()
        .filter(|entry| entry.status == ProjectEnvironmentEntryStatus::Found)
    {
        let value = &resolved.values[&(entry.workspace_id.clone(), entry.name.clone())];
        vault.set_secret(&service, &project_environment_vault_key(entry), value)?;
    }
    Ok(resolved)
}
