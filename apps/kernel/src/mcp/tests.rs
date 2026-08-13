use super::*;

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "chariox-mcp-registry-{name}-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    let _ = fs::remove_dir_all(&root);
    root
}

#[test]
fn registry_roots_can_be_isolated_for_managed_slice_runtime() {
    let _guard = crate::env_lock::lock();
    let isolation_root = temp_root("managed-slice-isolation");
    std::env::set_var("CHARIOX_CAPABILITY_ISOLATION_ROOT", &isolation_root);

    let project_root = CharioxMcpRegistry::project_root("/workspace");
    let user_root = CharioxMcpRegistry::user_root().expect("user root should resolve");

    std::env::remove_var("CHARIOX_CAPABILITY_ISOLATION_ROOT");
    let _ = fs::remove_dir_all(&isolation_root);

    assert!(project_root.starts_with(isolation_root.join("project")));
    assert!(project_root.ends_with("mcps"));
    assert_eq!(user_root, isolation_root.join("user").join("mcps"));
}

#[test]
fn registry_round_trips_stdio_mcp_config() {
    let root = temp_root("round-trip");
    let registry = CharioxMcpRegistry::new(vec![root.clone()]);
    let mut config =
        CharioxMcpServerConfig::stdio("browser", "npx", vec!["@playwright/mcp@latest".to_string()]);
    if let CharioxMcpTransportConfig::Stdio { env_vars, .. } = &mut config.transport {
        env_vars.push("BROWSER_TOKEN".to_string());
    }

    let path = registry.install(&config).expect("install should succeed");
    assert_eq!(path, root.join("browser.json"));

    let listed = registry.list().expect("list should succeed");
    assert_eq!(listed, vec![config.clone()]);
    assert_eq!(registry.get("browser").unwrap(), Some(config));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn registry_updates_and_uninstalls_existing_mcp_config() {
    let root = temp_root("update-remove");
    let registry = CharioxMcpRegistry::new(vec![root.clone()]);
    let original = CharioxMcpServerConfig::stdio("browser", "npx", vec!["old".to_string()]);
    registry.install(&original).unwrap();

    let updated = CharioxMcpServerConfig::stdio("browser", "node", vec!["new".to_string()]);
    let path = registry.update(&updated).expect("update should succeed");
    assert_eq!(path, root.join("browser.json"));
    assert_eq!(registry.get("browser").unwrap(), Some(updated));

    let removed = registry
        .uninstall("browser")
        .expect("uninstall should succeed");
    assert_eq!(removed, root.join("browser.json"));
    assert_eq!(registry.get("browser").unwrap(), None);
    assert!(!removed.exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn imports_codex_mcp_servers_from_config() {
    let root = temp_root("codex-import-registry");
    let codex_root = temp_root("codex-import-config");
    fs::create_dir_all(&codex_root).unwrap();
    fs::write(
        codex_root.join("config.toml"),
        r#"
[mcp_servers.docs]
command = "docs-server"
args = ["--verbose"]
env_vars = ["DOCS_TOKEN"]
startup_timeout_sec = 2.2

[mcp_servers.docs.env]
ALPHA = "1"

[mcp_servers.web]
url = "https://example.test/mcp"
bearer_token_env_var = "WEB_TOKEN"
enabled_tools = ["search"]

[mcp_servers.oauth]
url = "https://example.test/oauth"
oauth_resource = "unsupported"
"#,
    )
    .unwrap();

    let registry = CharioxMcpRegistry::new(vec![root.clone()]);
    let outcome =
        import_codex_mcp_servers_from_config_path(&registry, &codex_root.join("config.toml"), None)
            .unwrap();

    assert_eq!(
        outcome
            .imported
            .iter()
            .map(|mcp| mcp.name.as_str())
            .collect::<Vec<_>>(),
        vec!["docs", "web"]
    );
    assert_eq!(outcome.skipped.len(), 1);
    assert_eq!(outcome.skipped[0].name, "oauth");
    assert!(outcome.skipped[0].reason.contains("oauth_resource"));
    let docs = registry.get("docs").unwrap().expect("docs import");
    assert_eq!(docs.startup_timeout_sec, Some(3));
    match docs.transport {
        CharioxMcpTransportConfig::Stdio {
            command,
            args,
            env,
            env_vars,
            ..
        } => {
            assert_eq!(command, "docs-server");
            assert_eq!(args, vec!["--verbose"]);
            assert_eq!(env.get("ALPHA"), Some(&"1".to_string()));
            assert_eq!(env_vars, vec!["DOCS_TOKEN"]);
        }
        other => panic!("unexpected transport {other:?}"),
    }
    let web = registry.get("web").unwrap().expect("web import");
    assert_eq!(web.enabled_tools, Some(vec!["search".to_string()]));

    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(codex_root);
}

#[test]
fn imports_opencode_mcp_servers_from_jsonc_config() {
    let root = temp_root("opencode-import-registry");
    let opencode_root = temp_root("opencode-import-config");
    fs::create_dir_all(&opencode_root).unwrap();
    fs::write(
        opencode_root.join("opencode.jsonc"),
        r#"
{
  // OpenCode MCPs
      "mcp": {
        "docs": {
          "type": "local",
          "command": ["docs-server", "--verbose"],
          "environment": {
            "ALPHA": "1",
            "DOCS_TOKEN": "{env:DOCS_TOKEN}",
          },
          "timeout": 2500,
        },
        "web": {
          "type": "remote",
          "url": "https://example.test/mcp",
          "headers": {
            "X-Static": "42",
            "Authorization": "{env:WEB_TOKEN}",
          },
          "oauth": false,
          "enabled": false,
        },
        "oauth": {
          "type": "remote",
          "url": "https://example.test/oauth",
          "oauth": {},
        },
      },
}
"#,
    )
    .unwrap();

    let registry = CharioxMcpRegistry::new(vec![root.clone()]);
    let outcome = import_opencode_mcp_servers_from_config_path(
        &registry,
        &opencode_root.join("opencode.jsonc"),
        None,
    )
    .unwrap();

    assert_eq!(
        outcome
            .imported
            .iter()
            .map(|mcp| mcp.name.as_str())
            .collect::<Vec<_>>(),
        vec!["docs", "web"]
    );
    assert_eq!(outcome.skipped.len(), 1);
    assert_eq!(outcome.skipped[0].name, "oauth");
    assert!(outcome.skipped[0].reason.contains("OAuth"));
    let docs = registry.get("docs").unwrap().expect("docs import");
    assert_eq!(docs.tool_timeout_sec, Some(3));
    match docs.transport {
        CharioxMcpTransportConfig::Stdio {
            command,
            args,
            env,
            env_vars,
            ..
        } => {
            assert_eq!(command, "docs-server");
            assert_eq!(args, vec!["--verbose"]);
            assert_eq!(env.get("ALPHA"), Some(&"1".to_string()));
            assert_eq!(env_vars, vec!["DOCS_TOKEN"]);
        }
        other => panic!("unexpected transport {other:?}"),
    }
    let web = registry.get("web").unwrap().expect("web import");
    assert!(!web.enabled);
    match web.transport {
        CharioxMcpTransportConfig::StreamableHttp {
            http_headers,
            env_http_headers,
            ..
        } => {
            assert_eq!(http_headers.get("X-Static"), Some(&"42".to_string()));
            assert_eq!(
                env_http_headers.get("Authorization"),
                Some(&"WEB_TOKEN".to_string())
            );
        }
        other => panic!("unexpected transport {other:?}"),
    }

    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(opencode_root);
}

#[test]
fn imports_claude_mcp_servers_from_config() {
    let root = temp_root("claude-import-registry");
    let workspace = temp_root("claude-import-workspace");
    fs::create_dir_all(&workspace).unwrap();
    fs::write(
        workspace.join(".mcp.json"),
        r#"
{
  "mcpServers": {
    "docs": {
      "type": "stdio",
      "command": "docs-server",
      "args": ["--verbose"],
      "env": {
        "ALPHA": "1",
        "DOCS_TOKEN": "{env:DOCS_TOKEN}"
      },
      "cwd": "/tmp/docs",
      "startup_timeout_sec": 2.2,
      "enabledTools": ["search"]
    },
    "web": {
      "type": "http",
      "url": "https://example.test/mcp",
      "headers": {
        "X-Static": "42",
        "Authorization": "{env:WEB_TOKEN}"
      },
      "disabledTools": ["write"]
    },
    "oauth": {
      "type": "sse",
      "url": "https://example.test/sse"
    },
    "inline_auth": {
      "type": "http",
      "url": "https://example.test/mcp",
      "headers": {
        "Authorization": "Bearer secret"
      }
    }
  }
}
"#,
    )
    .unwrap();

    let registry = CharioxMcpRegistry::new(vec![root.clone()]);
    let outcome = import_claude_mcp_servers_from_config_path(
        &registry,
        &workspace.join(".mcp.json"),
        &workspace,
        None,
    )
    .unwrap();

    assert_eq!(
        outcome
            .imported
            .iter()
            .map(|mcp| mcp.name.as_str())
            .collect::<Vec<_>>(),
        vec!["docs", "web"]
    );
    let mut skipped_names = outcome
        .skipped
        .iter()
        .map(|skip| skip.name.as_str())
        .collect::<Vec<_>>();
    skipped_names.sort_unstable();
    assert_eq!(skipped_names, vec!["inline_auth", "oauth"]);
    assert!(outcome
        .skipped
        .iter()
        .any(|skip| skip.reason.contains("Authorization")));

    let docs = registry.get("docs").unwrap().expect("docs import");
    assert_eq!(docs.startup_timeout_sec, Some(3));
    assert_eq!(docs.enabled_tools, Some(vec!["search".to_string()]));
    match docs.transport {
        CharioxMcpTransportConfig::Stdio {
            command,
            args,
            env,
            env_vars,
            cwd,
            ..
        } => {
            assert_eq!(command, "docs-server");
            assert_eq!(args, vec!["--verbose"]);
            assert_eq!(env.get("ALPHA"), Some(&"1".to_string()));
            assert_eq!(env_vars, vec!["DOCS_TOKEN"]);
            assert_eq!(cwd, Some(PathBuf::from("/tmp/docs")));
        }
        other => panic!("unexpected transport {other:?}"),
    }
    let web = registry.get("web").unwrap().expect("web import");
    assert_eq!(web.disabled_tools, Some(vec!["write".to_string()]));
    match web.transport {
        CharioxMcpTransportConfig::StreamableHttp {
            http_headers,
            env_http_headers,
            ..
        } => {
            assert_eq!(http_headers.get("X-Static"), Some(&"42".to_string()));
            assert_eq!(
                env_http_headers.get("Authorization"),
                Some(&"WEB_TOKEN".to_string())
            );
        }
        other => panic!("unexpected transport {other:?}"),
    }

    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn imports_matching_project_mcp_servers_from_claude_user_config() {
    let root = temp_root("claude-project-import-registry");
    let workspace = temp_root("claude-project-import-workspace");
    let config_root = temp_root("claude-project-import-config");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&config_root).unwrap();
    let config_path = config_root.join(".claude.json");
    fs::write(
        &config_path,
        format!(
            r#"{{
  "mcpServers": {{
    "global_docs": {{
      "command": "global-docs"
    }}
  }},
  "projects": {{
    "{}": {{
      "mcpServers": {{
        "project_docs": {{
          "command": "project-docs"
        }}
      }}
    }},
    "/elsewhere": {{
      "mcpServers": {{
        "other_docs": {{
          "command": "other-docs"
        }}
      }}
    }}
  }}
}}"#,
            workspace.display()
        ),
    )
    .unwrap();

    let registry = CharioxMcpRegistry::new(vec![root.clone()]);
    let outcome =
        import_claude_mcp_servers_from_config_path(&registry, &config_path, &workspace, None)
            .unwrap();

    assert_eq!(
        outcome
            .imported
            .iter()
            .map(|mcp| mcp.name.as_str())
            .collect::<Vec<_>>(),
        vec!["global_docs", "project_docs"]
    );
    assert!(registry.get("other_docs").unwrap().is_none());

    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(workspace);
    let _ = fs::remove_dir_all(config_root);
}

#[test]
fn rejects_invalid_mcp_names() {
    let config = CharioxMcpServerConfig::stdio("../bad", "npx", Vec::new());
    assert!(config.validate().is_err());
}
