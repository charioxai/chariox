use std::collections::BTreeMap;

use chariox_app_package::{pack, verify, Limits, Manifest, TrustedPublisher, VerificationPolicy};
use ed25519_dalek::SigningKey;
use serde_json::{json, Value};

#[test]
fn campaign_fixture_is_a_verified_joint_release_with_declared_data_access() {
    let mut value: Value =
        serde_json::from_str(include_str!("../../../examples/apps/campaign/app.json")).unwrap();
    // Public, deterministic test identity only. No developer credential is read.
    value["publisher"] = json!({"id":"com.example","keyId":"fixture","name":"Fixture"});
    let manifest: Manifest = serde_json::from_value(value).unwrap();
    let files: BTreeMap<String, Vec<u8>> = [
        (
            "runtime/main.mjs",
            include_bytes!("../../../examples/apps/campaign/bundle/runtime/main.mjs").as_slice(),
        ),
        (
            "schemas/tools.json",
            include_bytes!("../../../examples/apps/campaign/bundle/schemas/tools.json").as_slice(),
        ),
        (
            "schemas/events.json",
            include_bytes!("../../../examples/apps/campaign/bundle/schemas/events.json").as_slice(),
        ),
        (
            "ui/index.html",
            include_bytes!("../../../examples/apps/campaign/bundle/ui/index.html").as_slice(),
        ),
        (
            "ui/app.mjs",
            include_bytes!("../../../examples/apps/campaign/bundle/ui/app.mjs").as_slice(),
        ),
        (
            "ui/style.css",
            include_bytes!("../../../examples/apps/campaign/bundle/ui/style.css").as_slice(),
        ),
    ]
    .into_iter()
    .map(|(path, bytes)| (path.to_owned(), bytes.to_vec()))
    .collect();
    let key = SigningKey::from_bytes(&[7; 32]);
    let policy = VerificationPolicy::new(
        500,
        vec![TrustedPublisher {
            publisher_id: "com.example".into(),
            key_id: "fixture".into(),
            public_key: key.verifying_key(),
        }],
    );
    let bytes = pack(&manifest, &files, &key, &Limits::default()).unwrap();
    let release = verify(&bytes, &policy).unwrap();
    assert_eq!(release.declarations().tools[0].name, "read_campaign");
    assert_eq!(release.declarations().events[0].name, "campaign_changed");
    assert_eq!(
        release.manifest().capabilities.network[0].origin,
        "https://campaigns.example.com"
    );
}
