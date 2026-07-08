use super::*;

const ROUTED_COMMAND_CASES: &[&str] = &[
    "prompt agent-2 investigate",
    "agent list",
    "agent ls",
    "agent spawn reviewer",
    "agent focus reviewer",
    "agent alias reviewer code-reviewer",
    "agent name reviewer code-reviewer",
    "agent delete code-reviewer",
    "agent destroy code-reviewer",
    "agent remove code-reviewer",
    "workflow",
    "workflow list",
    "workflow ls",
    "workflow new qa-flow",
    "workflow create qa-flow",
    "workflow resolve qa-flow",
    "workflow show qa-flow",
    "workflow alias wf-1 qa-flow",
    "workflow node add qa-flow reviewer",
    "workflow node remove qa-flow node-1",
    "workflow node instructions qa-flow node-1 Review the implementation",
    "workflow node can-complete qa-flow node-1 true",
    "workflow node intermediate-output qa-flow node-1 false",
    "workflow node wait-for-all-inputs qa-flow node-1 true",
    "workflow node max-turns qa-flow node-1 3",
    "workflow endpoint new qa-flow node-1 default",
    "workflow endpoint create qa-flow node-1 default",
    "workflow endpoint alias qa-flow endpoint-1 default",
    "workflow edge add qa-flow node-1 node-2",
    "workflow edge remove qa-flow edge-1",
    "workflow run qa-flow default Run QA",
    "workflow start qa-flow default Run QA",
    "workflow runs qa-flow",
    "workflow get-run run-1",
    "workflow run get run-1",
    "workflow cancel run-1",
    "workflow resume run-1",
    "mcp list",
    "mcp ls",
    "mcp show playwright",
    "mcp get playwright",
    "mcp install-json {\"name\":\"playwright\"}",
    "mcp update-json {\"name\":\"playwright\"}",
    "mcp uninstall playwright",
    "mcp remove playwright",
    "mcp import codex playwright",
    "mcp grant reviewer playwright",
    "mcp revoke reviewer playwright",
    "skill list",
    "skill ls",
    "skill show browser-qa",
    "skill get browser-qa",
    "skill install ./skills/browser-qa",
    "skill update ./skills/browser-qa",
    "skill uninstall browser-qa",
    "skill remove browser-qa",
    "skill import codex browser-qa",
    "skill grant reviewer browser-qa",
    "skill revoke reviewer browser-qa",
    "skills list",
    "skills grant reviewer browser-qa",
    "credential list",
    "credential ls",
    "credential get credential-1",
    "credential show credential-1",
    "credential upsert-json {\"id\":\"credential-1\"}",
    "credential remove credential-1",
    "credential vault status",
    "credential vault manage",
    "credentials list",
    "credentials get credential-1",
];

#[test]
fn documented_routed_examples_match_execution_policy() {
    for command in meta_commands().filter(|command| command.routed) {
        for example in command
            .examples
            .iter()
            .filter(|example| !example.starts_with("arroba.meta."))
        {
            let tokens = tokenize_command(example).unwrap_or_else(|error| {
                panic!("documented example `{example}` should parse: {error:?}")
            });
            assert_eq!(
                execution_policy(&tokens),
                MetaCommandExecutionPolicy::Routed,
                "documented example `{example}` for `{}` is not routed",
                command.name
            );
        }
    }
}

#[test]
fn routed_command_cases_match_execution_policy() {
    for command in ROUTED_COMMAND_CASES {
        let tokens = tokenize_command(command).unwrap_or_else(|error| {
            panic!("routed command case `{command}` should parse: {error:?}")
        });
        assert_eq!(
            execution_policy(&tokens),
            MetaCommandExecutionPolicy::Routed,
            "`{command}` should be routed by the metaagent command registry",
        );
    }
}

