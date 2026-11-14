//! What the examples that sign an App with a self-contained client view share: the component, the
//! fixed public test key and the publisher file the drill enrolls.
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::Path;

/// A core module becomes a component; a component (a `wasm32-wasip2` build) is taken as it is.
pub fn component(wasm: &Path) -> Vec<u8> {
    let bytes = std::fs::read(wasm).unwrap();
    if bytes.get(4..8) == Some(&[0x0d, 0x00, 0x01, 0x00]) {
        return bytes;
    }
    wit_component::ComponentEncoder::default()
        .module(&bytes)
        .unwrap()
        .validate(true)
        .encode()
        .unwrap()
}

/// The key is fixed public test material; the drill enrolls its public half like any publisher.
pub struct DevKey {
    pub key: ed25519_dalek::SigningKey,
    pub id: String,
}

pub fn dev_key() -> DevKey {
    let key = ed25519_dalek::SigningKey::from_bytes(&[31; 32]);
    let id = format!("dev-{:x}", Sha256::digest(key.verifying_key().as_bytes()));
    DevKey { key, id }
}

pub fn write_publisher(output: &Path, publisher_id: &str, name: &str, key: &DevKey) {
    std::fs::write(
        output.join("publisher.json"),
        serde_json::to_vec(&json!({
            "schema":"chariox.developer-publisher.v1", "algorithm":"ed25519",
            "publisher":{"id":publisher_id,"keyId":key.id,"name":name},
            "publicKey":STANDARD.encode(key.key.verifying_key().as_bytes()),
        }))
        .unwrap(),
    )
    .unwrap();
}
