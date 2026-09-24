mod dev_stub_adapter;

use super::{
    apply_managed_provider_isolation, apply_workspace_write_fence, plan_claude_launch,
    plan_codex_launch, plan_opencode_launch, workspace_write_fence_supported,
    LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
};
use crate::error::DaemonError;

use dev_stub_adapter::{DevStubAdapter, DEV_STUB_ADAPTER};
#[cfg(test)]
use dev_stub_adapter::{
    FailingPtyAdapter, ManagedDevStubAdapter, FAILING_PTY_ADAPTER, MANAGED_DEV_STUB_ADAPTER,
};

fn preflight_provider_working_directory(
    request: &LaunchProviderRequest,
) -> Result<(), DaemonError> {
    if !(crate::provider::managed_provider_isolation_required()
        && request.uses_workspace_live_sync())
    {
        if let Some(working_directory) = request.working_directory.as_deref() {
            crate::git_worktree_placement::preflight_working_directory(
                working_directory,
                "provider launch",
                false,
                &[],
            )?;
        }
    }
    Ok(())
}

pub trait AgentEndpointAdapter: Send + Sync {
    fn key(&self) -> &'static str;
    fn connect(&self, request: &LaunchProviderRequest)
        -> Result<ProviderLaunchResult, DaemonError>;
    fn supports_workspace_live_sync_write_enforcement(&self) -> bool {
        false
    }
    fn workspace_live_sync_write_enforcement_unavailable_reason(&self) -> &'static str {
        "this adapter cannot guarantee that provider-session writes are restricted to Chariox workspace live sync tools"
    }
    fn supports_turn_scoped_execution_config(&self) -> bool {
        false
    }
    fn park(&self, run: &RuntimeProviderRun);
    fn resume(&self, run: &RuntimeProviderRun);
    fn terminate(&self, run: &RuntimeProviderRun);
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ProviderRegistry;

impl ProviderRegistry {
    pub fn new() -> Self {
        Self
    }

    pub fn registered_adapter_count(&self) -> usize {
        #[cfg(test)]
        {
            6
        }

        #[cfg(not(test))]
        {
            4
        }
    }

    pub fn resolve(&self, key: &str) -> Option<&'static dyn AgentEndpointAdapter> {
        match key {
            DevStubAdapter::KEY => Some(&DEV_STUB_ADAPTER),
            ClaudeAdapter::KEY => Some(&CLAUDE_ADAPTER),
            CodexAdapter::KEY => Some(&CODEX_ADAPTER),
            OpenCodeAdapter::KEY => Some(&OPENCODE_ADAPTER),
            #[cfg(test)]
            ManagedDevStubAdapter::KEY => Some(&MANAGED_DEV_STUB_ADAPTER),
            #[cfg(test)]
            FailingPtyAdapter::KEY => Some(&FAILING_PTY_ADAPTER),
            _ => None,
        }
    }

    pub fn registered_adapter_keys(&self) -> Vec<String> {
        let keys = vec![
            DevStubAdapter::KEY.to_string(),
            ClaudeAdapter::KEY.to_string(),
            CodexAdapter::KEY.to_string(),
            OpenCodeAdapter::KEY.to_string(),
        ];
        #[cfg(not(test))]
        return keys;
        #[cfg(test)]
        {
            let mut keys = keys;
            keys.push(ManagedDevStubAdapter::KEY.to_string());
            keys.push(FailingPtyAdapter::KEY.to_string());
            keys
        }
    }

    pub fn advertised_provider_ids(&self) -> Vec<String> {
        let ids = vec![
            DevStubAdapter::KEY.to_string(),
            ClaudeAdapter::KEY.to_string(),
            "claude-headless".to_string(),
            "claude-p".to_string(),
            CodexAdapter::KEY.to_string(),
            OpenCodeAdapter::KEY.to_string(),
        ];
        #[cfg(not(test))]
        return ids;
        #[cfg(test)]
        {
            let mut ids = ids;
            ids.push(ManagedDevStubAdapter::KEY.to_string());
            ids.push(FailingPtyAdapter::KEY.to_string());
            ids
        }
    }
}

#[derive(Debug, Default)]
struct ClaudeAdapter;

impl ClaudeAdapter {
    const KEY: &'static str = "claude";
}

static CLAUDE_ADAPTER: ClaudeAdapter = ClaudeAdapter;