#[test]
fn list_commands_filters_descriptor_table_without_query() {
    let commands = list_commands(MetaCommandListArgs {
        tag: Some("credential".to_string()),
        scope: Some("global".to_string()),
        mutates: Some(true),
        policy: Some("allow".to_string()),
        limit: Some(10),
    });

    assert!(commands.iter().any(|command| {
        matches!(
            command.get("name").and_then(serde_json::Value::as_str),
            Some("credential upsert-json" | "credential remove" | "credential vault")
        )
    }));
    assert!(commands.iter().all(|command| {
        command
            .get("tags")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|tags| tags.iter().any(|tag| tag.as_str() == Some("credential")))
            && command.get("scope").and_then(serde_json::Value::as_str) == Some("global")
            && command.get("mutates").and_then(serde_json::Value::as_bool) == Some(true)
            && command
                .get("metaagent_policy")
                .and_then(serde_json::Value::as_str)
                == Some("allow")
    }));
}

#[test]
fn search_commands_ranks_natural_language_intents() {
    let cases = [
        ("create new agent", "agent spawn"),
        ("make worker", "agent spawn"),
        ("delegate task", "prompt"),
        ("store credential", "credential upsert-json"),
        ("enter new secret", "credential vault"),
        ("stop workflow", "workflow cancel"),
        ("continue workflow", "workflow resume"),
        ("connect workflow nodes", "workflow edge add"),
    ];

    for (query, expected_name) in cases {
        let commands = search_commands(MetaCommandSearchArgs {
            query: Some(query.to_string()),
            tag: None,
            scope: None,
            mutates: None,
            policy: Some("allow".to_string()),
            limit: Some(3),
        });
        assert!(
            commands.iter().any(|command| {
                command.get("name").and_then(serde_json::Value::as_str) == Some(expected_name)
            }),
            "`{query}` should return `{expected_name}` near the top: {commands:?}"
        );
        assert!(
            commands
                .iter()
                .all(|command| command.get("search_score").is_some()),
            "query searches should include match diagnostics"
        );
    }
}

#[test]
fn search_commands_finds_worker_capability_grants() {
    let commands = search_commands(MetaCommandSearchArgs {
        query: Some("give worker tool".to_string()),
        tag: None,
        scope: Some("session".to_string()),
        mutates: Some(true),
        policy: Some("allow".to_string()),
        limit: Some(4),
    });

    assert!(
        commands.iter().any(|command| {
            matches!(
                command.get("name").and_then(serde_json::Value::as_str),
                Some("mcp grant" | "skill grant")
            )
        }),
        "grant command should be discoverable: {commands:?}"
    );
}

#[test]
fn search_commands_finds_workflow_guides() {
    let commands = search_commands(MetaCommandSearchArgs {
        query: Some("build workflow tutorial connect nodes run endpoint".to_string()),
        tag: Some("guide".to_string()),
        scope: Some("session".to_string()),
        mutates: Some(false),
        policy: Some("allow".to_string()),
        limit: Some(5),
    });

    assert!(
        commands.iter().any(|command| {
            command.get("name").and_then(serde_json::Value::as_str)
                == Some("workflow tutorial basic")
        }),
        "workflow tutorial should be discoverable: {commands:?}"
    );
    let docs = command_docs(MetaCommandDocsArgs {
        command: "agent app guide".to_string(),
    })
    .expect("agent app guide docs should exist");
    assert_eq!(
        docs.get("routed").and_then(serde_json::Value::as_bool),
        Some(false)
    );
}

#[test]
fn workflow_command_catalog_includes_unrouted_workflow_capabilities() {
    let required = [
        "workflow node intermediate-output-schema",
        "workflow endpoint bind",
        "workflow endpoint remove",
        "workflow run-output-schema",
        "workflow max-turns",
        "workflow node extensions",
        "workflow pane",
    ];

    for command in required {
        let docs = command_docs(MetaCommandDocsArgs {
            command: command.to_string(),
        })
        .unwrap_or_else(|| panic!("missing docs for `{command}`"));
        assert_eq!(
            docs.get("routed").and_then(serde_json::Value::as_bool),
            Some(false),
            "`{command}` should be documented but not routed"
        );
    }
}

