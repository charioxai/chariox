//! Path 1 state bundles: the age identity that decrypts one kernel's saved bundles stays in an
//! owner's Vault. This module makes the identity, proves possession of it to Cloud and seals it
//! for one destination kernel. The only dependency on `age` in the kernel lives here.
use crate::error::DaemonError;
use age::secrecy::ExposeSecret;
use age::x25519::{Identity, Recipient};
use age::{Decryptor, Encryptor};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::str::FromStr;
use zeroize::Zeroizing;

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

/// What a seal is bound to. The destination checks it against its own expected launch, never against the item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundleKeyBinding {
    pub key_id: String,
    pub launch_id: String,
    pub source_kernel_id: String,
    pub target_kernel_id: String,
}

/// The bundle key and its binding, as encrypted to the destination.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SealedPayload {
    binding: BundleKeyBinding,
    identity: String,
}

/// An age file, in unpadded URL-safe base64, that only the destination's one-time recipient can open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleKeyMaterialization {
    pub schema_version: u32,
    pub ciphertext: String,
}

/// Hands the key to the one-time age recipient the destination machine made for this restore.
pub fn seal(
    destination_recipient: &str,
    binding: &BundleKeyBinding,
    identity: &str,
) -> Result<BundleKeyMaterialization, DaemonError> {
    parse_identity(identity)?;
    let recipient = Recipient::from_str(destination_recipient)
        .map_err(|_| error("the destination restore key is invalid"))?;
    let payload = Zeroizing::new(
        serde_json::to_vec(&SealedPayload {
            binding: binding.clone(),
            identity: identity.to_string(),
        })
        .map_err(|_| error("the bundle key could not be encoded"))?,
    );
    let encryptor = Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))
        .map_err(|_| error("the destination restore key is invalid"))?;
    let mut ciphertext = Vec::new();
    let mut writer = encryptor
        .wrap_output(&mut ciphertext)
        .map_err(|_| error("the bundle key could not be sealed"))?;
    writer
        .write_all(&payload)
        .and_then(|()| writer.finish().map(|_| ()))
        .map_err(|_| error("the bundle key could not be sealed"))?;
    Ok(BundleKeyMaterialization {
        schema_version: SCHEMA_VERSION,
        ciphertext: URL_SAFE_NO_PAD.encode(ciphertext),
    })
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

/// The owner kernel hands the stored key to one destination machine's one-time restore key, bound
/// to the launch, so a copy of the sealed item names the launch it was made for.
pub(crate) fn seal_from_vault(
    config: &crate::config::DaemonConfig,
    secrets: &crate::secret::RuntimeSecretService,
    owner_user_id: &str,
    key_id: &str,
    launch_id: &str,
    target_kernel_id: &str,
    destination_recipient: &str,
) -> Result<BundleKeyMaterialization, DaemonError> {
    let identity = read_from_vault(secrets, owner_user_id, key_id)?;
    let binding = BundleKeyBinding {
        key_id: key_id.to_string(),
        launch_id: launch_id.to_string(),
        source_kernel_id: config.daemon_id.clone(),
        target_kernel_id: target_kernel_id.to_string(),
    };
    seal(destination_recipient, &binding, &identity)
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
