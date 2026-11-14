// MP-08 / MP-09 / MP-10 / MP-11 (PR5-A4): another user's agent deadline is readable by whoever
// may view that agent's trace (its owner, who acts for the owner, Transparent and Full members);
// a Private member cannot read it. Arming one stays with the owner.
use super::historical_workflows::terminal_command;
use super::*;
use crate::session::CollaborationLevel;

const ENROLLED_OWNER: &str = "kernel-owner";
const DEADLINE_MS: u64 = 4_102_444_800_000;

struct Room {
    router: CommandRouter,
    session_id: String,
    agent_id: String,
    _worktree: crate::test_support::TestWorktree,
}

/// A room with one agent of the room owner, armed with a deadline, and one member per level.
async fn room_with_armed_deadline(label: &str, enrolled: bool) -> Room {
    let worktree = crate::test_support::TestWorktree::new(label);
    let mut config = DaemonConfig::for_tests();
    if enrolled {
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            user_id: ENROLLED_OWNER.into(),
            realm_id: "test-realm".into(),
            ..Default::default()
        });
    }
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let session = app
        .sessions_mut()
        .create_session(worktree.session_request())
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = spawn_test_agent(&mut app, &session_id, "owned", "dev-stub")
        .id()
        .to_string();
    for (user, level) in [
        ("private-user", CollaborationLevel::Private),
        ("transparent-user", CollaborationLevel::Transparent),
        ("full-user", CollaborationLevel::Full),
    ] {
        let (_, invite) = app
            .sessions_mut()
            .create_session_invite(
                &session_id,
                format!("invite-{user}"),
                DEFAULT_LOCAL_USER_ID.to_string(),
                None,
                Some(1),
                level,
            )
            .expect("invite should be created");
        app.sessions_mut()
            .join_session_invite(&session_id, invite.invite_id(), user.to_string(), 1)
            .expect("user should join session");
    }
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let room = Room {
        router,
        session_id,
        agent_id,
        _worktree: worktree,
    };
    room.set_as(DEFAULT_LOCAL_USER_ID)
        .await
        .expect("the owner arms the deadline");
    room
}

impl Room {
    async fn set_as(&self, user: &str) -> Result<LocalDaemonResponse, DaemonError> {
        let set = serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
            "SetAgentDeadline": {
                "session_id": self.session_id,
                "agent_id": self.agent_id,
                "deadline_ms": DEADLINE_MS,
            }
        }))
        .unwrap();
        self.router
            .dispatch(terminal_command(&set, Some(user)), set)
            .await
    }

    async fn read_as(&self, user: &str) -> Result<LocalDaemonResponse, DaemonError> {
        let get = serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
            "GetAgentDeadline": {"session_id": self.session_id, "agent_id": self.agent_id}
        }))
        .unwrap();
        self.router
            .dispatch(terminal_command(&get, Some(user)), get)
            .await
    }

    async fn assert_reads(&self, user: &str) {
        let response = self
            .read_as(user)
            .await
            .unwrap_or_else(|error| panic!("{user} must read the agent deadline: {error:?}"));
        assert_eq!(
            serde_json::to_value(response).unwrap()["AgentDeadline"]["deadline"]["deadline_ms"],
            DEADLINE_MS,
            "{user} read another deadline"
        );
    }
}

#[test]
fn a_private_member_cannot_read_another_users_agent_deadline() {
    run_remote_authorization_large_stack_test("deadline-read-private", || async {
        let room = room_with_armed_deadline("deadline-read-private", false).await;
        room.assert_reads(DEFAULT_LOCAL_USER_ID).await;
        assert_ownership_denied(
            room.read_as("private-user")
                .await
                .expect_err("a Private member must not read another user's agent deadline"),
            "private-user",
            DEFAULT_LOCAL_USER_ID,
        );
    });
}