impl AgentEndpointAdapter for ClaudeAdapter {
    fn key(&self) -> &'static str {
        Self::KEY
    }

    fn supports_workspace_live_sync_write_enforcement(&self) -> bool {
        workspace_write_fence_supported()
    }

    fn workspace_live_sync_write_enforcement_unavailable_reason(&self) -> &'static str {
        "managed workspace live sync needs selective write fencing, which is only implemented on macOS for this adapter; use tracked mode on this worker or run the managed provider on a supported host"
    }

    fn connect(
        &self,
        request: &LaunchProviderRequest,
    ) -> Result<ProviderLaunchResult, DaemonError> {
        preflight_provider_working_directory(request)?;
        let mut launch = plan_claude_launch(Some(request))?;
        launch.process_label = format!("claude:{}:{}", request.provider, request.model);
        launch.working_directory = request.working_directory.clone();
        let launch = apply_workspace_write_fence(launch, request)?;
        apply_managed_provider_isolation(launch, request)
    }

    fn park(&self, _run: &RuntimeProviderRun) {}

    fn resume(&self, _run: &RuntimeProviderRun) {}

    fn terminate(&self, _run: &RuntimeProviderRun) {}
}

#[derive(Debug, Default)]
struct OpenCodeAdapter;

impl OpenCodeAdapter {
    const KEY: &'static str = "opencode";
}

static OPENCODE_ADAPTER: OpenCodeAdapter = OpenCodeAdapter;

impl AgentEndpointAdapter for OpenCodeAdapter {
    fn key(&self) -> &'static str {
        Self::KEY
    }

    fn supports_workspace_live_sync_write_enforcement(&self) -> bool {
        workspace_write_fence_supported()
    }

    fn workspace_live_sync_write_enforcement_unavailable_reason(&self) -> &'static str {
        "managed workspace live sync needs selective write fencing, which is only implemented on macOS for this adapter; use tracked mode on this worker or run the managed provider on a supported host"
    }

    fn connect(
        &self,
        request: &LaunchProviderRequest,
    ) -> Result<ProviderLaunchResult, DaemonError> {
        preflight_provider_working_directory(request)?;
        let mut launch = plan_opencode_launch(Some(request))?;
        launch.process_label = format!("opencode:{}:{}", request.provider, request.model);
        launch.pty_target = None;
        launch.working_directory = request.working_directory.clone();
        let launch = apply_workspace_write_fence(launch, request)?;
        apply_managed_provider_isolation(launch, request)
    }

    fn park(&self, _run: &RuntimeProviderRun) {}

    fn resume(&self, _run: &RuntimeProviderRun) {}

    fn terminate(&self, _run: &RuntimeProviderRun) {}
}

#[derive(Debug, Default)]
struct CodexAdapter;

impl CodexAdapter {
    const KEY: &'static str = "codex";
}

static CODEX_ADAPTER: CodexAdapter = CodexAdapter;

