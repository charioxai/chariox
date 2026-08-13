mod dev_stub_adapter;

use super::{
    apply_workspace_write_fence, plan_claude_launch, plan_codex_launch, plan_opencode_launch,
    workspace_write_fence_supported, LaunchProviderRequest, ProviderLaunchResult,
    RuntimeProviderRun,
};
use crate::error::DaemonError;

use dev_stub_adapter::{DevStubAdapter, DEV_STUB_ADAPTER};
#[cfg(test)]
use dev_stub_adapter::{
    FailingPtyAdapter, ManagedDevStubAdapter, FAILING_PTY_ADAPTER, MANAGED_DEV_STUB_ADAPTER,
};

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
        let mut launch = plan_claude_launch(Some(request))?;
        launch.process_label = format!("claude:{}:{}", request.provider, request.model);
        launch.working_directory = request.working_directory.clone();
        apply_workspace_write_fence(launch, request)
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
        let mut launch = plan_opencode_launch(Some(request))?;
        launch.process_label = format!("opencode:{}:{}", request.provider, request.model);
        launch.pty_target = None;
        launch.working_directory = request.working_directory.clone();
        apply_workspace_write_fence(launch, request)
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
        let mut launch = plan_codex_launch(Some(request))?;
        launch.process_label = format!("codex:{}:{}", request.provider, request.model);
        launch.pty_target = None;
        launch.working_directory = request.working_directory.clone();
        apply_workspace_write_fence(launch, request)
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
}
