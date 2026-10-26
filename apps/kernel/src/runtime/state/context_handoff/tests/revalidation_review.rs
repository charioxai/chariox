//! MP-08 / MP-10 / MP-11: restored policy, bounded explicit history, prepared briefs.
use super::brief_protection::TestRoot;
use super::*;
use crate::runtime::state::room_secret_observation::RoomSecretObservations;

const SECRET: &str = "synthetic-handoff-observation";
const FRESH: &str = "## Goal\nFresh fold amber\n## Next Steps\nContinue";

// MP-08 / MP-10 / MP-11: newly retained earlier tool summaries share final protection.
#[tokio::test]
async fn fitting_earlier_tool_summary_is_reprotected_before_submission() {
    let (fixture, _root) = protected_fixture(false).await;
    let late_secret = "late-tool-observation";
    fixture.append(
        3,
        crate::history::SessionHistoryEntry::provider_output(
            &fixture.session_id,
            "source",
            Some(&fixture.agent_id),
            crate::terminal::TerminalOutputKind::ProviderTool,
            None,
            serde_json::json!({"id":"read-1","tool":"bash","status":"completed",
                "input":{"command":"read public source"},
                "output":format!("benign-tool-marker {late_secret}")})
            .to_string(),
        ),
        Default::default(),
        None,
    );
    fixture.user(4, "Continue the audit");
    fixture.output(5, "source", "codex", Some("source-thread"), "Audited");
    let target = fixture.run("new", "codex", None);
    let handoff = fixture
        .runtime
        .owned
        .context_handoff_for_dispatch(
            &fixture.session_id,
            &fixture.agent_id,
            &target,
            "next",
            false,
        )
        .unwrap()
        .unwrap();
    assert!(!handoff.needs_brief());
    assert!(handoff
        .render(handoff.budget())
        .unwrap()
        .contains(late_secret));
    fixture
        .runtime
        .owned
        .room_secret_observations
        .register(&fixture.session_id, late_secret)
        .unwrap();
    let replay = fixture
        .runtime
        .owned
        .revalidate_context_handoff(handoff, &fixture.session_id, &fixture.agent_id, "next")
        .unwrap();
    let packet = replay.render(replay.budget()).unwrap();
    assert!(packet.contains("benign-tool-marker"));
    assert!(!packet.contains(late_secret));
}

async fn protected_fixture(recovered: bool) -> (DerivedHandoffFixture, TestRoot) {
    let mut fixture = DerivedHandoffFixture::new().await;
    let root = TestRoot::new();
    let key = crate::transport::relay_crypto::generate_private_key_base64();
    let location = root.path().join("observations");
    let store =
        RoomSecretObservations::new(location.clone(), Default::default()).with_identity(&key);
    store.register(&fixture.session_id, SECRET).unwrap();
    fixture.runtime.owned.room_secret_observations = if recovered {
        RoomSecretObservations::new(location, Default::default()).with_identity(&key)
    } else {
        store
    };
    fixture
        .runtime
        .owned
        .room_secret_observations
        .require(&fixture.session_id, false)
        .unwrap();
    fixture.user(1, "Retain amber fact");
    fixture.output(
        2,
        "source",
        "codex",
        Some("source-thread"),
        "Amber retained",
    );
    (fixture, root)
}

fn explicit(fixture: &DerivedHandoffFixture, fork: bool) -> (PendingAgentContextHandoff, String) {
    let source = test_run(
        "source",
        &fixture.session_id,
        &fixture.agent_id,
        "codex",
        "gpt-6",
    );
    let agent_id = if fork {
        "fork-target"
    } else {
        &fixture.agent_id
    };
    let target = test_run("target", &fixture.session_id, agent_id, "opencode", "gpt-6");
    if fork {
        fixture
            .runtime
            .owned
            .prepare_agent_fork_context_handoff(&source, agent_id, &target);
    } else {
        fixture
            .runtime
            .owned
            .prepare_turn_substitute_context_handoff(
                &source,
                Some(target.id()),
                target.provider(),
                target.account_profile(),
                Some(target.model()),
            );
    }
    (
        fixture
            .runtime
            .owned
            .pending_agent_context_handoffs
            .peek_matching(&fixture.session_id, agent_id, &target)
            .unwrap(),
        agent_id.into(),
    )
}

#[tokio::test]
async fn explicit_fork_and_substitute_snapshots_captured_after_restore_are_delivered() {
    for fork in [false, true] {
        let (fixture, _root) = protected_fixture(true).await;
        assert!(fixture
            .runtime
            .owned
            .room_secret_observations
            .scrub_cached_result(&fixture.session_id, ())
            .is_err());
        let (snapshot, agent) = explicit(&fixture, fork);
        let replay = fixture.runtime.owned.revalidate_context_handoff(
            snapshot,
            &fixture.session_id,
            &agent,
            "next",
        );
        assert!(
            replay.is_ok(),
            "fresh protected explicit snapshot rejected after restore: {replay:?}"
        );
        assert!(replay
            .unwrap()
            .render(MAX_HANDOFF_BYTES)
            .unwrap()
            .contains("amber"));
    }
}

