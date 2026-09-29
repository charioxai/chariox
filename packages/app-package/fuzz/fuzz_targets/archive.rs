//! Arbitrary bytes as a downloaded package: the archive reader, envelope and
//! signature checks must reject them without panicking or over-allocating.
#![no_main]
use chariox_app_package::{inspect_untrusted, verify, Limits, TrustedPublisher, VerificationPolicy};
use ed25519_dalek::SigningKey;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _ = inspect_untrusted(bytes, &Limits::default());
    let policy = VerificationPolicy::new(
        500,
        vec![TrustedPublisher {
            publisher_id: "com.example".to_owned(),
            key_id: "developer-1".to_owned(),
            public_key: SigningKey::from_bytes(&[7; 32]).verifying_key(),
        }],
    );
    let _ = verify(bytes, &policy);
});
