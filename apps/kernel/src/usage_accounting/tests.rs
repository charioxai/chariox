use super::*;
use serde_json::json;

#[test]
fn codex_uses_cumulative_components_and_does_not_bill_reasoning_twice() {
    let usage = codex_usage(&json!({"inputTokens":1000,"cachedInputTokens":800,
        "outputTokens":100,"reasoningOutputTokens":60}))
    .unwrap();
    assert_eq!(usage.input, Some(1000));
    assert_eq!(usage.reasoning, Some(60));
    assert_eq!(quote("gpt-5.3-codex", None, &usage), Some(1_890_000));
}

#[test]
fn missing_and_invalid_components_are_not_free_usage() {
    assert!(codex_usage(&json!({"totalTokens":10})).is_none());
    assert!(codex_usage(&json!({"inputTokens":-1,"outputTokens":2})).is_none());
    assert!(
        codex_usage(&json!({"inputTokens":1,"cachedInputTokens":2,"outputTokens":0})).is_none()
    );
    let unknown_cache = codex_usage(&json!({"inputTokens":10,"outputTokens":2})).unwrap();
    assert_eq!(quote("gpt-5.3-codex", None, &unknown_cache), None);
    let zero =
        codex_usage(&json!({"inputTokens":0,"cachedInputTokens":0,"outputTokens":0})).unwrap();
    assert_eq!(quote("gpt-5.3-codex", None, &zero), Some(0));
    assert_eq!(quote("unknown", None, &zero), None);
}

#[test]
fn claude_cache_writes_require_reported_ttl_and_reads_are_separate() {
    let value = json!({"input_tokens":10,"output_tokens":2,
        "cache_read_input_tokens":100,"cache_creation_input_tokens":20,
        "cache_creation":{"ephemeral_5m_input_tokens":15,"ephemeral_1h_input_tokens":5}});
    let usage = claude_usage(&value).unwrap();
    assert_eq!(usage.input, Some(130));
    assert_eq!(quote("claude-sonnet-4-6", None, &usage), Some(176_250));
    let mut no_ttl = value.clone();
    no_ttl.as_object_mut().unwrap().remove("cache_creation");
    assert_eq!(
        quote("claude-sonnet-4-6", None, &claude_usage(&no_ttl).unwrap()),
        None
    );
}

#[test]
fn opencode_output_excludes_reasoning_so_normalization_includes_it_once() {
    let usage = opencode_usage(&json!({"input":10,"output":2,"reasoning":3,
        "cache":{"read":100,"write":0}}))
    .unwrap();
    assert_eq!(usage.input, Some(110));
    assert_eq!(usage.output, Some(5));
    assert_eq!(quote("claude-sonnet-4-6", None, &usage), Some(135_000));
}

#[test]
fn price_bands_and_models_are_exact_not_guessed_aliases() {
    let mut usage =
        codex_usage(&json!({"inputTokens":1000,"cachedInputTokens":800,"outputTokens":100}))
            .unwrap();
    usage.cache_write = Some(0); // Explicitly reported API cache-write count for band pricing.
    assert_eq!(quote("gpt-6.1-sol", None, &usage), None);
    assert_eq!(quote("gpt-6.1-sol", Some("short"), &usage), Some(1_480_000));
    assert_eq!(quote("gpt-6.1-sol", Some("long"), &usage), Some(2_460_000));
    assert_eq!(quote("openai/gpt-6.1-sol", Some("short"), &usage), None);
}

#[test]
fn openai_cache_writes_do_not_require_an_anthropic_ttl() {
    let usage = Usage {
        input: Some(1000),
        cached_input: Some(800),
        cache_write: Some(50),
        output: Some(100),
        ..Usage::default()
    };
    assert_eq!(quote("gpt-6.1-sol", Some("short"), &usage), Some(1_505_000));
}

#[test]
fn overflow_is_rejected_and_large_costs_do_not_wrap() {
    assert!(opencode_usage(
        &json!({"input":u64::MAX,"output":1,"reasoning":1,"cache":{"read":1,"write":0}})
    )
    .is_none());
    let usage =
        codex_usage(&json!({"inputTokens":u64::MAX,"cachedInputTokens":0,"outputTokens":0}))
            .unwrap();
    assert_eq!(
        quote("gpt-5.3-codex", None, &usage),
        Some(u128::from(u64::MAX) * 1750)
    );
}