#[test]
fn a_transparent_member_reads_another_users_agent_deadline() {
    run_remote_authorization_large_stack_test("deadline-read-transparent", || async {
        let room = room_with_armed_deadline("deadline-read-transparent", false).await;
        room.assert_reads("transparent-user").await;
    });
}

#[test]
fn a_full_member_reads_another_users_agent_deadline() {
    run_remote_authorization_large_stack_test("deadline-read-full", || async {
        let room = room_with_armed_deadline("deadline-read-full", false).await;
        room.assert_reads("full-user").await;
    });
}

#[test]
fn the_enrolled_owner_reads_the_deadline_of_an_agent_made_before_enrollment() {
    run_remote_authorization_large_stack_test("deadline-read-enrolled-owner", || async {
        let room = room_with_armed_deadline("deadline-read-enrolled-owner", true).await;
        room.assert_reads(ENROLLED_OWNER).await;
    });
}

#[test]
fn a_member_who_may_view_the_trace_still_cannot_arm_the_deadline() {
    run_remote_authorization_large_stack_test("deadline-set-transparent-full", || async {
        let room = room_with_armed_deadline("deadline-set-transparent-full", false).await;
        for user in ["transparent-user", "full-user"] {
            assert_ownership_denied(
                room.set_as(user)
                    .await
                    .expect_err("only who acts for the owner arms an agent deadline"),
                user,
                DEFAULT_LOCAL_USER_ID,
            );
        }
    });
}

#[test]
fn agent_peek_obeys_trace_visibility_and_confines_entries_to_the_requested_room() {
    run_remote_authorization_large_stack_test("agent-peek-visibility", || async {
        let room = room_with_armed_deadline("agent-peek-visibility", false).await;
        let peek =
            LocalDaemonRequest::PeekAgentTranscript(crate::local::PeekAgentTranscriptRequest {
                session_id: room.session_id.clone(),
                agent_id: room.agent_id.clone(),
                limit: None,
                before: None,
            });
        for user in [DEFAULT_LOCAL_USER_ID, "transparent-user", "full-user"] {
            assert!(matches!(
                room.router
                    .dispatch(terminal_command(&peek, Some(user)), peek.clone())
                    .await
                    .unwrap(),
                LocalDaemonResponse::AgentTranscriptPeek { .. }
            ));
        }
        let read = LocalDaemonRequest::ReadAgentTranscriptEntry(
            crate::local::ReadAgentTranscriptEntryRequest {
                session_id: room.session_id.clone(),
                agent_id: room.agent_id.clone(),
                entry_id: "history:1:1".into(),
                offset: 0,
                limit: None,
            },
        );
        for request in [peek.clone(), read] {
            assert_ownership_denied(
                room.router
                    .dispatch(terminal_command(&request, Some("private-user")), request)
                    .await
                    .unwrap_err(),
                "private-user",
                DEFAULT_LOCAL_USER_ID,
            );
        }
        let grant = room
            .router
            .runtime_state()
            .insert_access_grant_for_test(&room.session_id);
        for agent in [room.agent_id.as_str(), "agent-from-another-room"] {
            let mut request = peek.clone();
            if let LocalDaemonRequest::PeekAgentTranscript(peek) = &mut request {
                peek.agent_id = agent.into();
            }
            let mut command = terminal_command(&request, Some(DEFAULT_LOCAL_USER_ID));
            command.caller.connection_class =
                Some(crate::local::KernelConnectionClass::ExternalAgent);
            command.caller.caller_id = grant.clone();
            room.router
                .authorize_external_request(&command, &request)
                .unwrap();
            let response = room.router.dispatch(command, request).await;
            if agent == room.agent_id.as_str() {
                assert!(matches!(
                    response.unwrap(),
                    LocalDaemonResponse::AgentTranscriptPeek { .. }
                ));
            } else {
                assert!(
                    response.is_err(),
                    "a grant must not resolve an agent from outside the requested room"
                );
            }
        }
    });
}