#[test]
fn documented_unrouted_workflow_commands_return_specific_policy_errors() {
    let cases = [
        "workflow endpoint bind qa-flow default queue-1",
        "workflow run-output-schema qa-flow schema-1",
        "workflow edit qa-flow",
    ];

    for command in cases {
        let tokens = tokenize_command(command).expect("command should tokenize");
        let documented = documented_workflow_command(&tokens)
            .unwrap_or_else(|| panic!("`{command}` should resolve to documented workflow docs"));
        assert!(
            !documented.routed,
            "`{command}` should resolve to unrouted docs `{}`",
            documented.name
        );
        let policy = execution_policy(&tokens);
        match policy {
            MetaCommandExecutionPolicy::NotRouted { message } => {
                assert!(
                    message.contains("not routed")
                        || message.contains("not available")
                        || message.contains("not currently exposed")
                        || message.contains("not implemented"),
                    "`{command}` should produce a specific documented error: {message}"
                );
            }
            MetaCommandExecutionPolicy::Denied { message } => {
                assert!(
                    message.contains("not routed")
                        || message.contains("not available")
                        || message.contains("not currently exposed")
                        || message.contains("not implemented"),
                    "`{command}` should produce a specific documented error: {message}"
                );
            }
            MetaCommandExecutionPolicy::Routed => {
                panic!("`{command}` should not be routed");
            }
        }
    }
}

#[test]
fn tokenizer_preserves_quoted_prompt_text() {
    assert_eq!(
        tokenize_command(r#"prompt worker "please inspect the failing test""#)
            .expect("double-quoted prompt should parse"),
        vec!["prompt", "worker", "please inspect the failing test"],
    );
    assert_eq!(
        tokenize_command(r#"prompt worker 'do not expand \slashes'"#)
            .expect("single-quoted prompt should parse"),
        vec!["prompt", "worker", r#"do not expand \slashes"#],
    );
    assert_eq!(
        tokenize_command(r#"prompt worker escaped\ space"#)
            .expect("escaped whitespace should parse"),
        vec!["prompt", "worker", "escaped space"],
    );
}

#[test]
fn tokenizer_rejects_unterminated_quotes() {
    let error = tokenize_command(r#"prompt worker "unterminated"#)
        .expect_err("unterminated quotes should fail");
    assert_eq!(error.message(), "unterminated quote");
}

#[test]
fn command_docs_do_not_advertise_unrouted_subcommands() {
    let forbidden = [
        ("prompt", &["--wait", "--show-reply", "--show-summary"][..]),
        ("workflow", &["edit"][..]),
        ("workflow node add", &["node-name", "node alias"][..]),
        ("mcp", &["test", "adapter", "connector"][..]),
        ("skill", &["grants", "script", "connector"][..]),
        ("slice", &["reset-state"][..]),
        ("credential", &["set-secret", "delete-secret"][..]),
    ];

    for (command, terms) in forbidden {
        let docs = command_docs(MetaCommandDocsArgs {
            command: command.to_string(),
        })
        .unwrap_or_else(|| panic!("missing docs for `{command}`"));
        let rendered = serde_json::json!({
            "usage": docs.get("usage"),
            "aliases": docs.get("aliases"),
            "examples": docs.get("examples"),
        })
        .to_string();
        for term in terms {
            assert!(
                !rendered.contains(term),
                "`{command}` docs should not advertise unrouted `{term}`: {rendered}"
            );
        }
    }
}

#[test]
fn command_docs_prefer_specific_denial_policy_entries() {
    let docs = command_docs(MetaCommandDocsArgs {
        command: "credential set-secret".to_string(),
    })
    .expect("credential secret mutation docs should exist");

    assert_eq!(
        docs.get("name").and_then(serde_json::Value::as_str),
        Some("credential secret mutation")
    );
    assert_eq!(
        docs.get("metaagent_policy")
            .and_then(serde_json::Value::as_str),
        Some("deny")
    );
    assert_eq!(
        docs.get("routed").and_then(serde_json::Value::as_bool),
        Some(false)
    );
}