#[tokio::test]
async fn registering_a_known_value_keeps_the_pending_snapshot_revision() {
    let (fixture, _root) = protected_fixture(false).await;
    let (snapshot, agent) = explicit(&fixture, false);
    fixture
        .runtime
        .owned
        .room_secret_observations
        .register(&fixture.session_id, SECRET)
        .unwrap();
    assert_eq!(
        snapshot.room_revision,
        Some(
            fixture
                .runtime
                .owned
                .room_secret_observations
                .revision(&fixture.session_id)
                .unwrap()
        )
    );
    assert!(fixture
        .runtime
        .owned
        .revalidate_context_handoff(snapshot, &fixture.session_id, &agent, "next")
        .is_ok());
}

#[tokio::test]
async fn real_registration_rebuilds_fork_and_substitute_at_the_original_boundary() {
    for fork in [false, true] {
        let (fixture, _root) = protected_fixture(false).await;
        let new_secret = "new-observation-".repeat(200);
        fixture.user(3, &format!("Keep amber and {new_secret}"));
        let (snapshot, agent) = explicit(&fixture, fork);
        fixture.user(4, "AFTER-CAPTURE-MUST-NOT-REPLAY");
        fixture
            .runtime
            .owned
            .room_secret_observations
            .register(&fixture.session_id, &new_secret)
            .unwrap();
        let replay = fixture
            .runtime
            .owned
            .revalidate_context_handoff(snapshot, &fixture.session_id, &agent, "next")
            .unwrap();
        let packet = replay.render(MAX_HANDOFF_BYTES).unwrap();
        assert!(packet.contains("amber"));
        assert!(
            !packet.contains("new-observation-"),
            "registered value was clipped before protection"
        );
        assert!(
            !packet.contains("AFTER-CAPTURE-MUST-NOT-REPLAY"),
            "snapshot advanced past the fork/substitute boundary"
        );
    }
}

#[tokio::test]
async fn restored_room_final_packet_preserves_the_freshly_prepared_brief() {
    let (fixture, _root) = protected_fixture(true).await;
    let target = fixture.run("new", "claude", None);
    let mut handoff = fixture
        .runtime
        .owned
        .context_handoff_for_dispatch(
            &fixture.session_id,
            &fixture.agent_id,
            &target,
            "next",
            false,
        )
        .unwrap()
        .unwrap();
    handoff.conversation.brief = Some(FRESH.into());
    fixture
        .history
        .save_agent_handoff_brief(
            &fixture.session_id,
            &fixture.agent_id,
            &crate::history::AgentHandoffBrief {
                brief: FRESH.into(),
                covered_through_sequence: 2,
            },
        )
        .unwrap();
    let handoff = fixture
        .runtime
        .owned
        .revalidate_context_handoff(handoff, &fixture.session_id, &fixture.agent_id, "next")
        .unwrap();
    assert!(handoff
        .render(MAX_HANDOFF_BYTES)
        .unwrap()
        .contains("Fresh fold amber"));
}

#[tokio::test]
async fn fitting_structured_packet_never_acquires_a_stale_stored_brief_on_revalidation() {
    let (fixture, _root) = protected_fixture(false).await;
    fixture
        .history
        .save_agent_handoff_brief(
            &fixture.session_id,
            &fixture.agent_id,
            &crate::history::AgentHandoffBrief {
                brief: "## Goal\nSTALE-BRIEF\n## Next Steps\nWait".into(),
                covered_through_sequence: 1,
            },
        )
        .unwrap();
    let target = fixture.run("new", "codex", None);
    let handoff = fixture
        .runtime
        .owned
        .context_handoff_for_dispatch(
            &fixture.session_id,
            &fixture.agent_id,
            &target,
            "next",
            false,
        )
        .unwrap()
        .unwrap();
    assert!(handoff.conversation.brief.is_none());
    let handoff = fixture
        .runtime
        .owned
        .revalidate_context_handoff(handoff, &fixture.session_id, &fixture.agent_id, "next")
        .unwrap();
    assert!(
        handoff.conversation.brief.is_none(),
        "revalidation inserted a stale storage brief"
    );
}

// MP-08 / MP-10 / MP-11: retries carry their request only through active dispatch.
#[tokio::test]
async fn substitute_policy_rebuild_excludes_the_dispatched_request() {
    for attempted in [false, true] {
        let (fixture, _root) = protected_fixture(false).await;
        fixture.user(3, "CURRENT-SUBSTITUTE-REQUEST");
        if attempted {
            fixture.error(4, "source", "INTERRUPTED-ATTEMPT-DETAIL");
        }
        let (_, agent) = explicit(&fixture, false);
        let target = test_run("target", &fixture.session_id, &agent, "opencode", "gpt-6");
        let snapshot = fixture
            .runtime
            .owned
            .context_handoff_for_dispatch(&fixture.session_id, &agent, &target, "prompt-3", false)
            .unwrap()
            .unwrap();
        assert!(!snapshot
            .render(MAX_HANDOFF_BYTES)
            .unwrap()
            .contains("CURRENT-SUBSTITUTE-REQUEST"));
        fixture
            .runtime
            .owned
            .room_secret_observations
            .register(&fixture.session_id, "NEW-POLICY-OBSERVATION")
            .unwrap();
        let replay = fixture
            .runtime
            .owned
            .revalidate_context_handoff(snapshot, &fixture.session_id, &agent, "prompt-3")
            .unwrap()
            .render(MAX_HANDOFF_BYTES)
            .unwrap();
        assert!(
            !replay.contains("CURRENT-SUBSTITUTE-REQUEST"),
            "policy rebuild replayed the request: {replay}"
        );
        assert!(replay.contains("amber"));
        assert_eq!(replay.contains("INTERRUPTED-ATTEMPT-DETAIL"), attempted);
    }
}
