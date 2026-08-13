use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use rand::RngCore;

use crate::error::DaemonError;
use crate::provider::{AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult};

use super::executable_resolution::ExecutableResolutionState;

mod catalog;
mod launch_args;
mod mcp_config;
mod native_tui;

pub use catalog::claude_provider_catalog;
use catalog::CLAUDE_HEADLESS_PROVIDER_ID;
use launch_args::claude_launch_args;
#[cfg(test)]
pub(crate) use mcp_config::CLAUDE_MCP_CONFIG_PLACEHOLDER;
pub(crate) use mcp_config::{materialize_runtime_claude_mcp_config, ClaudeMcpConfigFile};
use native_tui::{claude_native_tui_args, prepare_claude_native_tui_files};

pub(crate) const CLAUDE_STRUCTURED_ENDPOINT: &str = "stdio://claude";

const CLAUDE_ENV_OVERRIDE: &str = "CHARIOX_CLAUDE_BIN";
const CLAUDE_CONFIG_DIR_ENV: &str = "CLAUDE_CONFIG_DIR";
const CLAUDE_HEADLESS_STATE_FILE: &str = ".claude.json";
const CLAUDE_DISABLE_AUTOUPDATER_ENV: &str = "DISABLE_AUTOUPDATER";
static CLAUDE_EXECUTABLE_RESOLUTION: ExecutableResolutionState =
    ExecutableResolutionState::new("claude");
const CLAUDE_AUTH_ENV_VARS: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_CUSTOM_HEADERS",
];
pub fn resolve_claude_executable() -> Result<PathBuf, DaemonError> {
    let _guard = crate::env_lock::lock();
    resolve_claude_executable_unlocked()
}

fn resolve_claude_executable_unlocked() -> Result<PathBuf, DaemonError> {
    if let Some(path) = env::var_os(CLAUDE_ENV_OVERRIDE).map(PathBuf::from) {
        return CLAUDE_EXECUTABLE_RESOLUTION
            .resolve(|| resolve_candidate(path.clone(), true))
            .ok_or_else(|| DaemonError::ProviderExecutableNotFound {
                adapter_key: "claude".to_string(),
                executable: env::var(CLAUDE_ENV_OVERRIDE).unwrap_or_else(|_| "claude".to_string()),
            });
    }

    CLAUDE_EXECUTABLE_RESOLUTION
        .resolve(|| resolve_candidate(PathBuf::from("claude"), false))
        .ok_or_else(|| DaemonError::ProviderExecutableNotFound {
            adapter_key: "claude".to_string(),
            executable: "claude".to_string(),
        })
}

pub fn plan_claude_launch(
    request: Option<&LaunchProviderRequest>,
) -> Result<ProviderLaunchResult, DaemonError> {
    let _guard = crate::env_lock::lock();
    plan_claude_launch_unlocked(request)
}

