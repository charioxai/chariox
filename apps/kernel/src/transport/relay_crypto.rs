use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use arroba_relay::protocol::EncryptedRelayPayload;
use base64::Engine;
use hkdf::Hkdf;
use p256::ecdh::diffie_hellman;
use p256::elliptic_curve::sec1::{FromEncodedPoint, ToEncodedPoint};
use p256::{EncodedPoint, PublicKey, SecretKey};
use rand::RngCore;
use sha2::Sha256;

use crate::error::DaemonError;

const RELAY_INFO: &[u8] = b"arroba-relay-v1";
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;

#[derive(Debug, Clone)]
pub struct DecryptedRelayPayload {
    pub plaintext: Vec<u8>,
    pub sender_public_key: String,
}

pub fn decrypt_payload_for_private_key(
    private_key_base64: &str,
    payload: &EncryptedRelayPayload,
) -> Result<DecryptedRelayPayload, DaemonError> {
    let private_key = decode_private_key(private_key_base64)?;
    let sender_public_key = decode_public_key(&payload.sender_public_key)?;
    let shared_secret = diffie_hellman(
        private_key.to_nonzero_scalar(),
        sender_public_key.as_affine(),
    );
    let key = derive_symmetric_key(shared_secret.raw_secret_bytes().as_slice())?;
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
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
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
    let private_key = decode_private_key(private_key_base64)?;
    let peer_public_key = decode_public_key(peer_public_key_base64)?;
    let shared_secret =
        diffie_hellman(private_key.to_nonzero_scalar(), peer_public_key.as_affine());
    let key = derive_symmetric_key(shared_secret.raw_secret_bytes().as_slice())?;
    let mut nonce = [0_u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext)
        .map_err(|error| relay_crypto_error("encrypt relay payload", &error.to_string()))?;
    Ok(EncryptedRelayPayload {
        sender_public_key: encode_public_key(&private_key.public_key()),
        nonce: base64::engine::general_purpose::STANDARD.encode(nonce),
        ciphertext: base64::engine::general_purpose::STANDARD.encode(ciphertext),
    })
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

fn decode_public_key(public_key_base64: &str) -> Result<PublicKey, DaemonError> {
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
}