impl AgentEndpointAdapter for CodexAdapter {
    fn key(&self) -> &'static str {
        Self::KEY
    }

    fn supports_workspace_live_sync_write_enforcement(&self) -> bool {
        workspace_write_fence_supported()
    }

    fn workspace_live_sync_write_enforcement_unavailable_reason(&self) -> &'static str {
        "managed workspace live sync needs selective write fencing, which is only implemented on macOS for this adapter; use tracked mode on this worker or run the managed provider on a supported host"
    }

    fn supports_turn_scoped_execution_config(&self) -> bool {
        true
    }

    fn connect(
        &self,
        request: &LaunchProviderRequest,
    ) -> Result<ProviderLaunchResult, DaemonError> {
        preflight_provider_working_directory(request)?;
        let mut launch = plan_codex_launch(Some(request))?;
        launch.process_label = format!("codex:{}:{}", request.provider, request.model);
        launch.pty_target = None;
        launch.working_directory = request.working_directory.clone();
        let launch = apply_workspace_write_fence(launch, request)?;
        apply_managed_provider_isolation(launch, request)
    }

    fn park(&self, _run: &RuntimeProviderRun) {}

    fn resume(&self, _run: &RuntimeProviderRun) {}

    fn terminate(&self, _run: &RuntimeProviderRun) {}
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::{LaunchProviderRequest, ProviderRegistry};

    #[test]
    fn advertised_provider_ids_include_native_and_backend_provider_modes() {
        let ids = ProviderRegistry::new().advertised_provider_ids();

        for provider in [
            "dev-stub",
            "claude",
            "claude-headless",
            "claude-p",
            "codex",
            "opencode",
        ] {
            assert!(
                ids.iter().any(|candidate| candidate == provider),
                "advertised provider ids should include {provider}; got {ids:?}",
            );
        }
    }

    #[test]
    fn managed_workspace_live_sync_support_matches_selective_write_fence_support() {
        let registry = ProviderRegistry::new();
        assert_eq!(
            registry
                .resolve("opencode")
                .expect("opencode adapter should exist")
                .supports_workspace_live_sync_write_enforcement(),
            cfg!(target_os = "macos"),
        );
        assert_eq!(
            registry
                .resolve("codex")
                .expect("codex adapter should exist")
                .supports_workspace_live_sync_write_enforcement(),
            cfg!(target_os = "macos"),
        );
        assert_eq!(
            registry
                .resolve("claude")
                .expect("claude adapter should exist")
                .supports_workspace_live_sync_write_enforcement(),
            cfg!(target_os = "macos"),
        );
        assert!(registry
            .resolve("opencode")
            .expect("opencode adapter should exist")
            .workspace_live_sync_write_enforcement_unavailable_reason()
            .contains("use tracked mode"));
        assert!(registry
            .resolve("claude")
            .expect("claude adapter should exist")
            .workspace_live_sync_write_enforcement_unavailable_reason()
            .contains("use tracked mode"));
    }

    #[test]
    fn opencode_adapter_resolves_override_and_uses_working_directory() {
        let _guard = crate::env_lock::lock();
        let executable = std::env::temp_dir().join(format!(
            "chariox-opencode-adapter-test-{}",
            std::process::id()
        ));
        fs::write(&executable, "#!/bin/sh\nsleep 60\n").expect("fixture executable should exist");
        std::env::set_var("CHARIOX_OPENCODE_BIN", &executable);
        std::env::set_var("CHARIOX_OPENCODE_PORT", "43112");

        let request = LaunchProviderRequest::new(
            "session-1",
            "opencode",
            "opencode",
            "default",
            "anthropic/claude-sonnet-4",
        )
        .with_working_directory(PathBuf::from("/tmp"));
        let launch_result = ProviderRegistry::new()
            .resolve("opencode")
            .expect("opencode adapter should exist")
            .connect(&request)
            .expect("opencode launch should resolve");

        std::env::remove_var("CHARIOX_OPENCODE_BIN");
        std::env::remove_var("CHARIOX_OPENCODE_PORT");
        let _ = fs::remove_file(&executable);

        let expected_program = executable.to_string_lossy().to_string();
        assert_eq!(
            launch_result.pty_program.as_deref(),
            Some(expected_program.as_str())
        );
        assert_eq!(launch_result.working_directory, Some(PathBuf::from("/tmp")));
        assert_eq!(launch_result.pty_args[0], "serve");
        assert_eq!(launch_result.pty_args[1], "--hostname");
        assert_eq!(launch_result.pty_args[2], "127.0.0.1");
        assert_eq!(launch_result.pty_args[3], "--port");
        let port = launch_result.pty_args[4]
            .parse::<u16>()
            .expect("port argument should be numeric");
        let endpoint = format!("http://127.0.0.1:{port}");
        assert_eq!(
            launch_result.structured_endpoint.as_deref(),
            Some(endpoint.as_str())
        );
    }

    #[test]
    fn ordinary_opencode_adapter_scrubs_path1_supervisor_controls_only() {
        let _guard = crate::env_lock::lock();
        let root =
            std::env::temp_dir().join(format!("chariox-registry-path1-env-{}", std::process::id()));
        fs::create_dir_all(&root).expect("adapter fixture root should exist");
        let executable = root.join("opencode");
        fs::write(&executable, "#!/bin/sh\n").expect("fixture executable should exist");

        let controls = [
            "CHARIOX_MANAGED_REPOSITORY_ROOT",
            "CHARIOX_KERNEL_HOST",
            "CHARIOX_KERNEL_PORT",
            "CHARIOX_ACCEPT_REMOTE_LEASES",
            "CHARIOX_KERNEL_RUNTIME_ROLE",
            "CHARIOX_REMOTE_LEASE_CAPACITY",
            "CHARIOX_LEASE_WORKER_HOME_CALLER",
            "CHARIOX_MANAGED_PROVIDER_TOPOLOGY",
            "CHARIOX_MANAGED_RELEASE_MANIFEST",
            "CHARIOX_MANAGED_KERNEL_BINARY",
        ];
        let mut names = controls.to_vec();
        names.extend([
            "CHARIOX_MANAGED_PROVIDER_ISOLATION",
            "CHARIOX_OPENCODE_BIN",
            "CHARIOX_OPENCODE_PORT",
        ]);
        let previous = names
            .iter()
            .map(|name| (*name, std::env::var_os(name)))
            .collect::<Vec<_>>();
        for name in controls {
            std::env::set_var(name, "supervisor-only-value");
        }
        std::env::remove_var("CHARIOX_MANAGED_PROVIDER_ISOLATION");
        std::env::set_var("CHARIOX_OPENCODE_BIN", &executable);
        std::env::set_var("CHARIOX_OPENCODE_PORT", "43112");
        let expected_program = executable.display().to_string();

        let mut provider_environment = std::collections::BTreeMap::from([(
            "CHARIOX_PROVIDER_ENV_TEST".to_string(),
            "preserved-provider-value".to_string(),
        )]);
        for name in controls {
            provider_environment.insert(name.to_string(), "provider-supplied-control".into());
        }
        let request = LaunchProviderRequest::new(
            "session-path1-environment",
            "opencode",
            "opencode",
            "default",
            "anthropic/claude-sonnet-4",
        )
        .with_provider_account_env(provider_environment);
        let result = ProviderRegistry::new()
            .resolve("opencode")
            .expect("opencode adapter should exist")
            .connect(&request);

        for (name, value) in previous {
            restore_env(name, value);
        }
        let _ = fs::remove_dir_all(root);
        let launch = result.expect("ordinary Path-1 adapter launch should resolve");
        assert_eq!(
            launch.pty_program.as_deref(),
            Some(expected_program.as_str()),
            "ordinary Path-1 must keep the provider executable as the child program"
        );

        for name in controls {
            assert!(
                launch.pty_env_remove.iter().any(|removed| removed == name),
                "ordinary provider adapter must remove inherited control {name}"
            );
            assert!(
                !launch.pty_env.contains_key(name),
                "ordinary provider adapter must discard explicit control {name}"
            );
        }
        for name in ["HOME", "PATH", "CHARIOX_PROVIDER_ENV_TEST"] {
            assert!(
                !launch.pty_env_remove.iter().any(|removed| removed == name),
                "ordinary provider adapter must preserve environment {name}"
            );
        }
        assert_eq!(
            launch
                .pty_env
                .get("CHARIOX_PROVIDER_ENV_TEST")
                .map(String::as_str),
            Some("preserved-provider-value")
        );
    }

    #[test]
    fn ordinary_provider_adapter_does_not_reexpose_live_sync_roots() {
        let _guard = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-registry-ordinary-preflight-{}",
            std::process::id()
        ));
        let service_root = root.join("service-state");
        let working_directory = service_root.join("workspace");
        let executable = root.join("opencode");
        fs::create_dir_all(&working_directory).expect("working directory should exist");
        fs::write(&executable, "#!/bin/sh\n").expect("fixture executable should exist");

        let previous_service_root = std::env::var_os("CHARIOX_MANAGED_SLICE_SERVICE_ROOT");
        let previous_isolation = std::env::var_os("CHARIOX_MANAGED_PROVIDER_ISOLATION");
        let previous_binary = std::env::var_os("CHARIOX_OPENCODE_BIN");
        let previous_port = std::env::var_os("CHARIOX_OPENCODE_PORT");
        std::env::set_var("CHARIOX_MANAGED_SLICE_SERVICE_ROOT", &service_root);
        std::env::remove_var("CHARIOX_MANAGED_PROVIDER_ISOLATION");
        std::env::set_var("CHARIOX_OPENCODE_BIN", &executable);
        std::env::set_var("CHARIOX_OPENCODE_PORT", "43112");

        let request = LaunchProviderRequest::new(
            "session-ordinary-preflight",
            "opencode",
            "opencode",
            "default",
            "anthropic/claude-sonnet-4",
        )
        .with_working_directory(working_directory.clone())
        .with_workspace_live_sync_roots(vec![working_directory]);
        let error = ProviderRegistry::new()
            .resolve("opencode")
            .expect("opencode adapter should exist")
            .connect(&request)
            .expect_err("ordinary live-sync roots must not exempt protected state");

        restore_env("CHARIOX_MANAGED_SLICE_SERVICE_ROOT", previous_service_root);
        restore_env("CHARIOX_MANAGED_PROVIDER_ISOLATION", previous_isolation);
        restore_env("CHARIOX_OPENCODE_BIN", previous_binary);
        restore_env("CHARIOX_OPENCODE_PORT", previous_port);
        let _ = fs::remove_dir_all(root);

        assert!(error
            .to_string()
            .contains("protected Chariox service state"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn official_opencode_adapter_reaches_filtered_managed_child_reexposure() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let _guard = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-registry-managed-child-{}",
            std::process::id()
        ));
        let service_root = root.join("service-state");
        let publication_root = service_root.join("publication");
        let selected = publication_root.join("selected-repository");
        let working_directory = selected.join("nested");
        let provider_home = root.join("provider-home");
        let executable = root.join("opencode");
        let bubblewrap = root.join("bwrap");
        fs::create_dir_all(&working_directory).expect("selected child should exist");
        fs::create_dir_all(&provider_home).expect("provider home should exist");
        fs::write(&executable, "#!/bin/sh\n").expect("fixture executable should exist");
        fs::write(&bubblewrap, "managed bubblewrap fixture\n")
            .expect("bubblewrap fixture should exist");
        fs::set_permissions(&bubblewrap, fs::Permissions::from_mode(0o755))
            .expect("bubblewrap fixture should be executable");
        if fs::metadata(&bubblewrap)
            .expect("bubblewrap fixture metadata should exist")
            .uid()
            != 0
        {
            let _ = fs::remove_dir_all(root);
            eprintln!("skipped managed child adapter test: fixture bubblewrap is not root-owned");
            return;
        }

        let names = [
            "CHARIOX_MANAGED_PROVIDER_ISOLATION",
            "CHARIOX_MANAGED_PROVIDER_BWRAP",
            "CHARIOX_MANAGED_PROVIDER_HOME",
            "CHARIOX_MANAGED_SLICE_SERVICE_ROOT",
            "CHARIOX_MANAGED_SLICE_PUBLICATION_ROOT",
            "CHARIOX_OPENCODE_BIN",
            "CHARIOX_OPENCODE_PORT",
        ];
        let previous = names
            .iter()
            .map(|name| (*name, std::env::var_os(name)))
            .collect::<Vec<_>>();
        std::env::set_var("CHARIOX_MANAGED_PROVIDER_ISOLATION", "1");
        std::env::set_var("CHARIOX_MANAGED_PROVIDER_BWRAP", &bubblewrap);
        std::env::set_var("CHARIOX_MANAGED_PROVIDER_HOME", &provider_home);
        std::env::set_var("CHARIOX_MANAGED_SLICE_SERVICE_ROOT", &service_root);
        std::env::set_var("CHARIOX_MANAGED_SLICE_PUBLICATION_ROOT", &publication_root);
        std::env::set_var("CHARIOX_OPENCODE_BIN", &executable);
        std::env::set_var("CHARIOX_OPENCODE_PORT", "43112");

        let request = LaunchProviderRequest::new(
            "session-managed-child",
            "opencode",
            "opencode",
            "default",
            "anthropic/claude-sonnet-4",
        )
        .with_working_directory(working_directory.clone())
        .with_workspace_live_sync_mode(crate::config::WorkspaceLiveSyncMode::Tracked)
        .with_workspace_live_sync_roots(vec![selected.clone()]);
        let launch = ProviderRegistry::new()
            .resolve("opencode")
            .expect("opencode adapter should exist")
            .connect(&request)
            .expect("managed child should reach the filtered re-exposure check");

        for (name, value) in previous {
            restore_env(name, value);
        }
        let _ = fs::remove_dir_all(root);

        let bubblewrap_text = bubblewrap.display().to_string();
        assert_eq!(
            launch.pty_program.as_deref(),
            Some(bubblewrap_text.as_str())
        );
        assert_eq!(launch.working_directory, Some(working_directory));
        let selected_text = selected.display().to_string();
        assert!(launch.pty_args.windows(3).any(|window| {
            window == ["--bind", selected_text.as_str(), selected_text.as_str()]
        }));
    }

    fn restore_env(name: &str, value: Option<std::ffi::OsString>) {
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }
}
