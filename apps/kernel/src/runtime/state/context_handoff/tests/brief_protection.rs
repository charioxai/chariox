use super::*;

struct TestRoot(std::path::PathBuf);
impl TestRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "chariox-brief-protection-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const SECRET: &str = "synthetic-brief-secret";
const SAFE_BRIEF: &str = "## Goal\nContinue safely\n## Constraints & Preferences\nNone\n## Progress\nDone\n## Key Decisions\nNone\n## Next Steps\nTest\n## Critical Context\nNone";

async fn fixture(recovered: bool) -> (DerivedHandoffFixture, TestRoot) {
    let mut fixture = DerivedHandoffFixture::new().await;
    let root = TestRoot::new();
    fixture.append(
        1,
        crate::history::SessionHistoryEntry::user_prompt(
            &fixture.session_id,
            "attachment",
            &fixture.agent_id,
            SECRET,
        ),
        Default::default(),
        Some(1),
    );
    fixture.output_at(2, "old", "codex", Some("old-thread"), "OK", Some(1));
    fixture
        .history
        .save_agent_handoff_brief(
            &fixture.session_id,
            &fixture.agent_id,
            &crate::history::AgentHandoffBrief {
                brief: format!("## Goal\n{SECRET}\n## Next Steps\nTest"),
                covered_through_sequence: 2,
            },
        )
        .unwrap();
    if recovered {
        fixture.runtime.owned.room_secret_observations =
            crate::runtime::state::room_secret_observation::RoomSecretObservations::new(
                root.path().join("observations"),
                std::collections::BTreeSet::from([fixture.session_id.clone()]),
            );
    } else {
        fixture
            .runtime
            .owned
            .room_secret_observations
            .register(&fixture.session_id, SECRET)
            .unwrap();
    }
    (fixture, root)
}

async fn assert_direct_and_no_new_history(recovered: bool) {
    let (fixture, _root) = fixture(recovered).await;
    let direct = fixture
        .dispatch(&fixture.run("direct", "opencode", None), 3)
        .unwrap();
    assert!(
        !direct.contains(SECRET),
        "direct handoff replayed protected brief: {direct}"
    );
    let target = fixture.run("refresh", "claude", None);
    let handoff = fixture
        .runtime
        .owned
        .context_handoff_for_dispatch(
            &fixture.session_id,
            &fixture.agent_id,
            &target,
            "prompt-next",
            false,
        )
        .unwrap();
    let refreshed = fixture
        .runtime
        .with_current_handoff_brief(
            &fixture.runtime.owned,
            handoff,
            &fixture.session_id,
            &fixture.agent_id,
            "prompt-next",
            &target,
            || true,
        )
        .await;
    let rendered = refreshed.render(refreshed.budget()).unwrap();
    assert!(
        !rendered.contains(SECRET),
        "unchanged/fallback brief replayed protected text"
    );
    if recovered {
        assert!(
            fixture
                .history
                .load_agent_handoff_brief(&fixture.session_id, &fixture.agent_id)
                .unwrap()
                .is_none(),
            "rejected cache must also lose its usable watermark"
        );
    }
}

#[tokio::test]
async fn persisted_brief_reuse_applies_later_secret_registration() {
    assert_direct_and_no_new_history(false).await;
}

#[tokio::test]
async fn persisted_brief_reuse_discards_recovered_cache_and_watermark() {
    assert_direct_and_no_new_history(true).await;
}

