use super::*;

#[test]
fn codex_and_opencode_require_explicit_completion() {
    assert!(ExternalProviderObservationPolicy::for_provider("codex").uses_explicit_completion());
    assert!(ExternalProviderObservationPolicy::for_provider("opencode").uses_explicit_completion());
    assert!(!ExternalProviderObservationPolicy::for_provider("claude").uses_explicit_completion());
    assert!(ExternalProviderObservationPolicy::for_provider(" Codex ").uses_explicit_completion());
}

#[test]
fn completion_and_abort_statuses_settle_turns() {
    for (provider, text) in [
        ("codex", "codex task_complete\n{}"),
        (
            "codex",
            "codex event turn_aborted {\"type\":\"turn_aborted\"}",
        ),
        ("claude", "claude message completed\n{}"),
        ("opencode", "opencode message completed\n{}"),
    ] {
        let policy = ExternalProviderObservationPolicy::for_provider(provider);
        assert!(
            policy.latest_effective_turn_settles(&[ObservedExternalProviderTurn {
                role: ObservedExternalProviderTurnRole::Status,
                text: text.to_string(),
                provider_turn_id: None,
                observed_at_ms: None,
            }]),
            "{provider} status should settle"
        );
        assert_eq!(
            policy
                .observation_for_turn(&ObservedExternalProviderTurn {
                    role: ObservedExternalProviderTurnRole::Status,
                    text: text.to_string(),
                    provider_turn_id: None,
                    observed_at_ms: None,
                })
                .map(|observation| observation.settles_active_prompt),
            Some(true),
            "{provider} status should be marked as settling"
        );
        assert_eq!(
            policy
                .observation_for_turn(&ObservedExternalProviderTurn {
                    role: ObservedExternalProviderTurnRole::Status,
                    text: text.to_string(),
                    provider_turn_id: None,
                    observed_at_ms: None,
                })
                .map(|observation| observation.passive_telemetry),
            Some(false),
            "{provider} settling status should not be passive telemetry"
        );
    }
}

#[test]
fn completion_statuses_are_scoped_to_provider_policy() {
    for (provider, foreign_text) in [
        ("codex", "claude message completed\n{}"),
        ("codex", "opencode message completed\n{}"),
        ("claude", "codex task_complete\n{}"),
        (
            "claude",
            "codex event turn_aborted {\"type\":\"turn_aborted\"}",
        ),
        ("claude", "opencode message completed\n{}"),
        ("opencode", "codex task_complete\n{}"),
        ("opencode", "claude message completed\n{}"),
    ] {
        assert!(
            !ExternalProviderObservationPolicy::for_provider(provider)
                .latest_effective_turn_settles(&[ObservedExternalProviderTurn {
                    role: ObservedExternalProviderTurnRole::Status,
                    text: foreign_text.to_string(),
                    provider_turn_id: None,
                    observed_at_ms: None,
                }]),
            "{provider} policy must not settle from foreign marker {foreign_text:?}"
        );
    }
}

#[test]
fn status_prefix_markers_require_boundaries() {
    let codex = ExternalProviderObservationPolicy::for_provider("codex");
    assert!(codex.status_settles("codex task_complete\n{}"));
    assert!(!codex.status_settles("codex task_completed\n{}"));
    assert!(codex.status_is_passive_telemetry("codex token_count\n{}"));
    assert!(!codex.status_is_passive_telemetry("codex token_count_extra\n{}"));

    let claude = ExternalProviderObservationPolicy::for_provider("claude");
    assert!(claude.status_settles("claude message completed {}"));
    assert!(!claude.status_settles("claude message completedness {}"));
    assert!(claude.status_is_passive_telemetry("claude ai-title {}"));
    assert!(!claude.status_is_passive_telemetry("claude ai-title-extra {}"));

    let opencode = ExternalProviderObservationPolicy::for_provider("opencode");
    assert!(opencode.status_settles("opencode message completed {}"));
    assert!(!opencode.status_settles("opencode message completedness {}"));
}

#[test]
fn provider_policy_tolerates_legacy_provider_casing_and_whitespace() {
    let codex = ExternalProviderObservationPolicy::for_provider(" Codex ");
    assert!(codex.status_settles(" Codex task_complete\n{}"));
    assert!(codex.status_is_passive_telemetry(" CODEX token_count\n{}"));

    let claude = ExternalProviderObservationPolicy::for_provider(" CLAUDE ");
    assert!(claude.status_settles(" Claude message completed\n{}"));
    assert!(claude.status_is_passive_telemetry(" CLAUDE last-prompt {\"lastPrompt\":\"prompt\"}"));

    let opencode = ExternalProviderObservationPolicy::for_provider(" OpenCode ");
    assert!(opencode.status_settles(" OpenCode message completed\n{}"));
}

