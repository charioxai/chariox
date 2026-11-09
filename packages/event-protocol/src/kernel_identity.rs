//! The kernel's relay identity key is its credential at the event gateways.
//!
//! A kernel proves who it is by signing with the P-256 key it already uses for relay peer
//! encryption; no separate key, secret store or operator-issued token exists. The gateway-facing
//! kernel id is the fingerprint of the public key (the same value as the kernel's
//! `kernel_public_key_thumbprint`), so one id cannot name two keys.
//!
//! The key also performs ECDH for the relay. Every signed message therefore starts with a
//! distinct domain tag and never covers bytes an attacker could make look like a relay frame.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use p256::ecdsa::signature::{Signer, Verifier};
use p256::ecdsa::{Signature, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};

pub const KERNEL_KEY_HEADER: &str = "x-chariox-kernel-key";
pub const KERNEL_TIMESTAMP_HEADER: &str = "x-chariox-kernel-timestamp";
pub const KERNEL_NONCE_HEADER: &str = "x-chariox-kernel-nonce";
pub const KERNEL_SIGNATURE_HEADER: &str = "x-chariox-kernel-signature";
pub const OWNER_ID_HEADER: &str = "x-chariox-owner-id";

const AEDS_HELLO_DOMAIN: &[u8] = b"chariox-aeds-hello-v4";
const AEGS_REQUEST_DOMAIN: &[u8] = b"chariox-aegs-request-v5";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernelIdentityError {
    InvalidKey,
    InvalidSignature,
    SignatureMismatch,
}

impl std::fmt::Display for KernelIdentityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidKey => "kernel identity key is not a P-256 key",
            Self::InvalidSignature => "kernel signature is not a P-256 signature",
            Self::SignatureMismatch => "kernel signature does not match the message",
        })
    }
}

impl std::error::Error for KernelIdentityError {}

/// The gateway-facing kernel id: lowercase hex SHA-256 of the base64 public key, the value the
/// kernel already reports as `kernel_public_key_thumbprint`.
pub fn kernel_id_for_public_key(public_key: &str) -> String {
    Sha256::digest(public_key.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Signs `message` with the kernel's base64 private key; the result is base64.
pub fn sign(private_key: &str, message: &[u8]) -> Result<String, KernelIdentityError> {
    let bytes = STANDARD
        .decode(private_key)
        .map_err(|_| KernelIdentityError::InvalidKey)?;
    let key = SigningKey::from_slice(&bytes).map_err(|_| KernelIdentityError::InvalidKey)?;
    let signature: Signature = key.sign(message);
    Ok(STANDARD.encode(signature.to_bytes()))
}

/// Checks a base64 signature against the base64 SEC1 public key.
pub fn verify(
    public_key: &str,
    message: &[u8],
    signature: &str,
) -> Result<(), KernelIdentityError> {
    let key_bytes = STANDARD
        .decode(public_key)
        .map_err(|_| KernelIdentityError::InvalidKey)?;
    let key =
        VerifyingKey::from_sec1_bytes(&key_bytes).map_err(|_| KernelIdentityError::InvalidKey)?;
    let signature_bytes = STANDARD
        .decode(signature)
        .map_err(|_| KernelIdentityError::InvalidSignature)?;
    let signature = Signature::from_slice(&signature_bytes)
        .map_err(|_| KernelIdentityError::InvalidSignature)?;
    key.verify(message, &signature)
        .map_err(|_| KernelIdentityError::SignatureMismatch)
}

/// What the kernel signs in answer to the AEDS challenge.
pub fn aeds_hello_message(nonce: &str, kernel_id: &str) -> Vec<u8> {
    join(&[AEDS_HELLO_DOMAIN, nonce.as_bytes(), kernel_id.as_bytes()])
}

/// The parts of one AEGS management request a kernel signs. `generator_id` names the AEGS the
/// request is for; `owner_id` is the secret connection owner the request acts for.
#[derive(Clone, Copy)]
pub struct ManagementRequest<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub generator_id: &'a str,
    pub owner_id: &'a str,
    pub timestamp_ms: u64,
    pub nonce: &'a str,
    pub body: &'a [u8],
}

impl ManagementRequest<'_> {
    pub fn message(&self) -> Vec<u8> {
        let timestamp = self.timestamp_ms.to_string();
        let body_digest = Sha256::digest(self.body);
        join(&[
            AEGS_REQUEST_DOMAIN,
            self.method.as_bytes(),
            self.path.as_bytes(),
            self.generator_id.as_bytes(),
            self.owner_id.as_bytes(),
            timestamp.as_bytes(),
            self.nonce.as_bytes(),
            &body_digest,
        ])
    }
}

fn join(parts: &[&[u8]]) -> Vec<u8> {
    let mut message = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        if index > 0 {
            message.push(0);
        }
        message.extend_from_slice(part);
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::SecretKey;

    fn key_pair() -> (String, String) {
        let secret = SecretKey::random(&mut p256::elliptic_curve::rand_core::OsRng);
        let private = STANDARD.encode(secret.to_bytes());
        let public = STANDARD.encode(
            p256::elliptic_curve::sec1::ToEncodedPoint::to_encoded_point(
                &secret.public_key(),
                false,
            )
            .as_bytes(),
        );
        (private, public)
    }

    #[test]
    fn a_signature_verifies_only_for_its_key_and_message() {
        let (private, public) = key_pair();
        let (_, other_public) = key_pair();
        let signature = sign(&private, b"message").unwrap();
        assert_eq!(verify(&public, b"message", &signature), Ok(()));
        assert_eq!(
            verify(&public, b"other", &signature),
            Err(KernelIdentityError::SignatureMismatch)
        );
        assert_eq!(
            verify(&other_public, b"message", &signature),
            Err(KernelIdentityError::SignatureMismatch)
        );
        assert_eq!(
            verify(&public, b"message", "not base64!"),
            Err(KernelIdentityError::InvalidSignature)
        );
        assert_eq!(
            verify("AAAA", b"message", &signature),
            Err(KernelIdentityError::InvalidKey)
        );
    }

    #[test]
    fn the_kernel_id_is_the_stable_key_fingerprint() {
        let (_, public) = key_pair();
        let id = kernel_id_for_public_key(&public);
        assert_eq!(id.len(), 64);
        assert!(id.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(id, kernel_id_for_public_key(&public));
        assert_ne!(id, kernel_id_for_public_key(&key_pair().1));
    }

    #[test]
    fn management_signatures_cover_every_part_of_the_request() {
        let base = ManagementRequest {
            method: "PUT",
            path: "/v1/subscriptions/reconcile",
            generator_id: "dev.example.generator",
            owner_id: "owner-secret",
            timestamp_ms: 1_000,
            nonce: "nonce-1",
            body: b"{}",
        };
        let message = base.message();
        for changed in [
            ManagementRequest {
                method: "POST",
                ..base
            },
            ManagementRequest {
                path: "/v1/other",
                ..base
            },
            ManagementRequest {
                generator_id: "dev.other",
                ..base
            },
            ManagementRequest {
                owner_id: "owner-other",
                ..base
            },
            ManagementRequest {
                timestamp_ms: 1_001,
                ..base
            },
            ManagementRequest {
                nonce: "nonce-2",
                ..base
            },
            ManagementRequest {
                body: b"{ }",
                ..base
            },
        ] {
            assert_ne!(changed.message(), message);
        }
        assert_ne!(aeds_hello_message("n", "k"), message);
    }
}