fn plan_claude_launch_unlocked(
    request: Option<&LaunchProviderRequest>,
) -> Result<ProviderLaunchResult, DaemonError> {
    let provider = request
        .map(|request| request.provider.as_str())
        .unwrap_or("<none>");
    crate::logging::info_with_fields(
        "daemon.provider.claude",
        "planning Claude launch",
        serde_json::json!({
            "provider": provider,
            "has_structured_endpoint": request.and_then(|request| request.structured_endpoint.as_ref()).is_some(),
            "client_interface": request.map(|request| format!("{:?}", request.client_interface)),
        }),
    );
    if let Some(endpoint) = request.and_then(|request| request.structured_endpoint.clone()) {
        let working_directory = request.and_then(|request| request.working_directory.clone());
        crate::logging::info_with_fields(
            "daemon.provider.claude",
            "planned Claude structured proxy launch",
            serde_json::json!({ "provider": provider }),
        );
        return Ok(ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "claude:structured-stdio-proxy".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: BTreeMap::new(),
            pty_env_remove: claude_provider_env_remove(request),
            working_directory,
            structured_endpoint: Some(endpoint),
        });
    }

    crate::logging::info_with_fields(
        "daemon.provider.claude",
        "resolving Claude executable",
        serde_json::json!({ "provider": provider }),
    );
    let executable = resolve_claude_executable_unlocked()?;
    let request = request.ok_or_else(|| DaemonError::LocalTransport {
        operation: "plan_claude_launch",
        message: "Claude provider launch requires a provider run request".to_string(),
    })?;
    crate::logging::info_with_fields(
        "daemon.provider.claude",
        "resolved Claude executable",
        serde_json::json!({
            "provider": request.provider.as_str(),
            "executable": executable.display().to_string(),
        }),
    );
    if !request.client_interface.is_chariox() && request.provider != CLAUDE_HEADLESS_PROVIDER_ID {
        crate::logging::info_with_fields(
            "daemon.provider.claude",
            "preparing Claude native TUI files",
            serde_json::json!({ "provider": request.provider.as_str() }),
        );
        let mut native = prepare_claude_native_tui_files(request)?;
        native.materialize_mcp_config(request)?;
        let mut pty_env = claude_process_env();
        pty_env.insert(
            "CHARIOX_CLAUDE_NATIVE_EVENTS".to_string(),
            native.events_file.display().to_string(),
        );
        pty_env.insert(
            "CHARIOX_CLAUDE_NATIVE_CONTEXT".to_string(),
            native.context_file.display().to_string(),
        );
        pty_env.insert(
            "CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES".to_string(),
            native.context_response_dir.display().to_string(),
        );
        pty_env.insert(
            "CHARIOX_CLAUDE_NATIVE_PERMISSION_RESPONSES".to_string(),
            native.permission_response_dir.display().to_string(),
        );
        pty_env.insert("TERM".to_string(), "xterm-256color".to_string());
        pty_env.insert("COLORTERM".to_string(), "truecolor".to_string());
        let launch = ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "claude:native-tui".to_string(),
            pty_target: None,
            pty_program: Some(executable.display().to_string()),
            pty_args: claude_native_tui_args(
                request,
                &native.settings_file,
                native.mcp_config_file(),
            )?,
            pty_env,
            pty_env_remove: claude_provider_env_remove(Some(request)),
            working_directory: request.working_directory.clone(),
            structured_endpoint: None,
        };
        native.persist_for_launch();
        return Ok(launch);
    }
    if request.provider == CLAUDE_HEADLESS_PROVIDER_ID {
        ensure_claude_headless_onboarding_state()?;
        crate::logging::info_with_fields(
            "daemon.provider.claude",
            "preparing Claude headless native bridge files",
            serde_json::json!({ "provider": request.provider.as_str() }),
        );
        let mut native = prepare_claude_native_tui_files(request)?;
        native.materialize_mcp_config(request)?;
        let mut pty_env = claude_process_env();
        pty_env.insert(
            "CHARIOX_CLAUDE_NATIVE_EVENTS".to_string(),
            native.events_file.display().to_string(),
        );
        pty_env.insert(
            "CHARIOX_CLAUDE_NATIVE_CONTEXT".to_string(),
            native.context_file.display().to_string(),
        );
        pty_env.insert(
            "CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES".to_string(),
            native.context_response_dir.display().to_string(),
        );
        pty_env.insert(
            "CHARIOX_CLAUDE_NATIVE_PERMISSION_RESPONSES".to_string(),
            native.permission_response_dir.display().to_string(),
        );
        pty_env.insert(
            "CHARIOX_CLAUDE_SETTINGS_FILE".to_string(),
            native.settings_file.display().to_string(),
        );
        crate::logging::info_with_fields(
            "daemon.provider.claude",
            "building Claude headless native bridge args",
            serde_json::json!({ "provider": request.provider.as_str() }),
        );
        let pty_args =
            claude_native_tui_args(request, &native.settings_file, native.mcp_config_file())?;
        crate::logging::info_with_fields(
            "daemon.provider.claude",
            "planned Claude headless native bridge launch",
            serde_json::json!({
                "provider": request.provider.as_str(),
                "arg_count": pty_args.len(),
            }),
        );
        let launch = ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "claude:headless".to_string(),
            pty_target: None,
            pty_program: Some(executable.display().to_string()),
            pty_args,
            pty_env,
            pty_env_remove: claude_provider_env_remove(Some(request)),
            working_directory: request.working_directory.clone(),
            structured_endpoint: None,
        };
        native.persist_for_launch();
        return Ok(launch);
    }
    let mut native = prepare_claude_native_tui_files(request)?;
    let mut pty_env = claude_process_env();
    pty_env.insert(
        "CHARIOX_CLAUDE_NATIVE_EVENTS".to_string(),
        native.events_file.display().to_string(),
    );
    pty_env.insert(
        "CHARIOX_CLAUDE_NATIVE_CONTEXT".to_string(),
        native.context_file.display().to_string(),
    );
    pty_env.insert(
        "CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES".to_string(),
        native.context_response_dir.display().to_string(),
    );
    pty_env.insert(
        "CHARIOX_CLAUDE_NATIVE_PERMISSION_RESPONSES".to_string(),
        native.permission_response_dir.display().to_string(),
    );
    pty_env.insert(
        "CHARIOX_CLAUDE_SETTINGS_FILE".to_string(),
        native.settings_file.display().to_string(),
    );
    let mut args = claude_launch_args(request)?;
    args.extend([
        "--settings".to_string(),
        native.settings_file.display().to_string(),
    ]);
    let launch = ProviderLaunchResult {
        endpoint_mode: AgentEndpointMode::External,
        process_label: "claude:stream-json".to_string(),
        pty_target: None,
        pty_program: Some(executable.display().to_string()),
        pty_args: args,
        pty_env,
        pty_env_remove: claude_provider_env_remove(Some(request)),
        working_directory: request.working_directory.clone(),
        structured_endpoint: Some(CLAUDE_STRUCTURED_ENDPOINT.to_string()),
    };
    native.persist_for_launch();
    Ok(launch)
}

