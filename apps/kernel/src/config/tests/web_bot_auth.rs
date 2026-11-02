use super::*;

#[test]
fn web_bot_auth_is_opt_in_and_invalid_mutation_keeps_previous_policy() {
    let mut config = CharioxUserConfig::default();
    assert!(config.web_bot_auth.projects.is_empty());
    config
        .set_value("web_bot_auth.projects", r#"["project-a"]"#.into())
        .unwrap();
    config
        .set_value(
            "web_bot_auth.signature_agent",
            "https://operator.example/".into(),
        )
        .unwrap();
    assert_eq!(
        config.web_bot_auth.signature_agent.as_deref(),
        Some("https://operator.example")
    );
    let previous = config.clone();
    for (field, value) in [
        ("web_bot_auth.projects", r#"[""]"#),
        ("web_bot_auth.projects", "all"),
        ("web_bot_auth.signature_agent", "http://operator.example"),
        (
            "web_bot_auth.signature_agent",
            "https://operator.example/not-an-origin",
        ),
    ] {
        assert!(config.set_value(field, value.into()).is_err());
        assert_eq!(config, previous);
    }
    let wire = serde_json::to_value(&config).unwrap();
    assert_eq!(
        wire["web_bot_auth"]["projects"],
        serde_json::json!(["project-a"])
    );
    assert_eq!(
        toml::from_str::<CharioxUserConfig>(&toml::to_string(&config).unwrap()).unwrap(),
        config
    );
    config.unset_value("web_bot_auth.projects").unwrap();
    config.unset_value("web_bot_auth.signature_agent").unwrap();
    assert_eq!(config.web_bot_auth, UserWebBotAuthConfig::default());
}
