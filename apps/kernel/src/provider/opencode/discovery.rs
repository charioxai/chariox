//! Ephemeral launch policy for project-environment discovery.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::error::DaemonError;
use crate::provider::RuntimeProviderRun;

/// Lives with the child process, not the durable provider/account configuration.
pub(crate) struct OpenCodeDiscoveryConfigDirectory(PathBuf);

impl Drop for OpenCodeDiscoveryConfigDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl OpenCodeDiscoveryConfigDirectory {
    pub(crate) fn apply_to_launch_args(
        &self,
        run: &RuntimeProviderRun,
        args: &mut Vec<String>,
    ) -> Result<(), DaemonError> {
        if run.adapter_key() != "opencode" || !run.read_only_discovery() {
            return Ok(());
        }

        let separator = args.iter().position(|argument| argument == "--");
        let managed_end = separator.unwrap_or(args.len());
        let managed = args[..managed_end].windows(3).any(|window| {
            window[0] == "--setenv"
                && window[1] == "CHARIOX_MANAGED_PROVIDER_ISOLATION_ACTIVE"
                && window[2] == "1"
        });
        if !managed {
            return Ok(());
        }
        let separator = separator.ok_or_else(|| DaemonError::ProviderProtocol {
            provider_run_id: run.id().to_string(),
            operation: "opencode_discovery_config",
            message: "managed OpenCode discovery launch is missing its command separator"
                .to_string(),
        })?;
        let failure = |message: &str| DaemonError::ProviderProtocol {
            provider_run_id: run.id().to_string(),
            operation: "opencode_discovery_config",
            message: message.to_string(),
        };
        let isolated_config_home = self.0.display().to_string();
        let mut separator = separator;
        let mut index = 0;
        let mut found_xdg_config_home = false;
        while index < separator {
            if args[index] != "--setenv" {
                index += 1;
                continue;
            }
            if index.saturating_add(2) >= separator {
                return Err(failure(
                    "managed OpenCode discovery launch contains an incomplete --setenv entry",
                ));
            }
            match args[index + 1].as_str() {
                "XDG_CONFIG_HOME" => {
                    args[index + 2] = isolated_config_home.clone();
                    found_xdg_config_home = true;
                    index += 3;
                }
                "OPENCODE_CONFIG" | "OPENCODE_CONFIG_DIR" => {
                    args.drain(index..index + 3);
                    separator -= 3;
                }
                _ => index += 1,
            }
        }
        if !found_xdg_config_home {
            args.splice(
                separator..separator,
                [
                    "--setenv".to_string(),
                    "XDG_CONFIG_HOME".to_string(),
                    isolated_config_home,
                ],
            );
        }
        Ok(())
    }
}

pub(crate) fn isolate_opencode_discovery_configuration(
    run: &RuntimeProviderRun,
    environment: &mut BTreeMap<String, String>,
    environment_remove: &mut Vec<String>,
) -> Result<Option<OpenCodeDiscoveryConfigDirectory>, DaemonError> {
    if run.adapter_key() != "opencode" || !run.read_only_discovery() {
        return Ok(None);
    }
    let failure = |message: &str| DaemonError::ProviderProtocol {
        provider_run_id: run.id().to_string(),
        operation: "opencode_discovery_config",
        message: message.to_string(),
    };
    let (home, _) = run
        .preparation_environment()
        .ok_or_else(|| failure("OpenCode discovery requires a kernel-owned preparation home"))?;
    // The preparation home is mounted at the same path inside managed workers.
    // A host temp directory would disappear behind their private /tmp mount.
    let directory = PathBuf::from(home).join(format!(
        "opencode-discovery-{:032x}",
        rand::random::<u128>()
    ));
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&directory).map_err(|_| {
        failure("could not create an isolated OpenCode discovery configuration directory")
    })?;
    let directory = OpenCodeDiscoveryConfigDirectory(directory);
    // Empty inline maps merge with global config. Isolate the source itself so
    // global MCP commands and plugins cannot execute before session permissions.
    environment.insert("XDG_CONFIG_HOME".into(), directory.0.display().to_string());
    for name in ["OPENCODE_CONFIG", "OPENCODE_CONFIG_DIR"] {
        environment.remove(name);
        if !environment_remove.iter().any(|value| value == name) {
            environment_remove.push(name.into());
        }
    }
    // Keep XDG_DATA_HOME untouched: provider-native authentication remains owned
    // by OpenCode, and ordinary restoration uses the original run environment.
    Ok(Some(directory))
}