fn ensure_claude_headless_onboarding_state() -> Result<(), DaemonError> {
    let state_path = if let Some(config_dir) = env::var_os(CLAUDE_CONFIG_DIR_ENV) {
        PathBuf::from(config_dir).join(CLAUDE_HEADLESS_STATE_FILE)
    } else {
        let home = env::var_os("HOME").ok_or_else(|| DaemonError::LocalTransport {
            operation: "initialize Claude headless onboarding state",
            message: "HOME is unavailable".to_string(),
        })?;
        PathBuf::from(home).join(CLAUDE_HEADLESS_STATE_FILE)
    };
    if state_path.exists() {
        return Ok(());
    }
    let parent = state_path
        .parent()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "initialize Claude headless onboarding state",
            message: "Claude config directory is unavailable".to_string(),
        })?;
    fs::create_dir_all(parent).map_err(|error| DaemonError::LocalTransport {
        operation: "initialize Claude headless onboarding state",
        message: error.to_string(),
    })?;
    let mut nonce = [0_u8; 8];
    rand::thread_rng().fill_bytes(&mut nonce);
    let pending_path = parent.join(format!(
        ".claude-headless-onboarding-{}-{}.tmp",
        std::process::id(),
        u64::from_le_bytes(nonce),
    ));
    let mut pending_options = fs::OpenOptions::new();
    pending_options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        pending_options.mode(0o600);
    }
    let mut pending =
        pending_options
            .open(&pending_path)
            .map_err(|error| DaemonError::LocalTransport {
                operation: "initialize Claude headless onboarding state",
                message: error.to_string(),
            })?;
    let write_result = pending
        .write_all(b"{\"hasCompletedOnboarding\":true}\n")
        .and_then(|()| pending.sync_all());
    if let Err(error) = write_result {
        drop(pending);
        let _ = fs::remove_file(pending_path);
        return Err(DaemonError::LocalTransport {
            operation: "initialize Claude headless onboarding state",
            message: error.to_string(),
        });
    }
    drop(pending);
    let result = match fs::hard_link(&pending_path, &state_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(DaemonError::LocalTransport {
            operation: "initialize Claude headless onboarding state",
            message: error.to_string(),
        }),
    };
    let _ = fs::remove_file(pending_path);
    result
}