#[test]
fn codex_token_count_status_projects_provider_run_usage() {
    assert_eq!(
        ExternalProviderObservationPolicy::for_provider("codex").status_usage(
            " Codex token_count\n{\"info\":{\"total_token_usage\":{\"total_tokens\":42000},\"model_context_window\":128000}}"
        ),
        Some(ProviderRunTokenUsage {
            total_tokens: Some(42_000),
            last_tokens: Some(42_000),
            context_tokens: Some(42_000),
            context_window: Some(128_000),
        })
    );
    assert_eq!(
        ExternalProviderObservationPolicy::for_provider("codex").status_usage(
            "codex token_count\n{\"last\":{\"totalTokens\":160000},\"modelContextWindow\":128000}"
        ),
        Some(ProviderRunTokenUsage {
            total_tokens: Some(160_000),
            last_tokens: Some(160_000),
            context_tokens: None,
            context_window: Some(128_000),
        })
    );
    assert_eq!(
        ExternalProviderObservationPolicy::for_provider("claude")
            .status_usage("codex token_count\n{\"last\":{\"totalTokens\":42}}"),
        None
    );
}

#[test]
fn claude_passive_telemetry_does_not_hide_prior_completion() {
    let policy = ExternalProviderObservationPolicy::for_provider("claude");
    assert!(
        policy.turn_is_passive_telemetry(&ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::Status,
            text: "claude last-prompt {\"lastPrompt\":\"prompt\"}".to_string(),
            provider_turn_id: None,
            observed_at_ms: None,
        })
    );
    assert_eq!(
        policy
            .observation_for_turn(&ObservedExternalProviderTurn {
                role: ObservedExternalProviderTurnRole::Status,
                text: "claude last-prompt {\"lastPrompt\":\"prompt\"}".to_string(),
                provider_turn_id: None,
                observed_at_ms: None,
            })
            .map(|observation| observation.passive_telemetry),
        Some(true)
    );
    assert!(policy.latest_effective_turn_settles(&[
        ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::Status,
            text: "claude message completed\n{}".to_string(),
            provider_turn_id: None,
            observed_at_ms: None,
        },
        ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::Status,
            text: "claude ai-title {\"title\":\"Title\"}".to_string(),
            provider_turn_id: None,
            observed_at_ms: None,
        },
    ]));
}

#[test]
fn codex_token_count_is_passive_telemetry_and_does_not_settle() {
    let policy = ExternalProviderObservationPolicy::for_provider("codex");
    let token_count = ObservedExternalProviderTurn {
        role: ObservedExternalProviderTurnRole::Status,
        text: "codex token_count\n{\"info\":{\"total_token_usage\":{\"total_tokens\":42}}}"
            .to_string(),
        provider_turn_id: None,
        observed_at_ms: None,
    };

    assert!(policy.turn_is_passive_telemetry(&token_count));
    assert!(!policy.latest_effective_turn_settles(std::slice::from_ref(&token_count)));
    assert_eq!(
        policy
            .observation_for_turn(&token_count)
            .map(|observation| observation.passive_telemetry),
        Some(true)
    );
}

#[test]
fn normalized_observed_prompt_text_collapses_whitespace_and_ignores_empty() {
    assert_eq!(
        normalized_observed_prompt_text("  run   this\nnow\t"),
        Some("run this now".to_string())
    );
    assert_eq!(normalized_observed_prompt_text(" \n\t "), None);
}

#[test]
fn normalized_observed_prompt_text_ignores_generated_attachment_markup() {
    assert_eq!(
        normalized_observed_prompt_text(
            "inspect this\n<image name=[Image #1] path=\"/tmp/screenshot.png\"> </image>"
        ),
        Some("inspect this".to_string())
    );
    assert_eq!(
        normalized_observed_prompt_text(
            "read this <file name=\"notes.txt\" path=\"/tmp/notes.txt\"> </file> now"
        ),
        Some("read this now".to_string())
    );
}

