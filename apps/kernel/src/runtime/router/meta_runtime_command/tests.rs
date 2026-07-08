use super::request::meta_extension_import_request;
use super::spawn_args::parse_meta_agent_spawn_args;
use super::*;

fn test_session() -> crate::session::RuntimeSession {
    crate::session::RuntimeSession::new(
        "session-1",
        None,
        "workspace",
        "/repo",
        "machine",
        "daemon",
    )
}

#[test]
fn meta_agent_spawn_parser_supports_new_slice_launch_parameters() {
    let args = vec![
        "builder".to_string(),
        "gpt-5.5".to_string(),
        "--slice".to_string(),
        "new:headed".to_string(),
        "--kernel".to_string(),
        "linux-worker".to_string(),
    ];

    let parsed = parse_meta_agent_spawn_args(&args, &test_session())
        .expect("new slice spawn args should parse");

    assert_eq!(parsed.alias.as_deref(), Some("builder"));
    assert_eq!(parsed.model.as_deref(), Some("gpt-5.5"));
    assert_eq!(parsed.kernel_ref.as_deref(), Some("linux-worker"));
    assert!(parsed.slice_ref.is_none());
    assert_eq!(
        parsed.slice_create.map(|create| create.display_mode),
        Some(crate::slice::SliceDisplayMode::Headed)
    );
}

#[test]
fn meta_agent_spawn_parser_supports_existing_slice_placement() {
    let args = vec![
        "checker".to_string(),
        "--slice".to_string(),
        "linux-dev".to_string(),
    ];

    let parsed = parse_meta_agent_spawn_args(&args, &test_session())
        .expect("existing slice spawn args should parse");

    assert_eq!(parsed.alias.as_deref(), Some("checker"));
    assert_eq!(parsed.slice_ref.as_deref(), Some("linux-dev"));
    assert!(parsed.slice_create.is_none());
}

#[test]
fn meta_extension_import_parser_supports_provider_sync() {
    let args = vec![
        "import".to_string(),
        "providers".to_string(),
        "--provider".to_string(),
        "codex".to_string(),
        "--provider".to_string(),
        "claude".to_string(),
        "--kind".to_string(),
        "skill".to_string(),
        "--name".to_string(),
        "docs-helper".to_string(),
        "--dry-run".to_string(),
    ];

    let request = meta_extension_import_request(&test_session(), &args)
        .expect("extension import args should parse");

    let LocalDaemonRequest::ImportProviderCapabilities(request) = request else {
        panic!("unexpected request");
    };
    assert_eq!(request.workspace_id.as_deref(), Some("workspace"));
    assert_eq!(request.providers, vec!["codex", "claude"]);
    assert_eq!(request.kind.as_deref(), Some("skill"));
    assert_eq!(request.name.as_deref(), Some("docs-helper"));
    assert!(request.dry_run);
}

#[test]
fn meta_agent_spawn_parser_supports_provider_model_and_effort() {
    let args = vec![
        "verifier".to_string(),
        "--provider".to_string(),
        "opencode".to_string(),
        "--model".to_string(),
        "opencode/gpt-5.2".to_string(),
        "--variant".to_string(),
        "high".to_string(),
    ];

    let parsed = parse_meta_agent_spawn_args(&args, &test_session())
        .expect("provider launch profile should parse");

    assert_eq!(parsed.alias.as_deref(), Some("verifier"));
    assert_eq!(parsed.provider.as_deref(), Some("opencode"));
    assert_eq!(parsed.model.as_deref(), Some("opencode/gpt-5.2"));
    assert_eq!(parsed.effort.as_deref(), Some("high"));
}

#[test]
fn meta_agent_spawn_parser_rejects_positional_and_flag_model() {
    let args = vec![
        "verifier".to_string(),
        "gpt-5.5".to_string(),
        "--model".to_string(),
        "opencode/gpt-5.2".to_string(),
    ];

    let error = parse_meta_agent_spawn_args(&args, &test_session())
        .expect_err("model should be specified once");

    assert!(format!("{error}").contains("either positional [model] or --model"));
}