fn claude_provider_env_remove(request: Option<&LaunchProviderRequest>) -> Vec<String> {
    let mut names = request
        .map(|request| request.provider_env_remove.clone())
        .unwrap_or_default();
    for name in CLAUDE_AUTH_ENV_VARS {
        if !names.iter().any(|existing| existing == name) {
            names.push((*name).to_string());
        }
    }
    names
}

fn claude_process_env() -> BTreeMap<String, String> {
    BTreeMap::from([(CLAUDE_DISABLE_AUTOUPDATER_ENV.to_string(), "1".to_string())])
}

fn resolve_candidate(candidate: PathBuf, treat_as_literal_path: bool) -> Option<PathBuf> {
    if treat_as_literal_path || candidate.components().count() > 1 {
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
        return resolve_native_claude_binary_from_stub(&candidate);
    }

    if is_executable_file(&candidate) {
        return Some(candidate);
    }

    let path_var = env::var_os("PATH")?;
    for directory in env::split_paths(&path_var) {
        let path = directory.join(&candidate);
        if is_executable_file(&path) {
            return Some(path);
        }
        if let Some(native) = resolve_native_claude_binary_from_stub(&path) {
            return Some(native);
        }
    }
    None
}

fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        return path
            .metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false);
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn resolve_native_claude_binary_from_stub(candidate: &Path) -> Option<PathBuf> {
    let canonical = fs::canonicalize(candidate).ok()?;
    let file_name = canonical.file_name().and_then(|name| name.to_str())?;
    if file_name != "claude" && file_name != "claude.exe" {
        return None;
    }
    let bin_dir = canonical.parent()?;
    if bin_dir.file_name().and_then(|name| name.to_str()) != Some("bin") {
        return None;
    }
    let package_root = bin_dir.parent()?;
    let platform_package = claude_native_platform_package()?;
    let native = package_root
        .join("node_modules")
        .join("@anthropic-ai")
        .join(platform_package)
        .join("claude");
    is_executable_file(&native).then_some(native)
}

