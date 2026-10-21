//! MP-08 / MP-10 / MP-11: populated handoff settings are part of the client wire.
use super::*;

#[test]
fn populated_handoff_config_responses_have_versioned_shapes_and_hashes() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let mut config = crate::config::CharioxUserConfig::default();
    config.history.handoff.codex_brief_model = Some("gpt-6-luna".into());
    config.history.handoff.claude_brief_model = Some("claude-haiku-5-5".into());
    let responses = [
        LocalDaemonResponse::UserConfig {
            path: "/state/config.toml".into(),
            config: config.clone(),
        },
        LocalDaemonResponse::UserConfigUpdated {
            path: "/state/config.toml".into(),
            config,
            effects: Vec::new(),
        },
    ];
    for (response, kind) in responses
        .into_iter()
        .zip(["UserConfig", "UserConfigUpdated"])
    {
        let wire = serde_json::to_value(response).unwrap();
        let handoff = &wire[kind]["config"]["history"]["handoff"];
        assert_eq!(
            handoff,
            &serde_json::json!({
                "codex_brief_model": "gpt-6-luna",
                "claude_brief_model": "claude-haiku-5-5",
            })
        );
        let digest = Sha256::digest(serde_json::to_vec(handoff).unwrap());
        assert_eq!(
            format!("{digest:x}"),
            "40158e7f834b1d99b63ba7853672e2e5d4c671729965b78bdde2d9529919d1c3"
        );
        let decoded: LocalDaemonResponse = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), wire);
    }
}
