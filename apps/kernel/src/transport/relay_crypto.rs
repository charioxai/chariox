use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::Engine;
use chariox_relay::protocol::EncryptedRelayPayload;
use hkdf::Hkdf;
use p256::ecdh::diffie_hellman;
use p256::elliptic_curve::sec1::{FromEncodedPoint, ToEncodedPoint};
use p256::{EncodedPoint, PublicKey, SecretKey};
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::error::DaemonError;

const RELAY_INFO: &[u8] = b"chariox-relay-v1";
const BOUND_INFO_PREFIX: &[u8] = b"chariox-bound-v1\0";
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
const MAX_ENCODED_PUBLIC_KEY_BYTES: usize = 256;
const PEER_CRYPTO_CONTEXT_CACHE_LIMIT: usize = 256;

static PEER_CRYPTO_CONTEXTS: OnceLock<Mutex<HashMap<[u8; 32], Arc<PeerCryptoContext>>>> =
    OnceLock::new();

#[derive(Debug)]
struct PeerCryptoContext {
    key: [u8; 32],
    local_public_key: String,
}

#[derive(Debug, Clone)]
pub struct DecryptedRelayPayload {
    pub plaintext: Vec<u8>,
    pub sender_public_key: String,
}

pub fn decrypt_payload_for_private_key(
    private_key_base64: &str,
    payload: &EncryptedRelayPayload,
) -> Result<DecryptedRelayPayload, DaemonError> {
    let context = peer_crypto_context(private_key_base64, &payload.sender_public_key)?;
    let nonce_bytes = decode_bytes(&payload.nonce, "decode relay nonce")?;
    if nonce_bytes.len() != NONCE_LEN {
        return Err(relay_crypto_error(
            "decode relay nonce",
            "nonce must be 12 bytes",
        ));
    }
    let ciphertext = decode_bytes(&payload.ciphertext, "decode relay ciphertext")?;
    if ciphertext.len() < TAG_LEN {
        return Err(relay_crypto_error(
            "decode relay ciphertext",
            "ciphertext must include authentication tag",
        ));
    }
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&context.key));
    let plaintext = cipher
        .decrypt(Nonce::from_slice(&nonce_bytes), ciphertext.as_ref())
        .map_err(|error| relay_crypto_error("decrypt relay payload", &error.to_string()))?;
    Ok(DecryptedRelayPayload {
        plaintext,
        sender_public_key: payload.sender_public_key.clone(),
    })
}

pub fn encrypt_payload_for_peer(
    private_key_base64: &str,
    peer_public_key_base64: &str,
    plaintext: &[u8],
) -> Result<EncryptedRelayPayload, DaemonError> {
    let context = peer_crypto_context(private_key_base64, peer_public_key_base64)?;
    let mut nonce = [0_u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&context.key));
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext)
        .map_err(|error| relay_crypto_error("encrypt relay payload", &error.to_string()))?;
    Ok(EncryptedRelayPayload {
        sender_public_key: context.local_public_key.clone(),
        nonce: base64::engine::general_purpose::STANDARD.encode(nonce),
        ciphertext: base64::engine::general_purpose::STANDARD.encode(ciphertext),
    })
}

pub(crate) fn encrypt_payload_for_peer_bound(
    private_key_base64: &str,
    peer_public_key_base64: &str,
    purpose: &[u8],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<EncryptedRelayPayload, DaemonError> {
    let private_key = decode_private_key(private_key_base64)?;
    let peer_public_key = decode_public_key(peer_public_key_base64)?;
    let key = derive_bound_symmetric_key(&private_key, &peer_public_key, purpose)?;
    let mut nonce = [0_u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|error| relay_crypto_error("encrypt bound peer payload", &error.to_string()))?;
    Ok(EncryptedRelayPayload {
        sender_public_key: encode_public_key(&private_key.public_key()),
        nonce: base64::engine::general_purpose::STANDARD.encode(nonce),
        ciphertext: base64::engine::general_purpose::STANDARD.encode(ciphertext),
    })
}

pub(crate) fn decrypt_payload_for_private_key_bound(
    private_key_base64: &str,
    payload: &EncryptedRelayPayload,
    purpose: &[u8],
    aad: &[u8],
) -> Result<DecryptedRelayPayload, DaemonError> {
    let private_key = decode_private_key(private_key_base64)?;
    let sender_public_key = decode_public_key(&payload.sender_public_key)?;
    let key = derive_bound_symmetric_key(&private_key, &sender_public_key, purpose)?;
    let nonce = decode_bytes(&payload.nonce, "decode bound peer nonce")?;
    if nonce.len() != NONCE_LEN {
        return Err(relay_crypto_error(
            "decode bound peer nonce",
            "nonce must be 12 bytes",
        ));
    }
    let ciphertext = decode_bytes(&payload.ciphertext, "decode bound peer ciphertext")?;
    if ciphertext.len() < TAG_LEN {
        return Err(relay_crypto_error(
            "decode bound peer ciphertext",
            "ciphertext must include authentication tag",
        ));
    }
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let plaintext = cipher
        .decrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &ciphertext,
                aad,
            },
        )
        .map_err(|error| relay_crypto_error("decrypt bound peer payload", &error.to_string()))?;
    Ok(DecryptedRelayPayload {
        plaintext,
        sender_public_key: payload.sender_public_key.clone(),
    })
}

