//! MP-08 / MP-09 / MP-10 / MP-11: a provider login held in the owner's Vault, sealed for
//! one managed-context target. The package carries only ciphertext; the target opens it
//! with its own relay key and writes the value into its own Vault.
use crate::account_profile::ProviderAccountReplicaMetadata;
use crate::error::DaemonError;
use crate::transport::relay_crypto;
use chariox_relay::protocol::EncryptedRelayPayload;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

const PURPOSE: &[u8] = b"chariox-provider-vault-login-v1";
const SCHEMA_VERSION: u32 = 1;

/// A login sealed from the source kernel's relay key to the target's. Debug never prints it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SealedVaultLogin(EncryptedRelayPayload);

impl std::fmt::Debug for SealedVaultLogin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SealedVaultLogin([SEALED])")
    }
}

/// One selected profile whose login is a Vault credential rather than provider-native files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VaultLoginMaterialization {
    pub schema_version: u32,
    pub profile: ProviderAccountReplicaMetadata,
    pub sealed: SealedVaultLogin,
    pub generated_at_ms: u64,
}

/// What a seal is bound to. The target builds it from its own expected binding, never from the item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VaultLoginBinding {
    pub context_id: String,
    pub source_kernel_id: String,
    pub target_kernel_id: String,
    pub provider: String,
    pub profile_id: String,
}

impl VaultLoginBinding {
    fn aad(&self) -> Result<Vec<u8>, DaemonError> {
        serde_json::to_vec(self).map_err(|_| error("the Vault login binding could not be encoded"))
    }
}

pub(crate) fn seal(
    source_private_key: &str,
    target_public_key: &str,
    binding: &VaultLoginBinding,
    profile: ProviderAccountReplicaMetadata,
    secret: &str,
) -> Result<VaultLoginMaterialization, DaemonError> {
    if profile.provider != binding.provider || profile.profile_id != binding.profile_id {
        return Err(error("the Vault login names another profile"));
    }
    let sealed = relay_crypto::encrypt_payload_for_peer_bound(
        source_private_key,
        target_public_key,
        PURPOSE,
        &binding.aad()?,
        secret.as_bytes(),
    )?;
    Ok(VaultLoginMaterialization {
        schema_version: SCHEMA_VERSION,
        profile,
        sealed: SealedVaultLogin(sealed),
        generated_at_ms: crate::session::unix_epoch_ms(),
    })
}

pub(crate) fn open(
    target_private_key: &str,
    source_key_thumbprint: &str,
    binding: &VaultLoginBinding,
    login: &VaultLoginMaterialization,
) -> Result<Zeroizing<String>, DaemonError> {
    if login.schema_version != SCHEMA_VERSION
        || login.profile.provider != binding.provider
        || login.profile.profile_id != binding.profile_id
    {
        return Err(error("the Vault login does not match the launch plan"));
    }
    if crate::runtime::terminal_pairings::public_key_thumbprint(&login.sealed.0.sender_public_key)
        != source_key_thumbprint
    {
        return Err(error("the Vault login was not sealed by the source kernel"));
    }
    let opened = relay_crypto::decrypt_payload_for_private_key_bound(
        target_private_key,
        &login.sealed.0,
        PURPOSE,
        &binding.aad()?,
    )?;
    let mut plaintext = opened.plaintext;
    String::from_utf8(std::mem::take(&mut *plaintext))
        .map(Zeroizing::new)
        .map_err(|_| error("the Vault login could not be read"))
}

/// Digest of the sealed item, which is all the receipt records of it.
pub(crate) fn materialization_sha256(
    login: &VaultLoginMaterialization,
) -> Result<String, DaemonError> {
    use sha2::{Digest, Sha256};
    let bytes =
        serde_json::to_vec(login).map_err(|_| error("the Vault login could not be encoded"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn error(message: &'static str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "provider Vault login",
        message: message.to_string(),
    }
}
