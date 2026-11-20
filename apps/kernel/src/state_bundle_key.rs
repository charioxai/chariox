//! Path 1 state bundles: the age identity that decrypts one kernel's saved bundles stays in an
//! owner's Vault. This module makes the identity, proves possession of it to Cloud and seals it
//! for one destination kernel. The only dependency on `age` in the kernel lives here.
use crate::error::DaemonError;
use crate::transport::relay_crypto;
use age::secrecy::ExposeSecret;
use age::x25519::{Identity, Recipient};
use age::Decryptor;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use chariox_relay::protocol::EncryptedRelayPayload;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::str::FromStr;
use zeroize::Zeroizing;

const PURPOSE: &[u8] = b"chariox-state-bundle-key-v1";
const SCHEMA_VERSION: u32 = 1;
const MAX_CHALLENGE_BYTES: usize = 4096;

/// A new identity: the secret for the Vault and the recipient a source machine encrypts to.
pub struct NewBundleKey {
    pub identity: Zeroizing<String>,
    pub recipient: String,
}

pub fn generate() -> NewBundleKey {
    let identity = Identity::generate();
    NewBundleKey {
        recipient: identity.to_public().to_string(),
        identity: Zeroizing::new(identity.to_string().expose_secret().to_owned()),
    }
}

/// The public recipient of a stored identity, so a Vault item can be listed without its secret.
pub fn recipient_of(identity: &str) -> Result<String, DaemonError> {
    Ok(parse_identity(identity)?.to_public().to_string())
}

/// Decrypts Cloud's recovery challenge, which is an age file in unpadded URL-safe base64.
/// The answer is the proof, sent with this kernel's own Cloud credential.
pub fn answer_challenge(
    identity: &str,
    ciphertext: &str,
) -> Result<Zeroizing<String>, DaemonError> {
    let identity = parse_identity(identity)?;
    let bytes = URL_SAFE_NO_PAD
        .decode(ciphertext)
        .ok()
        .filter(|bytes| bytes.len() <= MAX_CHALLENGE_BYTES)
        .ok_or_else(|| error("the recovery challenge is malformed"))?;
    let decryptor =
        Decryptor::new(&bytes[..]).map_err(|_| error("the recovery challenge is malformed"))?;
    let mut reader = decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .map_err(|_| error("this Vault does not hold the bundle key"))?;
    let mut answer = Zeroizing::new(Vec::new());
    reader
        .by_ref()
        .take(MAX_CHALLENGE_BYTES as u64)
        .read_to_end(&mut answer)
        .map_err(|_| error("the recovery challenge could not be read"))?;
    String::from_utf8(std::mem::take(&mut *answer))
        .map(Zeroizing::new)
        .map_err(|_| error("the recovery challenge could not be read"))
}

/// A bundle key sealed from the owner kernel's relay key to the destination's.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SealedBundleKey(EncryptedRelayPayload);

impl std::fmt::Debug for SealedBundleKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SealedBundleKey([SEALED])")
    }
}

/// What a seal is bound to. The destination builds it from its own expected launch, never from the item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleKeyBinding {
    pub key_id: String,
    pub launch_id: String,
    pub source_kernel_id: String,
    pub target_kernel_id: String,
}

