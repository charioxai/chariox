//! MP-08 / MP-10 / MP-11: supplementary restart and Claude-mode compaction recovery regressions.
use super::*;

#[tokio::test]
async fn compacted_native_session_recovers_omitted_records_after_fresh_kernel_bootstrap() {
    let fixture = DerivedHandoffFixture::new().await;
    fixture.user(1, "Preserve the original named proposal.");
    fixture.output(
        2,
        "source",
        "claude",
        Some("native"),
        "Original proposal: Durable Amber Ledger.",
    );
    fixture.user(3, "Continue the latest task.");
    fixture.output(
        4,
        "source",
        "claude",
        Some("native"),
        "Latest progress without the original proposal.",
    );
    let source = fixture
        .run("source", "claude", Some("native"))
        .projected_for_home_agent_with_id(
            "source",
            fixture.session_id.clone(),
            fixture.agent_id.clone(),
        );
    fixture
        .runtime
        .owned
        .prepare_compacted_session_context_handoff(&source)
        .unwrap();
    let config = fixture.runtime.owned.config_projection.snapshot();
    let mut target = fixture
        .run("after-restart", "claude", Some("native"))
        .projected_for_home_agent_with_id(
            "after-restart",
            fixture.session_id.clone(),
            fixture.agent_id.clone(),
        );
    target.set_model("haiku");
    let session_id = fixture.session_id.clone();
    let agent_id = fixture.agent_id.clone();
    // A cold bootstrap must release the previous durable-state owner.
    drop(fixture);
    let restarted_app = crate::DaemonApp::bootstrap(config).unwrap();
    let restarted = owned_runtime_state(&Arc::new(Mutex::new(restarted_app))).await;
    let handoff = restarted.owned.context_handoff_for_dispatch(
        &session_id, &agent_id, &target, "after-restart-turn", false,
    ).unwrap().expect("fresh kernel lost the post-compaction recovery boundary for the retained native session");
    assert!(handoff
        .render_hidden(24_000)
        .contains("Durable Amber Ledger"));
    restarted
        .owned
        .consume_pending_context_handoff(&session_id, &agent_id, &target);
    assert!(
        restarted
            .owned
            .context_handoff_for_dispatch(&session_id, &agent_id, &target, "following-turn", false,)
            .unwrap()
            .is_none(),
        "delivered recovery must not repeat"
    );
}

#[tokio::test]
async fn compacted_session_recovery_matches_destination_claude_mode() {
    let fixture = DerivedHandoffFixture::new().await;
    fixture.user(1, "Retain the pre-compaction named proposal.");
    fixture.output(
        2,
        "source",
        "claude-p",
        Some("native"),
        "Original proposal: Mode Amber Ledger.",
    );
    let source = fixture
        .run("source", "claude-p", Some("native"))
        .projected_for_home_agent_with_id(
            "source",
            fixture.session_id.clone(),
            fixture.agent_id.clone(),
        );
    fixture
        .runtime
        .owned
        .prepare_compacted_session_context_handoff(&source)
        .unwrap();
    let mut target = fixture
        .run("target", "claude", Some("native"))
        .projected_for_home_agent_with_id(
            "target",
            fixture.session_id.clone(),
            fixture.agent_id.clone(),
        );
    target.set_model("haiku");
    let handoff = fixture
        .runtime
        .owned
        .context_handoff_for_dispatch(
            &fixture.session_id,
            &fixture.agent_id,
            &target,
            "mode-change-turn",
            false,
        )
        .unwrap()
        .expect("destination Claude mode rejected the compacted native-session recovery");
    assert!(handoff.render_hidden(24_000).contains("Mode Amber Ledger"));
}
