//! Row 207: a file slice crosses the relay once as bytes, not as base64 inside base64.
use super::*;

use crate::transport::relay_peer::{RelayPeerResponse, RemoteFileBytes};

fn keys() -> (String, String, String) {
    let sender = relay_crypto::generate_private_key_base64();
    let receiver = relay_crypto::generate_private_key_base64();
    let receiver_public = relay_crypto::public_key_from_private_key_base64(&receiver).unwrap();
    (sender, receiver, receiver_public)
}

fn slice_reply(bytes: &[u8]) -> RelayPeerResponse {
    RelayPeerResponse::FileSliceRead {
        size_bytes: bytes.len() as u64,
        sha256: None,
        revision: "revision-1".into(),
        content: RemoteFileBytes::encode(bytes),
    }
}

// The sealed reply to a 3,584,000-byte part (the size rentalroot measured at 6,372,158 B on
// the relay) is the part plus a small envelope before the relay's own framing.
#[test]
fn a_file_slice_reply_is_sealed_at_its_own_size() {
    let bytes: Vec<u8> = (0..3_584_000u32).map(|n| (n % 251) as u8).collect();
    let (sender, receiver, receiver_public) = keys();
    let outcome = encrypt_peer_response(&sender, &receiver_public, slice_reply(&bytes));
    let sealed = outcome.encrypted_response.expect("sealed reply");
    let ciphertext = base64::engine::general_purpose::STANDARD
        .decode(&sealed.ciphertext)
        .unwrap();
    assert!(
        ciphertext.len() < bytes.len() + bytes.len() / 20,
        "the reply is {} bytes for {} bytes of file",
        ciphertext.len(),
        bytes.len()
    );
    let opened = relay_crypto::decrypt_payload_for_private_key(&receiver, &sealed).unwrap();
    let (json, _) = crate::transport::framed_plaintext::open(&opened.plaintext).unwrap();
    let RelayPeerResponse::FileSliceRead { content, .. } =
        serde_json::from_slice(&json).unwrap()
    else {
        panic!("a file slice reply");
    };
    assert_eq!(content.decode().unwrap().as_slice(), bytes.as_slice());
}
