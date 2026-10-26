//! Build the real R3 source App's signed local .cxapp for R4 drills.
#[path = "../tests/support/package.rs"]
mod package;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::json;

fn main() {
    let core = std::fs::read(
        std::env::var("CHARIOX_WORKER_DRILL_CORE").expect("build the source App first"),
    )
    .unwrap();
    let output = std::path::PathBuf::from(std::env::args_os().nth(1).expect("output directory"));
    std::fs::create_dir_all(&output).unwrap();
    let (bytes, publisher) = package::package(&core);
    std::fs::write(output.join("source.cxapp"), bytes).unwrap();
    std::fs::write(output.join("publisher.json"), serde_json::to_vec(&json!({
        "schema":"chariox.developer-publisher.v1", "algorithm":"ed25519",
        "publisher":{"id":publisher.publisher_id,"keyId":publisher.key_id,"name":"R4 source App drill"},
        "publicKey":STANDARD.encode(publisher.public_key.as_bytes()),
    })).unwrap()).unwrap();
    println!("Source App package and public publisher enrollment written");
}
