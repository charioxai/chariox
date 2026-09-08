use crate::local::LOCAL_DAEMON_PROTOCOL_VERSION;
use chariox_app_package::SUPPORTED_SDK_VERSION;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[test]
fn fetch_sdk_contract_is_pinned_to_its_kernel_and_runtime_release() {
    const BYTES: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../packages/app-sdk/test/fetch-contract.json"
    ));
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 297);
    let fixture: Value = serde_json::from_slice(BYTES).unwrap();
    assert_eq!(fixture["minimumKernelProtocol"], 296);
    assert_eq!(fixture["sdkVersion"], SUPPORTED_SDK_VERSION);
    // JavaScript checks the actual Fetch response against this shared fixture;
    // the HTTP decoder/encoder fixture separately checks the underlying calls.
    assert_eq!(
        format!("{:x}", Sha256::digest(BYTES)),
        "379fb9102708420307c726daba28105793bdc54a618cf7d4abc221a67733903b"
    );
}