pub(crate) fn validate_encrypted_payload_shape(
    payload: &EncryptedRelayPayload,
    maximum_ciphertext_bytes: usize,
) -> Result<(), DaemonError> {
    if payload.sender_public_key.len() > MAX_ENCODED_PUBLIC_KEY_BYTES {
        return Err(relay_crypto_error(
            "validate encrypted relay payload",
            "sender public key exceeds its size limit",
        ));
    }
    decode_public_key(&payload.sender_public_key)?;
    if payload.nonce.len() > maximum_base64_length(NONCE_LEN) {
        return Err(relay_crypto_error(
            "validate encrypted relay payload",
            "nonce encoding exceeds its size limit",
        ));
    }
    let nonce = decode_bytes(&payload.nonce, "decode relay nonce")?;
    if nonce.len() != NONCE_LEN {
        return Err(relay_crypto_error(
            "validate encrypted relay payload",
            "nonce must be 12 bytes",
        ));
    }
    if payload.ciphertext.len() > maximum_base64_length(maximum_ciphertext_bytes) {
        return Err(relay_crypto_error(
            "validate encrypted relay payload",
            "ciphertext encoding exceeds its size limit",
        ));
    }
    let ciphertext = decode_bytes(&payload.ciphertext, "decode relay ciphertext")?;
    if ciphertext.len() < TAG_LEN || ciphertext.len() > maximum_ciphertext_bytes {
        return Err(relay_crypto_error(
            "validate encrypted relay payload",
            "ciphertext size is invalid",
        ));
    }
    Ok(())
}

fn maximum_base64_length(decoded_bytes: usize) -> usize {
    decoded_bytes
        .saturating_add(2)
        .saturating_div(3)
        .saturating_mul(4)
}

fn peer_crypto_context(
    private_key_base64: &str,
    peer_public_key_base64: &str,
) -> Result<Arc<PeerCryptoContext>, DaemonError> {
    let cache_key = peer_crypto_context_cache_key(private_key_base64, peer_public_key_base64);
    let contexts = PEER_CRYPTO_CONTEXTS.get_or_init(|| Mutex::new(HashMap::new()));
    {
        let contexts = contexts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(context) = contexts.get(&cache_key) {
            return Ok(Arc::clone(context));
        }
    }

    let private_key = decode_private_key(private_key_base64)?;
    let peer_public_key = decode_public_key(peer_public_key_base64)?;
    let shared_secret =
        diffie_hellman(private_key.to_nonzero_scalar(), peer_public_key.as_affine());
    let context = Arc::new(PeerCryptoContext {
        key: derive_symmetric_key(shared_secret.raw_secret_bytes().as_slice())?,
        local_public_key: encode_public_key(&private_key.public_key()),
    });
    let mut contexts = contexts
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if contexts.len() >= PEER_CRYPTO_CONTEXT_CACHE_LIMIT {
        contexts.clear();
    }
    Ok(Arc::clone(
        contexts.entry(cache_key).or_insert_with(|| context),
    ))
}

fn peer_crypto_context_cache_key(
    private_key_base64: &str,
    peer_public_key_base64: &str,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(private_key_base64.as_bytes());
    digest.update([0]);
    digest.update(peer_public_key_base64.as_bytes());
    digest.finalize().into()
}

pub fn encode_public_key(public_key: &PublicKey) -> String {
    base64::engine::general_purpose::STANDARD.encode(public_key.to_encoded_point(false).as_bytes())
}

pub fn generate_private_key_base64() -> String {
    let private_key = SecretKey::random(&mut p256::elliptic_curve::rand_core::OsRng);
    base64::engine::general_purpose::STANDARD.encode(private_key.to_bytes())
}

pub fn public_key_from_private_key_base64(private_key_base64: &str) -> Result<String, DaemonError> {
    let private_key = decode_private_key(private_key_base64)?;
    Ok(encode_public_key(&private_key.public_key()))
}

fn decode_private_key(private_key_base64: &str) -> Result<SecretKey, DaemonError> {
    let bytes = decode_bytes(private_key_base64, "decode relay private key")?;
    SecretKey::from_slice(&bytes)
        .map_err(|error| relay_crypto_error("decode relay private key", &error.to_string()))
}

pub(crate) fn decode_public_key(public_key_base64: &str) -> Result<PublicKey, DaemonError> {
    let bytes = decode_bytes(public_key_base64, "decode relay public key")?;
    let encoded = EncodedPoint::from_bytes(bytes)
        .map_err(|error| relay_crypto_error("decode relay public key", &error.to_string()))?;
    PublicKey::from_encoded_point(&encoded)
        .into_option()
        .ok_or_else(|| relay_crypto_error("decode relay public key", "invalid encoded public key"))
}

