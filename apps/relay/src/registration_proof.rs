//! A kernel proves at registration that it holds the private key of the public key it
//! registers. The relay offers a fresh challenge in every WebSocket handshake and the kernel
//! signs it, bound to the relay address it dialed, its kernel id and its relay token, so a
//! leaked relay credential alone can neither register as a kernel nor displace it.

use std::io::Read as _;

use base64::Engine as _;
use p256::ecdsa::signature::{Signer as _, Verifier as _};
use p256::ecdsa::{Signature, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::handshake::client::Response;

use crate::protocol::DaemonRegistration;

/// The handshake response header that carries the connection's registration challenge.
pub const CHALLENGE_HEADER: &str = "x-chariox-registration-challenge";
const DOMAIN: &str = "chariox-relay-registration-v1";

/// A fresh challenge for one relay connection; it admits one registration.
pub fn new_challenge() -> std::io::Result<String> {
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom").and_then(|mut random| random.read_exact(&mut bytes))?;
    Ok(hex(&bytes))
}

/// The challenge the relay offered in this handshake.
pub fn offered_challenge(handshake: &Response) -> Option<&str> {
    handshake
        .headers()
        .get(CHALLENGE_HEADER)
        .and_then(|value| value.to_str().ok())
}

/// The relay address as the kernel dials it: the Host header of its WebSocket request,
/// which the relay reads back from the same request.
pub fn dialed_address(relay_url: &str) -> Result<String, String> {
    let request = relay_url
        .into_client_request()
        .map_err(|error| error.to_string())?;
    request
        .headers()
        .get("host")
        .and_then(|host| host.to_str().ok())
        .map(str::to_owned)
        .ok_or_else(|| format!("relay URL {relay_url} names no host"))
}

/// Signs the registration for the connection that offered `challenge`.
pub fn prove(
    identity_key: &p256::SecretKey,
    relay_address: &str,
    registration: &DaemonRegistration,
    challenge: &str,
) -> String {
    let signature: Signature =
        SigningKey::from(identity_key).sign(&message(relay_address, registration, challenge));
    base64::engine::general_purpose::STANDARD.encode(signature.to_bytes())
}

/// Checks that the registration's own public key signed this connection's challenge.
pub fn verify(
    relay_address: &str,
    registration: &DaemonRegistration,
    challenge: &str,
    proof: &str,
) -> Result<(), &'static str> {
    let public_key = base64::engine::general_purpose::STANDARD
        .decode(&registration.public_key)
        .ok()
        .and_then(|bytes| VerifyingKey::from_sec1_bytes(&bytes).ok())
        .ok_or("registration public key is not a kernel identity key")?;
    let signature = base64::engine::general_purpose::STANDARD
        .decode(proof)
        .ok()
        .and_then(|bytes| Signature::from_slice(&bytes).ok())
        .ok_or("registration proof is not a signature")?;
    public_key
        .verify(&message(relay_address, registration, challenge), &signature)
        .map_err(|_| {
            "registration proof is not signed by the kernel identity key for this connection"
        })
}

fn message(relay_address: &str, registration: &DaemonRegistration, challenge: &str) -> Vec<u8> {
    let token_fingerprint = hex(&Sha256::digest(registration.auth_token.as_bytes()));
    serde_json::to_vec(&[
        DOMAIN,
        &relay_address.to_ascii_lowercase(),
        &registration.daemon_id,
        &token_fingerprint,
        challenge,
    ])
    .expect("strings serialize")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
