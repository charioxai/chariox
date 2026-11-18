//! A hot reload is not the end of the kernel: a run whose process it kept authenticates its
//! tool calls (runtime tools, Vault) from the first moment, before any recovered prompt
//! claims the run. The new kernel has no run under the token yet; only the handover knows it.
use super::*;

/// The run as a hot reload hands it over: the new kernel's provider store has no live run for the
/// agent (the fixture's run is ended) and does not know the token the process calls with.
struct Surviving {
    token: String,
    run_id: String,
}

impl Drop for Surviving {
    fn drop(&mut self) {
        crate::hot_reload::forget_unclaimed_runs_for_tests();
    }
}

async fn surviving_run(f: &Fixture) -> Surviving {
    let mut run = f.app.lock().await.providers.get_run(&f.run).unwrap();
    f.app
        .lock()
        .await
        .providers
        .mark_run_ended_provider_only(&f.room, &f.run)
        .unwrap();
    run.adopt_handed_over_identity(f.run.clone(), Some("handed-over-token".into()), None);
    let surviving = Surviving {
        token: "handed-over-token".into(),
        run_id: run.id().into(),
    };
    assert!(f
        .runtime
        .owned
        .provider_store
        .get_runs_by_runtime_mcp_auth_token(&surviving.token)
        .is_empty());
    crate::hot_reload::hold_unclaimed_run_for_tests(run.handover_identity());
    surviving
}

#[tokio::test]
async fn a_surviving_run_calls_a_runtime_tool_before_a_prompt_claims_it() {
    let f = fixture("handed-over-runtime-tool", true).await;
    let surviving = surviving_run(&f).await;
    let task = f.begin(&f.prompt_id);
    let timer = call(
        &f,
        &surviving.token,
        "chariox.events.timer",
        &task,
        serde_json::json!({"label":"armed after the reload","delay_ms":3_600_000}),
    )
    .await
    .expect("the handed-over run's token still authenticates its runtime tools");
    assert!(timer.ok, "{:?}", timer.payload);
    assert!(
        f.store()
            .agent_wake(timer.payload["wake_id"].as_str().unwrap())
            .unwrap()
            .is_some(),
        "the timer is the kernel's, as one armed before the reload"
    );
}

#[tokio::test]
async fn a_surviving_run_calls_a_vault_tool_before_a_prompt_claims_it() {
    let f = fixture("handed-over-vault-tool", true).await;
    let surviving = surviving_run(&f).await;
    let context = f
        .runtime
        .vault_agent_context(&surviving.token)
        .expect("the Vault knows the handed-over run's agent");
    assert_eq!(context.run.id(), surviving.run_id);
    assert_eq!(context.agent_id, f.agent);
    assert_eq!(context.session_id, f.room);
    let task = f.begin(&f.prompt_id);
    let refused = call(
        &f,
        &surviving.token,
        "chariox.vault.use_env",
        &task,
        serde_json::json!({"origin":"https://example.test","argv":["true"],"env":{}}),
    )
    .await
    .err()
    .map(|error| error.to_string())
    .unwrap_or_default();
    for authentication in [
        "Vault operations require one authenticated provider run",
        "invalid runtime MCP auth token",
    ] {
        assert!(!refused.contains(authentication), "{refused}");
    }
}

#[tokio::test]
async fn a_token_nobody_holds_is_still_refused() {
    let f = fixture("handed-over-unknown-token", true).await;
    let _surviving = surviving_run(&f).await;
    let task = f.begin(&f.prompt_id);
    let refused = call(
        &f,
        "a-token-no-run-holds",
        "chariox.events.timer",
        &task,
        serde_json::json!({"label":"no","delay_ms":3_600_000}),
    )
    .await
    .expect_err("a token of no run, handed over or not, authenticates nothing");
    assert!(refused
        .to_string()
        .contains("invalid runtime MCP auth token"));
}