#[test]
fn clean_provider_prompt_strips_system_wrappers_and_compacts_request_text() {
    assert_eq!(
        clean_provider_prompt(
            "# AGENTS.md instructions for /repo\n\n<INSTRUCTIONS>hidden</INSTRUCTIONS>".to_string()
        ),
        None
    );
    assert_eq!(
        clean_provider_prompt("<environment_context>\n  <cwd>/repo</cwd>".to_string()),
        None
    );
    assert_eq!(
        clean_provider_prompt(
            "preamble\n## My request for Codex:\n  run   the\ncheck  ".to_string()
        ),
        Some("run the check".to_string())
    );
    assert_eq!(
        clean_provider_prompt("meta\n## My request:\n  use   provider form  ".to_string()),
        Some("use provider form".to_string())
    );
}

#[test]
fn observed_turn_text_cleanup_is_role_specific() {
    assert_eq!(
        clean_observed_turn_text(Some("user"), "  ask   this\nnow ".to_string()),
        Some("ask this now".to_string())
    );
    assert_eq!(
        clean_observed_turn_text(Some("assistant"), "  final answer\n".to_string()),
        Some("final answer".to_string())
    );
    assert_eq!(
        clean_observed_turn_text(Some("status"), "  codex task_complete {}\n".to_string()),
        Some("codex task_complete {}".to_string())
    );
    assert_eq!(
        clean_observed_turn_text(Some("unknown"), "text".to_string()),
        None
    );
}

#[test]
fn text_from_content_extracts_provider_content_shapes() {
    assert_eq!(
        text_from_content(&serde_json::json!("plain text")),
        Some("plain text".to_string())
    );
    assert_eq!(
        text_from_content(&serde_json::json!([
            {"type": "text", "text": "first"},
            {"type": "image", "url": "ignored"},
            {"type": "text", "content": "second"},
            {"value": "third"}
        ])),
        Some("first\nsecond\nthird".to_string())
    );
    assert_eq!(
        text_from_content(&serde_json::json!({"content": "object content"})),
        Some("object content".to_string())
    );
}

#[test]
fn active_external_prompt_turn_uses_latest_user_until_explicit_settlement() {
    let policy = ExternalProviderObservationPolicy::for_provider("codex");
    let turns = vec![
        ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::User,
            text: "first prompt".to_string(),
            provider_turn_id: Some("user-1".to_string()),
            observed_at_ms: None,
        },
        ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::Assistant,
            text: "working".to_string(),
            provider_turn_id: Some("assistant-1".to_string()),
            observed_at_ms: None,
        },
    ];

    let latest = policy
        .active_external_prompt_turn(&turns, false, &BTreeSet::new())
        .expect("codex should stay active before explicit completion");

    assert_eq!(latest.provider_turn_id.as_deref(), Some("user-1"));
    let settled = [
        turns,
        vec![ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::Status,
            text: "codex task_complete {}".to_string(),
            provider_turn_id: Some("done".to_string()),
            observed_at_ms: None,
        }],
    ]
    .concat();
    assert!(
        policy
            .active_external_prompt_turn(&settled, false, &BTreeSet::new())
            .is_none()
    );
}

#[test]
fn active_external_prompt_turn_filters_arroba_owned_provider_turn_ids() {
    let policy = ExternalProviderObservationPolicy::for_provider("claude");
    let mut arroba_owned = BTreeSet::new();
    arroba_owned.insert("user-1".to_string());

    assert!(
        policy
            .active_external_prompt_turn(
                &[ObservedExternalProviderTurn {
                    role: ObservedExternalProviderTurnRole::User,
                    text: "same   prompt".to_string(),
                    provider_turn_id: Some("user-1".to_string()),
                    observed_at_ms: None,
                }],
                true,
                &arroba_owned,
            )
            .is_none()
    );
}

#[test]
fn active_prompt_sync_settles_claude_stable_assistant_after_quiet_poll() {
    let policy = ExternalProviderObservationPolicy::for_provider("claude");
    let turns = vec![
        ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::User,
            text: "prompt".to_string(),
            provider_turn_id: Some("user-1".to_string()),
            observed_at_ms: None,
        },
        ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::Assistant,
            text: "final answer".to_string(),
            provider_turn_id: Some("assistant-1".to_string()),
            observed_at_ms: None,
        },
    ];

    let first_poll = policy.active_prompt_sync(&turns, 2, 2, true, &BTreeSet::new());
    assert!(first_poll.should_sync_active_prompt);
    assert_eq!(
        first_poll
            .active_prompt_turn
            .and_then(|turn| turn.provider_turn_id.as_deref()),
        Some("user-1")
    );

    let quiet_poll = policy.active_prompt_sync(&turns, 0, 0, true, &BTreeSet::new());
    assert!(quiet_poll.should_sync_active_prompt);
    assert!(quiet_poll.active_prompt_turn.is_none());
    assert!(!quiet_poll.latest_observation_settles);
}