async fn assert_utility_input(recovered: bool) {
    let (fixture, root) = fixture(recovered).await;
    fixture.append(
        3,
        crate::history::SessionHistoryEntry::user_prompt(
            &fixture.session_id,
            "attachment",
            &fixture.agent_id,
            &format!("fresh benign history {}", "notes ".repeat(500)),
        ),
        Default::default(),
        Some(fixture.runtime.owned.room_secret_observations.epoch + 1),
    );
    let captured = root.path().join("utility-input");
    let program = root.path().join("brief-fixture");
    let answer = serde_json::json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":SAFE_BRIEF}]}}).to_string();
    std::fs::write(&program, format!(
        "#!/bin/sh\nwhile IFS= read -r line; do printf '%s\\n' \"$line\" >> '{}'; printf '%s\\n' '{}' '{{\"type\":\"result\",\"subtype\":\"success\"}}'; done\n",
        captured.display(), answer,
    )).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    let request = crate::provider::LaunchProviderRequest::new(
        &fixture.session_id,
        "claude",
        "claude",
        "default",
        "sonnet",
    )
    .with_agent_id(&fixture.agent_id);
    let mut target = RuntimeProviderRun::new(
        "brief-protection-run",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "fixture".into(),
            pty_target: None,
            pty_program: Some(program.display().to_string()),
            pty_args: vec![],
            pty_env: Default::default(),
            pty_env_remove: vec![],
            working_directory: None,
            structured_endpoint: None,
        },
    );
    target.mark_running();
    fixture
        .runtime
        .owned
        .provider_store
        .write()
        .insert_run_for_test(target.clone());
    fixture
        .runtime
        .owned
        .provider_store
        .initialize_runtime(&target)
        .unwrap();
    fixture
        .runtime
        .owned
        .provider_run_projection
        .update(target.clone());
    let handoff = fixture
        .runtime
        .owned
        .context_handoff_for_dispatch(
            &fixture.session_id,
            &fixture.agent_id,
            &target,
            "prompt-next",
            false,
        )
        .unwrap();
    let handoff = fixture
        .runtime
        .with_current_handoff_brief(
            &fixture.runtime.owned,
            handoff,
            &fixture.session_id,
            &fixture.agent_id,
            "prompt-next",
            &target,
            || true,
        )
        .await;
    fixture
        .runtime
        .owned
        .provider_store
        .terminate_run_provider_only(&fixture.session_id, target.id())
        .unwrap();
    let input = std::fs::read_to_string(captured).expect("must reach actual utility input");
    assert!(
        !input.contains(SECRET),
        "utility replayed protected cache: {input}"
    );
    assert!(!handoff.render(handoff.budget()).unwrap().contains(SECRET));
    assert!(input.contains("fresh benign history"));
    if recovered {
        assert!(
            input.contains("recovered sensitive Room history withheld"),
            "reset watermark must rebuild protected earlier history"
        );
    }
}

#[tokio::test]
async fn persisted_brief_utility_input_applies_later_secret_registration() {
    assert_utility_input(false).await;
}

#[tokio::test]
async fn persisted_brief_utility_input_rebuilds_after_room_recovery() {
    assert_utility_input(true).await;
}

#[tokio::test]
async fn persisted_brief_reuse_discards_a_fenced_registry_cache() {
    let (mut fixture, root) = fixture(false).await;
    use crate::runtime::state::room_secret_observation::RoomSecretObservations;
    let path = root.path().join("fenced-observations");
    let observations = RoomSecretObservations::new(path.clone(), Default::default());
    observations.register(&fixture.session_id, SECRET).unwrap();
    // No sealed registry: a later boot must fence this marked Room.
    fixture.runtime.owned.room_secret_observations =
        RoomSecretObservations::new(path, Default::default());
    let direct = fixture
        .dispatch(&fixture.run("direct-fenced", "opencode", None), 3)
        .unwrap();
    assert!(!direct.contains(SECRET));
    assert!(fixture
        .history
        .load_agent_handoff_brief(&fixture.session_id, &fixture.agent_id)
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn persisted_brief_refresh_error_fallback_is_currently_protected() {
    for recovered in [false, true] {
        let (fixture, _root) = fixture(recovered).await;
        fixture.user(3, &format!("more history {}", "notes ".repeat(500)));
        let target = fixture.run("unavailable-runtime", "claude", None);
        let handoff = fixture
            .runtime
            .owned
            .context_handoff_for_dispatch(
                &fixture.session_id,
                &fixture.agent_id,
                &target,
                "prompt-next",
                false,
            )
            .unwrap();
        // Missing target actor makes the actual utility refresh fail.
        let handoff = fixture
            .runtime
            .with_current_handoff_brief(
                &fixture.runtime.owned,
                handoff,
                &fixture.session_id,
                &fixture.agent_id,
                "prompt-next",
                &target,
                || true,
            )
            .await;
        assert!(!handoff.render(handoff.budget()).unwrap().contains(SECRET));
        assert_eq!(handoff.conversation.brief.is_none(), recovered);
    }
}