impl BundleKeyBinding {
    fn aad(&self) -> Result<Vec<u8>, DaemonError> {
        serde_json::to_vec(self).map_err(|_| error("the bundle key binding could not be encoded"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleKeyMaterialization {
    pub schema_version: u32,
    pub sealed: SealedBundleKey,
}

pub fn seal(
    source_private_key: &str,
    target_public_key: &str,
    binding: &BundleKeyBinding,
    identity: &str,
) -> Result<BundleKeyMaterialization, DaemonError> {
    parse_identity(identity)?;
    let sealed = relay_crypto::encrypt_payload_for_peer_bound(
        source_private_key,
        target_public_key,
        PURPOSE,
        &binding.aad()?,
        identity.as_bytes(),
    )?;
    Ok(BundleKeyMaterialization {
        schema_version: SCHEMA_VERSION,
        sealed: SealedBundleKey(sealed),
    })
}

pub fn open(
    target_private_key: &str,
    source_key_thumbprint: &str,
    binding: &BundleKeyBinding,
    key: &BundleKeyMaterialization,
) -> Result<Zeroizing<String>, DaemonError> {
    if key.schema_version != SCHEMA_VERSION {
        return Err(error("the bundle key does not match the launch"));
    }
    if crate::runtime::terminal_pairings::public_key_thumbprint(&key.sealed.0.sender_public_key)
        != source_key_thumbprint
    {
        return Err(error("the bundle key was not sealed by the owner's kernel"));
    }
    let opened = relay_crypto::decrypt_payload_for_private_key_bound(
        target_private_key,
        &key.sealed.0,
        PURPOSE,
        &binding.aad()?,
    )?;
    let mut plaintext = opened.plaintext;
    let identity = String::from_utf8(std::mem::take(&mut *plaintext))
        .map(Zeroizing::new)
        .map_err(|_| error("the bundle key could not be read"))?;
    parse_identity(&identity)?;
    Ok(identity)
}

/// The Vault slot of one owner's bundle key. Handle-less: no agent can name or read it.
fn vault_slot(owner_user_id: &str, key_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("{owner_user_id}\0{key_id}").as_bytes());
    format!("state-bundle-key-{digest:x}")
}

pub fn validate_key_id(key_id: &str) -> Result<(), DaemonError> {
    if (1..=160).contains(&key_id.len())
        && key_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        Ok(())
    } else {
        Err(error("the bundle key id is invalid"))
    }
}

/// Stores a new identity, or returns the recipient of the one already stored under this id,
/// so a repeated request does not replace a key that saved bundles depend on.
pub(crate) fn create_in_vault(
    secrets: &crate::secret::RuntimeSecretService,
    owner_user_id: &str,
    key_id: &str,
) -> Result<String, DaemonError> {
    validate_key_id(key_id)?;
    let slot = vault_slot(owner_user_id, key_id);
    if let Some(existing) = secrets.internal_vault_secret(&slot)? {
        return recipient_of(&existing);
    }
    let key = generate();
    secrets.set_vault_secret(&slot, &key.identity)?;
    Ok(key.recipient)
}

pub(crate) fn read_from_vault(
    secrets: &crate::secret::RuntimeSecretService,
    owner_user_id: &str,
    key_id: &str,
) -> Result<Zeroizing<String>, DaemonError> {
    validate_key_id(key_id)?;
    secrets
        .internal_vault_secret(&vault_slot(owner_user_id, key_id))?
        .ok_or_else(|| error("this Vault does not hold the bundle key"))
}

#[derive(Debug, Deserialize)]
pub(crate) struct VerifiedRecovery {
    pub(crate) recovery_id: String,
    pub(crate) verified_at: String,
}

/// Decrypts Cloud's challenge with the stored key and presents the answer with this kernel's own
/// enrollment. Cloud accepts it only from a kernel independent of the machine being saved.
pub(crate) async fn answer_to_cloud(
    config: &crate::config::DaemonConfig,
    secrets: &crate::secret::RuntimeSecretService,
    owner_user_id: &str,
    key_id: &str,
    recovery_id: &str,
    ciphertext: &str,
) -> Result<VerifiedRecovery, DaemonError> {
    let identity = read_from_vault(secrets, owner_user_id, key_id)?;
    let answer = answer_challenge(&identity, ciphertext)?;
    crate::runtime::control_plane::post_enrolled(
        config,
        serde_json::json!({
            "action": "verify_state_bundle_key",
            "recovery_id": recovery_id,
            "answer": &*answer,
        }),
    )
    .await
}

fn parse_identity(identity: &str) -> Result<Identity, DaemonError> {
    Identity::from_str(identity.trim()).map_err(|_| error("the bundle key is not an age identity"))
}

/// The public recipient is the only part a source machine ever holds.
pub fn validate_recipient(recipient: &str) -> Result<(), DaemonError> {
    Recipient::from_str(recipient)
        .map(|_| ())
        .map_err(|_| error("the bundle recipient is invalid"))
}

fn error(message: &'static str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "state bundle key",
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests;
