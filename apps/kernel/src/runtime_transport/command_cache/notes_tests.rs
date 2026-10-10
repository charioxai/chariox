//! MD-N3 / MP-11: private responses cannot replay across terminal identities.
use super::*;
#[tokio::test]
async fn notes_cached_response_cannot_cross_owner_or_client() {
    let request = LocalDaemonRequest::Notes(crate::local::NotesRequest {
        command: crate::local::NoteCommand::Read {
            note_id: "fixture-note".into(),
        },
    });
    let mut command = KernelCommand::from_local_request("MD-N3-replay", None, None, &request);
    command.caller.user_id = Some("alice".into());
    command.caller.client_id = Some("alice-client".into());
    let fingerprint = CommandFingerprint::from_command_and_request(&command, &request);
    assert!(
        !should_persist_completed_result(&fingerprint),
        "MD-N3: notes must not persist in the transport cache"
    );
    let cache = CommandResultCache::default();
    assert!(matches!(
        cache.reserve(&command.command_id, &fingerprint).await,
        CommandReservation::Dispatch
    ));
    let response = serde_json::json!({"Notes":{"result":{"event":"note_changed","note":{"comment":"private fixture text"}}}});
    cache
        .complete(
            command.command_id.clone(),
            fingerprint.clone(),
            &KernelOutgoingFrame::Response {
                request_id: command.command_id.clone(),
                response: Box::new(Some(response.clone())),
                error: None,
            },
        )
        .await;
    for (user, client) in [("bob", "alice-client"), ("alice", "other-client")] {
        let mut foreign = command.clone();
        foreign.caller.user_id = Some(user.into());
        foreign.caller.client_id = Some(client.into());
        let identity = CommandFingerprint::from_command_and_request(&foreign, &request);
        assert!(matches!(
            cache.reserve(&command.command_id, &identity).await,
            CommandReservation::Conflict
        ));
    }
    let CommandReservation::Wait(replay) = cache.reserve(&command.command_id, &fingerprint).await
    else {
        panic!("owner retry expected")
    };
    assert_eq!(*replay.await.unwrap().response_value(), Some(response));
}