fn decode_bytes(value: &str, operation: &'static str) -> Result<Vec<u8>, DaemonError> {
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(|error| relay_crypto_error(operation, &error.to_string()))
}

fn derive_symmetric_key(shared_secret: &[u8]) -> Result<[u8; 32], DaemonError> {
    let hk = Hkdf::<Sha256>::new(None, shared_secret);
    let mut key = [0_u8; 32];
    hk.expand(RELAY_INFO, &mut key)
        .map_err(|error| relay_crypto_error("derive relay symmetric key", &error.to_string()))?;
    Ok(key)
}

fn derive_bound_symmetric_key(
    private_key: &SecretKey,
    peer_public_key: &PublicKey,
    purpose: &[u8],
) -> Result<[u8; 32], DaemonError> {
    if purpose.is_empty() || purpose.len() > 128 {
        return Err(relay_crypto_error(
            "derive bound peer symmetric key",
            "purpose must contain between 1 and 128 bytes",
        ));
    }
    let shared_secret =
        diffie_hellman(private_key.to_nonzero_scalar(), peer_public_key.as_affine());
    let hk = Hkdf::<Sha256>::new(None, shared_secret.raw_secret_bytes().as_slice());
    let mut info = Vec::with_capacity(BOUND_INFO_PREFIX.len() + purpose.len());
    info.extend_from_slice(BOUND_INFO_PREFIX);
    info.extend_from_slice(purpose);
    let mut key = [0_u8; 32];
    hk.expand(&info, &mut key).map_err(|error| {
        relay_crypto_error("derive bound peer symmetric key", &error.to_string())
    })?;
    Ok(key)
}

fn relay_crypto_error(operation: &'static str, message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation,
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_payload_roundtrip_succeeds() {
        let sender_private = generate_private_key_base64();
        let receiver_private = generate_private_key_base64();
        let receiver_public = public_key_from_private_key_base64(&receiver_private)
            .expect("receiver public key should derive");
        let payload = encrypt_payload_for_peer(&sender_private, &receiver_public, b"hello relay")
            .expect("payload should encrypt");
        let decrypted = decrypt_payload_for_private_key(&receiver_private, &payload)
            .expect("payload should decrypt");
        assert_eq!(decrypted.plaintext, b"hello relay");
    }

    #[test]
    fn encrypted_payload_shape_is_bounded_and_validates_key_nonce_and_tag() {
        let sender_private = generate_private_key_base64();
        let receiver_private = generate_private_key_base64();
        let receiver_public = public_key_from_private_key_base64(&receiver_private)
            .expect("receiver public key should derive");
        let payload = encrypt_payload_for_peer(&sender_private, &receiver_public, b"shape")
            .expect("payload should encrypt");
        validate_encrypted_payload_shape(&payload, 64).expect("payload shape should validate");

        let mut malformed = payload.clone();
        malformed.sender_public_key = "not-base64".to_string();
        assert!(validate_encrypted_payload_shape(&malformed, 64).is_err());
        let mut malformed = payload.clone();
        malformed.nonce = base64::engine::general_purpose::STANDARD.encode([0_u8; 11]);
        assert!(validate_encrypted_payload_shape(&malformed, 64).is_err());
        let mut malformed = payload;
        malformed.ciphertext = base64::engine::general_purpose::STANDARD.encode([0_u8; 15]);
        assert!(validate_encrypted_payload_shape(&malformed, 64).is_err());
    }

    #[test]
    fn relay_peer_crypto_context_is_reused_for_stable_keys() {
        let sender_private = generate_private_key_base64();
        let receiver_private = generate_private_key_base64();
        let receiver_public = public_key_from_private_key_base64(&receiver_private)
            .expect("receiver public key should derive");

        let first = peer_crypto_context(&sender_private, &receiver_public)
            .expect("peer context should derive");
        let second = peer_crypto_context(&sender_private, &receiver_public)
            .expect("peer context should reuse");

        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn bound_payload_rejects_aad_or_purpose_changes() {
        let sender_private = generate_private_key_base64();
        let receiver_private = generate_private_key_base64();
        let receiver_public = public_key_from_private_key_base64(&receiver_private)
            .expect("receiver public key should derive");
        let payload = encrypt_payload_for_peer_bound(
            &sender_private,
            &receiver_public,
            b"vault-key",
            b"environment=one",
            b"secret material",
        )
        .expect("bound payload should encrypt");

        assert_eq!(
            decrypt_payload_for_private_key_bound(
                &receiver_private,
                &payload,
                b"vault-key",
                b"environment=one",
            )
            .expect("bound payload should decrypt")
            .plaintext,
            b"secret material"
        );
        assert!(decrypt_payload_for_private_key_bound(
            &receiver_private,
            &payload,
            b"vault-key",
            b"environment=two",
        )
        .is_err());
        assert!(decrypt_payload_for_private_key_bound(
            &receiver_private,
            &payload,
            b"other-purpose",
            b"environment=one",
        )
        .is_err());
    }
}