fn claude_native_platform_package() -> Option<&'static str> {
    match (env::consts::OS, env::consts::ARCH) {
        ("macos", "aarch64") => Some("claude-code-darwin-arm64"),
        ("macos", "x86_64") => Some("claude-code-darwin-x64"),
        ("linux", "aarch64") => Some("claude-code-linux-arm64"),
        ("linux", "x86_64") => Some("claude-code-linux-x64"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use crate::mcp::CharioxMcpServerConfig;
    use crate::provider::{
        AgentEndpointMode, AgentExecutionMode, AgentPermissionLevel, LaunchProviderRequest,
        ProviderClientInterface, RuntimeMcpBinding,
    };

    use super::{
        claude_provider_catalog, ensure_claude_headless_onboarding_state, plan_claude_launch,
        resolve_claude_executable, CLAUDE_MCP_CONFIG_PLACEHOLDER,
    };

    fn env_guard() -> crate::env_lock::EnvGuard {
        crate::env_lock::lock()
    }

    fn write_executable_fixture(path: &std::path::Path, contents: &str) {
        fs::write(path, contents).expect("fixture should exist");
        #[cfg(unix)]
        {
            let mut permissions = fs::metadata(path)
                .expect("fixture metadata should exist")
                .permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(path, permissions).expect("fixture should be executable");
        }
    }

    #[test]
    fn resolves_override_path_for_tests() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "chariox-claude-resolve-test-{}",
            std::process::id()
        ));
        write_executable_fixture(&path, "#!/bin/sh\nexit 0\n");
        std::env::set_var("CHARIOX_CLAUDE_BIN", &path);

        let resolved = resolve_claude_executable().expect("override path should resolve");

        std::env::remove_var("CHARIOX_CLAUDE_BIN");
        let _ = fs::remove_file(&path);
        assert_eq!(resolved, path);
    }

    #[test]
    fn resolves_native_binary_when_path_shim_is_non_executable_stub() {
        let _guard = env_guard();
        std::env::remove_var("CHARIOX_CLAUDE_BIN");
        let root = std::env::temp_dir().join(format!(
            "chariox-claude-stub-resolve-test-{}",
            std::process::id()
        ));
        let bin_dir = root.join("bin");
        let native_dir = root
            .join("node_modules")
            .join("@anthropic-ai")
            .join(super::claude_native_platform_package().unwrap_or("unsupported"));
        let shim = bin_dir.join("claude.exe");
        let command = root.join("claude");
        let native = native_dir.join("claude");
        fs::create_dir_all(&bin_dir).expect("bin dir should exist");
        fs::create_dir_all(&native_dir).expect("native dir should exist");
        fs::write(&shim, "echo native binary not installed\n").expect("shim should exist");
        write_executable_fixture(&native, "#!/bin/sh\nexit 0\n");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&shim, &command).expect("command shim should link");
        }
        #[cfg(not(unix))]
        {
            fs::copy(&shim, &command).expect("command shim should exist");
        }
        let previous_path = std::env::var_os("PATH");
        std::env::set_var("PATH", &root);

        let resolved = resolve_claude_executable().expect("native binary should resolve");
        let expected = fs::canonicalize(&native).expect("native should canonicalize");

        if let Some(previous_path) = previous_path {
            std::env::set_var("PATH", previous_path);
        } else {
            std::env::remove_var("PATH");
        }
        let _ = fs::remove_dir_all(&root);
        assert_eq!(resolved, expected);
    }

    #[test]
    fn catalog_reads_additional_claude_model_options_cache() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "chariox-claude-config-models-{}.json",
            std::process::id()
        ));
        fs::write(
            &path,
            serde_json::json!({
                "additionalModelOptionsCache": [
                    { "model": "claude-sonnet-4-8", "displayName": "Claude Sonnet 4.8" },
                    "claude-opus-4-8",
                    { "model": "not-a-claude-model", "displayName": "Ignored" }
                ],
                "additionalModelCostsCache": {
                    "claude-haiku-4-5": {}
                }
            })
            .to_string(),
        )
        .expect("fixture should exist");
        std::env::set_var("CHARIOX_CLAUDE_CONFIG", &path);

        let catalog = claude_provider_catalog();

        std::env::remove_var("CHARIOX_CLAUDE_CONFIG");
        let _ = fs::remove_file(&path);

        assert_eq!(
            catalog
                .all
                .iter()
                .map(|provider| provider.id.as_str())
                .collect::<Vec<_>>(),
            vec!["claude-headless", "claude-p"]
        );
        assert_eq!(
            catalog.default.get("claude-headless").map(String::as_str),
            Some("sonnet")
        );
        assert_eq!(
            catalog.default.get("claude-p").map(String::as_str),
            Some("sonnet")
        );
        let models = &catalog.all[0].models;
        assert!(models.contains_key("haiku"));
        assert!(models.contains_key("sonnet"));
        assert!(models.contains_key("opus"));
        assert!(models.contains_key("claude-sonnet-4-6"));
        assert_eq!(
            models
                .get("claude-sonnet-4-8")
                .map(|model| model.name.as_str()),
            Some("Claude Sonnet 4.8")
        );
        assert!(models.contains_key("claude-opus-4-8"));
        assert!(models.contains_key("claude-haiku-4-5"));
        assert!(!models.contains_key("not-a-claude-model"));
    }

    #[test]
    fn plans_structured_stdio_launch_with_permission_mapping() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "chariox-claude-resolve-test-{}-launch",
            std::process::id()
        ));
        write_executable_fixture(&path, "#!/bin/sh\nsleep 60\n");
        std::env::set_var("CHARIOX_CLAUDE_BIN", &path);
        std::env::set_var("ANTHROPIC_API_KEY", "not-used-by-chariox");

        let request = LaunchProviderRequest::new(
            "session-1",
            "claude",
            "claude",
            "default",
            "claude/claude-sonnet-4-6",
        )
        .with_variant(Some("high".to_string()))
        .with_execution_mode(AgentExecutionMode::Plan)
        .with_permission_level(AgentPermissionLevel::Yolo);
        let launch = plan_claude_launch(Some(&request)).expect("launch should resolve");

        std::env::remove_var("CHARIOX_CLAUDE_BIN");
        std::env::remove_var("ANTHROPIC_API_KEY");
        let _ = fs::remove_file(&path);

        assert_eq!(launch.endpoint_mode, AgentEndpointMode::External);
        assert_eq!(
            launch.structured_endpoint.as_deref(),
            Some("stdio://claude")
        );
        assert!(launch
            .pty_args
            .windows(2)
            .any(|pair| pair == ["--model", "claude-sonnet-4-6"]));
        assert!(launch
            .pty_args
            .windows(2)
            .any(|pair| pair == ["--effort", "high"]));
        assert!(launch
            .pty_args
            .windows(2)
            .any(|pair| pair == ["--permission-mode", "plan"]));
        assert!(launch
            .pty_env_remove
            .iter()
            .any(|name| name == "ANTHROPIC_API_KEY"));
        assert_eq!(
            launch
                .pty_env
                .get("DISABLE_AUTOUPDATER")
                .map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn plans_claude_print_mode_with_structured_stdio() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "chariox-claude-resolve-test-{}-print-mode",
            std::process::id()
        ));
        write_executable_fixture(&path, "#!/bin/sh\nsleep 60\n");
        std::env::set_var("CHARIOX_CLAUDE_BIN", &path);

        let request = LaunchProviderRequest::new(
            "session-1",
            "claude",
            "claude-p",
            "default",
            "claude-p/claude-sonnet-4-6",
        );
        let launch = plan_claude_launch(Some(&request)).expect("launch should resolve");

        std::env::remove_var("CHARIOX_CLAUDE_BIN");
        let _ = fs::remove_file(&path);

        assert_eq!(launch.endpoint_mode, AgentEndpointMode::External);
        assert_eq!(
            launch.structured_endpoint.as_deref(),
            Some("stdio://claude")
        );
        assert_eq!(launch.pty_args.first().map(String::as_str), Some("-p"));
        assert_eq!(
            launch
                .pty_env
                .get("DISABLE_AUTOUPDATER")
                .map(String::as_str),
            Some("1")
        );
        assert!(launch
            .pty_args
            .windows(2)
            .any(|pair| pair == ["--model", "claude-sonnet-4-6"]));
    }

    #[test]
    fn plans_claude_headless_mode_without_print_stream_json() {
        let _guard = env_guard();
        let root = std::env::temp_dir().join(format!(
            "chariox-claude-resolve-test-{}-headless-mode",
            std::process::id()
        ));
        let path = root.join("claude");
        let config_dir = root.join("config");
        fs::create_dir_all(&config_dir).expect("config dir should exist");
        write_executable_fixture(&path, "#!/bin/sh\nsleep 60\n");
        std::env::set_var("CHARIOX_CLAUDE_BIN", &path);
        let previous_config_dir = std::env::var_os("CLAUDE_CONFIG_DIR");
        std::env::set_var("CLAUDE_CONFIG_DIR", &config_dir);

        let request = LaunchProviderRequest::new(
            "session-1",
            "claude",
            "claude-headless",
            "default",
            "claude-headless/claude-sonnet-4-6",
        );
        let launch = plan_claude_launch(Some(&request)).expect("launch should resolve");

        std::env::remove_var("CHARIOX_CLAUDE_BIN");
        if let Some(previous_config_dir) = previous_config_dir {
            std::env::set_var("CLAUDE_CONFIG_DIR", previous_config_dir);
        } else {
            std::env::remove_var("CLAUDE_CONFIG_DIR");
        }

        assert_eq!(launch.endpoint_mode, AgentEndpointMode::Managed);
        assert_eq!(launch.process_label, "claude:headless");
        assert_eq!(launch.structured_endpoint, None);
        assert!(!launch.pty_args.iter().any(|arg| arg == "-p"));
        assert!(!launch.pty_args.iter().any(|arg| arg == "stream-json"));
        assert!(launch
            .pty_args
            .windows(2)
            .any(|pair| pair == ["--model", "claude-sonnet-4-6"]));
        assert!(launch.pty_env.contains_key("CHARIOX_CLAUDE_SETTINGS_FILE"));
        assert_eq!(
            launch
                .pty_env
                .get("DISABLE_AUTOUPDATER")
                .map(String::as_str),
            Some("1")
        );
        let state_path = config_dir.join(".claude.json");
        assert_eq!(
            fs::read_to_string(&state_path).expect("headless state should exist"),
            "{\"hasCompletedOnboarding\":true}\n"
        );
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&state_path)
                .expect("headless state metadata should exist")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn preserves_existing_claude_headless_state() {
        let _guard = env_guard();
        let root = std::env::temp_dir().join(format!(
            "chariox-claude-existing-headless-state-{}",
            std::process::id()
        ));
        let config_dir = root.join("config");
        let state_path = config_dir.join(".claude.json");
        fs::create_dir_all(&config_dir).expect("config dir should exist");
        fs::write(&state_path, "{\"existing\":true}\n").expect("existing state should write");
        let previous_config_dir = std::env::var_os("CLAUDE_CONFIG_DIR");
        std::env::set_var("CLAUDE_CONFIG_DIR", &config_dir);

        ensure_claude_headless_onboarding_state().expect("existing state should be accepted");

        if let Some(previous_config_dir) = previous_config_dir {
            std::env::set_var("CLAUDE_CONFIG_DIR", previous_config_dir);
        } else {
            std::env::remove_var("CLAUDE_CONFIG_DIR");
        }
        assert_eq!(
            fs::read_to_string(&state_path).expect("existing state should remain"),
            "{\"existing\":true}\n"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn maps_yolo_build_to_bypass_permissions() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "chariox-claude-resolve-test-{}-yolo",
            std::process::id()
        ));
        write_executable_fixture(&path, "#!/bin/sh\nsleep 60\n");
        std::env::set_var("CHARIOX_CLAUDE_BIN", &path);

        let request = LaunchProviderRequest::new(
            "session-1",
            "claude",
            "claude",
            "default",
            "claude-sonnet-4-6",
        );
        let launch = plan_claude_launch(Some(&request)).expect("launch should resolve");

        std::env::remove_var("CHARIOX_CLAUDE_BIN");
        let _ = fs::remove_file(&path);

        assert!(launch
            .pty_args
            .windows(2)
            .any(|pair| pair == ["--permission-mode", "bypassPermissions"]));
        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg == "--allow-dangerously-skip-permissions"));
    }

    #[test]
    fn injects_runtime_mcp_config_into_launch_args() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "chariox-claude-resolve-test-{}-mcp",
            std::process::id()
        ));
        write_executable_fixture(&path, "#!/bin/sh\nsleep 60\n");
        std::env::set_var("CHARIOX_CLAUDE_BIN", &path);

        let request = LaunchProviderRequest::new(
            "session-1",
            "claude",
            "claude",
            "default",
            "claude-sonnet-4-6",
        )
        .with_runtime_mcp_binding(RuntimeMcpBinding::new(
            "http://127.0.0.1:43120/mcp",
            "token-123",
        ));
        let launch = plan_claude_launch(Some(&request)).expect("launch should resolve");

        std::env::remove_var("CHARIOX_CLAUDE_BIN");
        let _ = fs::remove_file(&path);

        let config_arg = launch
            .pty_args
            .windows(2)
            .find_map(|pair| (pair[0] == "--mcp-config").then(|| pair[1].as_str()))
            .expect("mcp config should be passed");
        assert_eq!(config_arg, CLAUDE_MCP_CONFIG_PLACEHOLDER);
        assert!(launch
            .pty_args
            .iter()
            .all(|arg| !arg.contains("token-123") && !arg.contains("mcpServers")));
        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg == "--strict-mcp-config"));
        assert!(launch
            .pty_args
            .windows(2)
            .any(|pair| pair == ["--disallowedTools", "ToolSearch"]));
    }

    #[test]
    fn metaagent_launch_disables_claude_builtin_tools_but_keeps_runtime_mcp() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "chariox-claude-resolve-test-{}-meta-tools",
            std::process::id()
        ));
        write_executable_fixture(&path, "#!/bin/sh\nsleep 60\n");
        std::env::set_var("CHARIOX_CLAUDE_BIN", &path);

        let request =
            LaunchProviderRequest::new("session-1", "claude", "claude-p", "default", "haiku")
                .with_runtime_mcp_binding(RuntimeMcpBinding::new(
                    "http://127.0.0.1:43120/mcp",
                    "token-123",
                ))
                .with_provider_config_override(
                    "chariox.metaagent_tools_only",
                    serde_json::json!(true),
                );
        let launch = plan_claude_launch(Some(&request)).expect("launch should resolve");

        std::env::remove_var("CHARIOX_CLAUDE_BIN");
        let _ = fs::remove_file(&path);

        assert!(launch
            .pty_args
            .windows(2)
            .any(|pair| pair == ["--tools", ""]));
        assert!(launch.pty_args.iter().any(|arg| arg == "--mcp-config"));
    }

    #[test]
    fn injects_mcp_config_into_native_tui_launch_args() {
        let _guard = env_guard();
        let path = std::env::temp_dir().join(format!(
            "chariox-claude-resolve-test-{}-native-mcp",
            std::process::id()
        ));
        write_executable_fixture(&path, "#!/bin/sh\nsleep 60\n");
        std::env::set_var("CHARIOX_CLAUDE_BIN", &path);

        let request = LaunchProviderRequest::new(
            "session-1",
            "claude",
            "claude",
            "default",
            "claude-sonnet-4-6",
        )
        .with_client_interface(ProviderClientInterface::NativeTui)
        .with_runtime_mcp_binding(RuntimeMcpBinding::new(
            "http://127.0.0.1:43120/mcp",
            "token-123",
        ))
        .with_mcp_servers(vec![CharioxMcpServerConfig::stdio(
            "browser",
            "npx",
            vec!["@playwright/mcp@latest".to_string()],
        )]);
        let launch = plan_claude_launch(Some(&request)).expect("launch should resolve");

        std::env::remove_var("CHARIOX_CLAUDE_BIN");
        let _ = fs::remove_file(&path);

        assert_eq!(launch.endpoint_mode, AgentEndpointMode::Managed);
        assert_eq!(
            launch
                .pty_env
                .get("DISABLE_AUTOUPDATER")
                .map(String::as_str),
            Some("1")
        );
        let config_arg = launch
            .pty_args
            .windows(2)
            .find_map(|pair| (pair[0] == "--mcp-config").then(|| pair[1].as_str()))
            .expect("mcp config should be passed");
        let config_path = std::path::PathBuf::from(config_arg);
        let config_root = config_path
            .parent()
            .expect("config should have a root")
            .to_path_buf();
        assert!(config_path.is_file());
        assert!(launch
            .pty_args
            .iter()
            .all(|arg| !arg.contains("token-123") && !arg.contains("mcpServers")));
        #[cfg(unix)]
        assert_eq!(
            std::fs::metadata(&config_path)
                .expect("config metadata should exist")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let config: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&config_path).expect("config should be readable by the kernel"),
        )
        .expect("config should be JSON");
        assert_eq!(
            config.pointer("/mcpServers/chariox/url"),
            Some(&serde_json::json!("http://127.0.0.1:43120/mcp"))
        );
        assert!(config.pointer("/mcpServers/browser").is_some());
        assert!(launch
            .pty_args
            .iter()
            .any(|arg| arg == "--strict-mcp-config"));
        assert!(launch
            .pty_args
            .windows(2)
            .any(|pair| pair == ["--allowedTools", "mcp__chariox__*"]));
        std::fs::remove_dir_all(config_root).expect("test config root should clean up");
    }
}