#[test]
fn active_prompt_sync_keeps_codex_active_until_explicit_completion() {
    let policy = ExternalProviderObservationPolicy::for_provider("codex");
    let turns = vec![
        ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::User,
            text: "prompt".to_string(),
            provider_turn_id: Some("user-1".to_string()),
            observed_at_ms: None,
        },
        ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::Assistant,
            text: "intermediate answer".to_string(),
            provider_turn_id: Some("assistant-1".to_string()),
            observed_at_ms: None,
        },
    ];

    let quiet_poll = policy.active_prompt_sync(&turns, 0, 0, true, &BTreeSet::new());
    assert!(quiet_poll.should_sync_active_prompt);
    assert_eq!(
        quiet_poll
            .active_prompt_turn
            .and_then(|turn| turn.provider_turn_id.as_deref()),
        Some("user-1")
    );
    assert!(!quiet_poll.latest_observation_settles);

    let completed_turns = [
        turns,
        vec![ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::Status,
            text: "codex task_complete\n{}".to_string(),
            provider_turn_id: Some("task-complete-1".to_string()),
            observed_at_ms: None,
        }],
    ]
    .concat();
    let completion = policy.active_prompt_sync(&completed_turns, 1, 1, true, &BTreeSet::new());
    assert!(completion.should_sync_active_prompt);
    assert!(completion.active_prompt_turn.is_none());
    assert!(completion.latest_observation_settles);
}

#[test]
fn active_prompt_sync_ignores_passive_telemetry_only_poll() {
    let policy = ExternalProviderObservationPolicy::for_provider("codex");
    let turns = vec![ObservedExternalProviderTurn {
        role: ObservedExternalProviderTurnRole::Status,
        text: "codex token_count\n{\"info\":{\"total_token_usage\":{\"total_tokens\":42}}}"
            .to_string(),
        provider_turn_id: Some("usage-1".to_string()),
        observed_at_ms: None,
    }];

    let sync = policy.active_prompt_sync(&turns, 1, 0, true, &BTreeSet::new());
    assert!(!sync.should_sync_active_prompt);
    assert!(sync.active_prompt_turn.is_none());
    assert!(!sync.latest_observation_settles);
}

#[test]
fn active_prompt_sync_does_not_stably_settle_arroba_owned_echoes() {
    let policy = ExternalProviderObservationPolicy::for_provider("codex");
    let turns = vec![
        ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::User,
            text: "arroba prompt".to_string(),
            provider_turn_id: Some("user-owned".to_string()),
            observed_at_ms: None,
        },
        ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::Assistant,
            text: "intermediate response".to_string(),
            provider_turn_id: Some("assistant-owned".to_string()),
            observed_at_ms: None,
        },
    ];
    let mut arroba_owned = BTreeSet::new();
    arroba_owned.insert("user-owned".to_string());

    let sync = policy.active_prompt_sync(&turns, 0, 0, true, &arroba_owned);

    assert!(!sync.should_sync_active_prompt);
    assert!(sync.active_prompt_turn.is_none());
    assert!(!sync.latest_observation_settles);
}

#[test]
fn active_prompt_sync_preserves_explicit_settlement_for_arroba_owned_echoes() {
    let policy = ExternalProviderObservationPolicy::for_provider("codex");
    let turns = vec![
        ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::User,
            text: "arroba prompt".to_string(),
            provider_turn_id: Some("user-owned".to_string()),
            observed_at_ms: None,
        },
        ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::Status,
            text: "codex task_complete\n{}".to_string(),
            provider_turn_id: Some("complete-owned".to_string()),
            observed_at_ms: None,
        },
    ];
    let mut arroba_owned = BTreeSet::new();
    arroba_owned.insert("user-owned".to_string());

    let sync = policy.active_prompt_sync(&turns, 0, 0, true, &arroba_owned);

    assert!(sync.should_sync_active_prompt);
    assert!(sync.active_prompt_turn.is_none());
    assert!(sync.latest_observation_settles);
}
