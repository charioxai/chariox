//! Arbitrary bytes as a downloaded package: the archive reader, envelope and
//! signature checks must reject them without panicking or over-allocating.
#![no_main]
use chariox_app_package::{
    inspect_untrusted, verify, Limits, TrustedPublisher, VerificationPolicy,
};
use ed25519_dalek::SigningKey;
use libfuzzer_sys::fuzz_target;

fn policy() -> VerificationPolicy {
    VerificationPolicy::new(
        500,
        vec![TrustedPublisher {
            publisher_id: "com.example".to_owned(),
            key_id: "developer-1".to_owned(),
            public_key: SigningKey::from_bytes(&[7; 32]).verifying_key(),
        }],
    )
}

static SEED: std::sync::Once = std::sync::Once::new();

fuzz_target!(|bytes: &[u8]| {
    // The committed seed is what lets mutations reach the manifest and
    // declaration checks; a stale one would fuzz only the rejection path.
    SEED.call_once(|| {
        if let Err(error) = verify(include_bytes!("../seeds/archive/0"), &policy()) {
            panic!("seeds/archive/0 must verify under the fuzz key; regenerate it (package README): {error:?}");
        }
    });
    let _ = inspect_untrusted(bytes, &Limits::default());
    let _ = verify(bytes, &policy());
});
