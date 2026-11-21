//! Path 1 bundle key: the Vault client answers Cloud's challenge only with the right identity
//! and hands the identity to one destination kernel only.
use super::*;
use age::Encryptor;
use std::io::Write;

fn challenge(recipient: &str, answer: &str) -> String {
    let recipient = Recipient::from_str(recipient).expect("recipient");
    let encryptor = Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))
        .expect("encryptor");
    let mut ciphertext = Vec::new();
    let mut writer = encryptor.wrap_output(&mut ciphertext).expect("wrap");
    writer.write_all(answer.as_bytes()).expect("write");
    writer.finish().expect("finish");
    URL_SAFE_NO_PAD.encode(ciphertext)
}

#[test]
fn only_the_holder_of_the_identity_answers_the_challenge() {
    let key = generate();
    let other = generate();
    assert_eq!(recipient_of(&key.identity).unwrap(), key.recipient);
    validate_recipient(&key.recipient).unwrap();
    let answer = "A".repeat(43);
    let ciphertext = challenge(&key.recipient, &answer);
    assert_eq!(
        &*answer_challenge(&key.identity, &ciphertext).unwrap(),
        &answer
    );
    assert!(
        answer_challenge(&other.identity, &ciphertext).is_err(),
        "a different Vault cannot answer"
    );
    assert!(answer_challenge(&key.identity, "not-base64!").is_err());
    assert!(answer_challenge(&key.identity, &URL_SAFE_NO_PAD.encode(b"not an age file")).is_err());
    assert!(answer_challenge("AGE-SECRET-KEY-1BAD", &ciphertext).is_err());
}

/// What the destination's restore helper does with its one-time age key: open the item and check its binding.
fn open(
    destination: &Identity,
    expected: &BundleKeyBinding,
    key: &BundleKeyMaterialization,
) -> Result<String, String> {
    let bytes = URL_SAFE_NO_PAD
        .decode(&key.ciphertext)
        .map_err(|_| "malformed")?;
    let decryptor = Decryptor::new(&bytes[..]).map_err(|_| "malformed")?;
    let mut reader = decryptor
        .decrypt(std::iter::once(destination as &dyn age::Identity))
        .map_err(|_| "not for this restore")?;
    let mut plaintext = Vec::new();
    reader
        .read_to_end(&mut plaintext)
        .map_err(|_| "unreadable")?;
    let payload: SealedPayload = serde_json::from_slice(&plaintext).map_err(|_| "malformed")?;
    if &payload.binding != expected {
        return Err("another launch".into());
    }
    Ok(payload.identity)
}

#[test]
fn a_sealed_key_opens_only_for_its_destination_restore_and_launch() {
    let key = generate();
    let destination = Identity::generate();
    let stranger = Identity::generate();
    let binding = BundleKeyBinding {
        key_id: "key-1".into(),
        launch_id: "launch-1".into(),
        source_kernel_id: "owner-kernel".into(),
        target_kernel_id: "target-kernel".into(),
    };
    let recipient = destination.to_public().to_string();
    let sealed = seal(&recipient, &binding, &key.identity).unwrap();
    assert_eq!(
        open(&destination, &binding, &sealed).unwrap(),
        *key.identity
    );
    assert!(
        open(&stranger, &binding, &sealed).is_err(),
        "another restore cannot open it"
    );
    let other_launch = BundleKeyBinding {
        launch_id: "launch-2".into(),
        ..binding.clone()
    };
    assert_eq!(
        open(&destination, &other_launch, &sealed).unwrap_err(),
        "another launch",
        "a seal names its launch"
    );
    assert!(
        !sealed.ciphertext.contains("AGE-SECRET") && !format!("{sealed:?}").contains("AGE-SECRET")
    );
    assert!(seal(&recipient, &binding, "not an identity").is_err());
    assert!(seal("age1invalid", &binding, &key.identity).is_err());
}