pub(crate) fn apply_opencode_discovery_environment(
    run: &RuntimeProviderRun,
    environment: &mut BTreeMap<String, String>,
) -> Result<(), DaemonError> {
    if run.adapter_key() != "opencode" || !run.read_only_discovery() {
        return Ok(());
    }
    let invalid_config = || DaemonError::ProviderProtocol {
        provider_run_id: run.id().to_string(),
        operation: "opencode_discovery_config",
        message: "OpenCode discovery requires an object-valued inline configuration".to_string(),
    };
    let mut config = match environment.get("OPENCODE_CONFIG_CONTENT") {
        Some(value) => {
            serde_json::from_str::<serde_json::Value>(value).map_err(|_| invalid_config())?
        }
        None => serde_json::json!({}),
    };
    let object = config.as_object_mut().ok_or_else(invalid_config)?;
    // Do not give the discovery child ordinary runtime/granted MCP credentials.
    // This changes only the spawn copy, so ordinary restoration retains them.
    object.insert("mcp".into(), serde_json::json!({}));
    // Plugins execute during native initialization, before session permissions
    // can constrain tools. The isolated global directory does not remove inline
    // plugins, so clear those in the discovery-only spawn copy as well.
    object.insert("plugin".into(), serde_json::json!([]));
    let permissions = serde_json::json!({
        "*": "deny", "read": "allow", "glob": "allow", "grep": "allow", "list": "allow"
    });
    object.insert("permission".into(), permissions.clone());
    // Agent policy overrides global policy in OpenCode. The utility uses plan.
    let agents = object
        .entry("agent")
        .or_insert_with(|| serde_json::json!({}));
    let agents = agents.as_object_mut().ok_or_else(invalid_config)?;
    let plan = agents
        .entry("plan")
        .or_insert_with(|| serde_json::json!({}));
    plan.as_object_mut()
        .ok_or_else(invalid_config)?
        .insert("permission".into(), permissions);
    environment.insert("OPENCODE_CONFIG_CONTENT".into(), config.to_string());
    environment.insert("OPENCODE_DISABLE_PROJECT_CONFIG".into(), "true".into());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult};

    fn run(discovery: bool) -> RuntimeProviderRun {
        run_with_program(discovery, None)
    }

    fn run_with_program(discovery: bool, pty_program: Option<&str>) -> RuntimeProviderRun {
        let request =
            LaunchProviderRequest::new("session", "opencode", "opencode", "default", "model");
        let mut run = RuntimeProviderRun::new(
            "run",
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: "discovery-policy-test".into(),
                pty_target: None,
                pty_program: pty_program.map(str::to_owned),
                pty_args: Vec::new(),
                pty_env: BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        );
        run.set_read_only_discovery(discovery);
        run
    }

    #[cfg(unix)]
    #[test]
    fn failed_discovery_spawn_cleans_isolated_configuration_directory() {
        let root = std::env::temp_dir().join(format!(
            "chariox-discovery-failed-spawn-{:032x}",
            rand::random::<u128>()
        ));
        let home = root
            .join(".chariox-project-environment")
            .join("a".repeat(64));
        std::fs::create_dir_all(&home).unwrap();

        let mut provider = run_with_program(true, Some("/definitely/not/a/provider"));
        provider
            .set_preparation_environment(home.display().to_string(), "/usr/bin")
            .unwrap();

        let mut manager = crate::pty::PtyManager::new();
        let error = manager
            .spawn_for_run(&provider)
            .expect_err("an invalid provider executable must fail to spawn");
        assert!(
            error.to_string().contains("failed to spawn PTY"),
            "unexpected failed-spawn error: {error}"
        );

        let leftovers = std::fs::read_dir(&home)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("opencode-discovery-"))
            })
            .collect::<Vec<_>>();
        assert!(
            leftovers.is_empty(),
            "failed discovery spawn left isolated config directories: {leftovers:?}"
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discovery_removes_inline_mcp_and_overrides_plan_permissions() {
        let original = BTreeMap::from([(
            "OPENCODE_CONFIG_CONTENT".into(),
            serde_json::json!({
                "mcp": {"mutating": {"enabled": true}},
                "plugin": ["file:///ordinary/startup-plugin.mjs"],
                "permission": "allow",
                "agent": {"plan": {"permission": "allow", "temperature": 0.2}},
                "model": "fixture/model"
            })
            .to_string(),
        )]);
        let mut child = original.clone();
        apply_opencode_discovery_environment(&run(true), &mut child).unwrap();
        let config: serde_json::Value =
            serde_json::from_str(&child["OPENCODE_CONFIG_CONTENT"]).unwrap();
        assert_eq!(config["mcp"], serde_json::json!({}));
        assert_eq!(config["plugin"], serde_json::json!([]));
        let preserved: serde_json::Value =
            serde_json::from_str(&original["OPENCODE_CONFIG_CONTENT"]).unwrap();
        assert_eq!(
            preserved["plugin"][0],
            "file:///ordinary/startup-plugin.mjs"
        );
        assert_eq!(config["permission"]["*"], "deny");
        assert_eq!(config["agent"]["plan"]["permission"], config["permission"]);
        assert_eq!(config["agent"]["plan"]["temperature"], 0.2);
        assert_eq!(config["model"], "fixture/model");
        assert_eq!(child["OPENCODE_DISABLE_PROJECT_CONFIG"], "true");
        assert_ne!(child, original);
    }

    #[test]
    fn ordinary_spawn_keeps_configuration_unchanged() {
        let mut environment =
            BTreeMap::from([("OPENCODE_CONFIG_CONTENT".into(), "original".into())]);
        let original = environment.clone();
        apply_opencode_discovery_environment(&run(false), &mut environment).unwrap();
        assert_eq!(environment, original);
        let mut removed = Vec::new();
        assert!(isolate_opencode_discovery_configuration(
            &run(false),
            &mut environment,
            &mut removed,
        )
        .unwrap()
        .is_none());
        assert_eq!(environment, original);
        assert!(removed.is_empty());
    }

    #[test]
    fn discovery_isolates_global_configuration_and_cleans_it_after_child_exit() {
        let root = std::env::temp_dir().join(format!(
            "chariox-discovery-config-test-{:032x}",
            rand::random::<u128>()
        ));
        let home = root
            .join(".chariox-project-environment")
            .join("a".repeat(64));
        std::fs::create_dir_all(&home).unwrap();
        let mut provider = run(true);
        provider
            .set_preparation_environment(home.display().to_string(), "/usr/bin")
            .unwrap();
        let original = BTreeMap::from([
            ("OPENCODE_CONFIG".into(), "/ordinary/config.json".into()),
            ("OPENCODE_CONFIG_DIR".into(), "/ordinary/config".into()),
            ("XDG_CONFIG_HOME".into(), "/ordinary/xdg-config".into()),
            ("XDG_DATA_HOME".into(), "/ordinary/provider-auth".into()),
        ]);
        let mut child = original.clone();
        let mut removed = Vec::new();
        let guard = isolate_opencode_discovery_configuration(&provider, &mut child, &mut removed)
            .unwrap()
            .unwrap();
        let isolated = PathBuf::from(&child["XDG_CONFIG_HOME"]);
        assert!(isolated.starts_with(&home));
        assert!(isolated.is_dir());
        assert_eq!(std::fs::read_dir(&isolated).unwrap().count(), 0);
        assert_eq!(child["XDG_DATA_HOME"], original["XDG_DATA_HOME"]);
        for name in ["OPENCODE_CONFIG", "OPENCODE_CONFIG_DIR"] {
            assert!(!child.contains_key(name));
            assert!(removed.iter().any(|value| value == name));
        }
        drop(guard);
        assert!(!isolated.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn managed_discovery_rewrites_final_child_config_sources() {
        let root = std::env::temp_dir().join(format!(
            "chariox-discovery-final-args-test-{:032x}",
            rand::random::<u128>()
        ));
        let isolated = root.join(".chariox-project-environment").join("isolated");
        std::fs::create_dir_all(&isolated).unwrap();
        let guard = OpenCodeDiscoveryConfigDirectory(isolated.clone());
        let mut args = vec![
            "--setenv".into(),
            "CHARIOX_MANAGED_PROVIDER_ISOLATION_ACTIVE".into(),
            "1".into(),
            "--unsetenv".into(),
            "XDG_CONFIG_HOME".into(),
            "--setenv".into(),
            "XDG_CONFIG_HOME".into(),
            "/home/chariox/.provider-account/root-3".into(),
            "--unsetenv".into(),
            "OPENCODE_CONFIG_DIR".into(),
            "--setenv".into(),
            "OPENCODE_CONFIG_DIR".into(),
            "/home/chariox/.provider-account/root-7".into(),
            "--setenv".into(),
            "OPENCODE_CONFIG".into(),
            "/home/chariox/.provider-account/global.json".into(),
            "--".into(),
            "/usr/local/bin/opencode".into(),
            "debug".into(),
            "config".into(),
        ];
        let command_tail = args[args.iter().position(|arg| arg == "--").unwrap() + 1..].to_vec();

        guard.apply_to_launch_args(&run(true), &mut args).unwrap();

        let separator = args.iter().position(|arg| arg == "--").unwrap();
        let namespace_args = &args[..separator];
        let xdg_bindings = namespace_args
            .windows(3)
            .filter(|window| window[0] == "--setenv" && window[1] == "XDG_CONFIG_HOME")
            .collect::<Vec<_>>();
        assert_eq!(xdg_bindings.len(), 1);
        assert_eq!(xdg_bindings[0][2], isolated.display().to_string());
        for name in ["OPENCODE_CONFIG", "OPENCODE_CONFIG_DIR"] {
            assert!(!namespace_args
                .windows(3)
                .any(|window| { window[0] == "--setenv" && window[1] == name }));
        }
        // These are the finalized child sources: the former global XDG path
        // and account-local OPENCODE_CONFIG_DIR must not survive the launch
        // boundary, while the official OpenCode command remains unchanged.
        assert!(!namespace_args
            .iter()
            .any(|argument| argument.contains(".provider-account/root-3")));
        assert!(!namespace_args
            .iter()
            .any(|argument| argument.contains(".provider-account/root-7")));
        assert_eq!(&args[separator + 1..], command_tail);
        drop(guard);
        assert!(!isolated.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discovery_without_preparation_home_fails_before_environment_mutation() {
        let mut environment = BTreeMap::new();
        let mut removed = Vec::new();
        assert!(isolate_opencode_discovery_configuration(
            &run(true),
            &mut environment,
            &mut removed,
        )
        .is_err());
        assert!(environment.is_empty());
        assert!(removed.is_empty());
    }

    #[test]
    fn discovery_rejects_invalid_config_without_mutating_environment() {
        for content in [
            "invalid",
            "[]",
            "null",
            r#"{"agent":false}"#,
            r#"{"agent":{"plan":false}}"#,
        ] {
            let mut environment =
                BTreeMap::from([("OPENCODE_CONFIG_CONTENT".into(), content.into())]);
            let original = environment.clone();
            assert!(apply_opencode_discovery_environment(&run(true), &mut environment).is_err());
            assert_eq!(environment, original);
        }
    }
}
