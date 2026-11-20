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

#[test]
fn a_sealed_key_opens_only_for_its_destination_and_binding() {
    let key = generate();
    let owner = relay_crypto::generate_private_key_base64();
    let destination = relay_crypto::generate_private_key_base64();
    let stranger = relay_crypto::generate_private_key_base64();
    let owner_public = relay_crypto::public_key_from_private_key_base64(&owner).unwrap();
    let destination_public =
        relay_crypto::public_key_from_private_key_base64(&destination).unwrap();
    let binding = BundleKeyBinding {
        key_id: "key-1".into(),
        launch_id: "launch-1".into(),
        source_kernel_id: "owner-kernel".into(),
        target_kernel_id: "target-kernel".into(),
    };
    let sealed = seal(&owner, &destination_public, &binding, &key.identity).unwrap();
    let thumbprint = crate::runtime::terminal_pairings::public_key_thumbprint(&owner_public);
    assert_eq!(
        &*open(&destination, &thumbprint, &binding, &sealed).unwrap(),
        &*key.identity
    );
    assert!(
        open(&stranger, &thumbprint, &binding, &sealed).is_err(),
        "another kernel cannot open it"
    );
    let other_launch = BundleKeyBinding {
        launch_id: "launch-2".into(),
        ..binding.clone()
    };
    assert!(
        open(&destination, &thumbprint, &other_launch, &sealed).is_err(),
        "a seal is bound to its launch"
    );
    assert!(
        open(&destination, "someone-else", &binding, &sealed).is_err(),
        "only the owner's kernel may seal it"
    );
    assert!(
        format!("{sealed:?}").contains("SEALED") && !format!("{sealed:?}").contains("AGE-SECRET")
    );
    assert!(seal(&owner, &destination_public, &binding, "not an identity").is_err());
}
