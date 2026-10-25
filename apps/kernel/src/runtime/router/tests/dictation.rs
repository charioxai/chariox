use base64::Engine as _;

use crate::local::test_support::LocalRouterTestHarness;
use crate::local::{
    DictationCommand, DictationRequest, DictationResult, LocalDaemonRequest, LocalDaemonResponse,
};

fn dictation(
    harness: &LocalRouterTestHarness,
    user: &str,
    command: DictationCommand,
) -> DictationResult {
    let response = harness
        .dispatch_as_user(
            user,
            LocalDaemonRequest::Dictation(DictationRequest { command }),
        )
        .expect("dictation request should be served");
    let LocalDaemonResponse::Dictation { result } = response else {
        panic!("unexpected dictation response");
    };
    result
}

fn append(upload_id: &str, offset: u64, bytes: usize) -> DictationCommand {
    DictationCommand::AppendAudio {
        upload_id: upload_id.to_string(),
        offset,
        pcm_base64: base64::engine::general_purpose::STANDARD.encode(vec![1_u8; bytes]),
    }
}

#[test]
fn one_terminal_user_cannot_touch_the_recording_or_language_of_another() {
    let harness = LocalRouterTestHarness::new();
    let DictationResult::AudioAccepted { received_bytes, .. } =
        dictation(&harness, "alice", append("rec-1", 0, 1000))
    else {
        panic!("alice's chunk")
    };
    assert_eq!(received_bytes, 1000);

    // bob's chunk with the same id starts his own recording instead of extending alice's
    let DictationResult::AudioAccepted { received_bytes, .. } =
        dictation(&harness, "bob", append("rec-1", 0, 400))
    else {
        panic!("bob's chunk")
    };
    assert_eq!(received_bytes, 400);
    let DictationResult::AudioAccepted { received_bytes, .. } =
        dictation(&harness, "alice", append("rec-1", 1000, 1000))
    else {
        panic!("alice's second chunk")
    };
    assert_eq!(
        received_bytes, 2000,
        "alice's recording was not modified by bob"
    );

    dictation(
        &harness,
        "alice",
        DictationCommand::SetLanguage {
            language: Some("es".to_string()),
        },
    );
    let DictationResult::Status { status } = dictation(&harness, "bob", DictationCommand::Status)
    else {
        panic!("bob's status")
    };
    assert_eq!(status.language, None, "alice's language is not bob's");
}

#[test]
fn remote_dictation_uses_owner_scoped_vault_handles_and_refuses_foreign_unlock_context() {
    let harness = LocalRouterTestHarness::new();
    let request = harness
        .fixture_session_request("dictation-vault-context")
        .with_owner_user_id("alice");
    let session = harness.with_app_mut(|app| {
        crate::app::KernelSessionService::new(app)
            .create_session(request)
            .unwrap()
            .0
    });
    dictation(
        &harness,
        "alice",
        DictationCommand::SetService {
            service: crate::local::DictationProvider::Openai,
        },
    );
    let DictationResult::Status { status: alice } =
        dictation(&harness, "alice", DictationCommand::Status)
    else {
        panic!("alice's status")
    };
    let DictationResult::Status { status: bob } =
        dictation(&harness, "bob", DictationCommand::Status)
    else {
        panic!("bob's status")
    };
    assert_eq!(
        bob.service,
        crate::local::DictationProvider::Openai,
        "the choice belongs to the kernel"
    );
    assert_ne!(
        alice.vault_key, bob.vault_key,
        "the keys belong to their authenticated users"
    );
    let error = harness
        .dispatch_as_user(
            "bob",
            LocalDaemonRequest::Dictation(DictationRequest {
                command: DictationCommand::Transcribe {
                    upload_id: "rec-1".into(),
                    context: Some(crate::local::DictationContext {
                        session_id: session.id().into(),
                        agent_id: "foreign".into(),
                    }),
                },
            }),
        )
        .unwrap_err();
    assert!(
        matches!(error, crate::error::DaemonError::RelayTransport { code, .. } if code == "dictation_context_invalid")
    );
}
