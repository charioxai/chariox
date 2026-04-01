use super::{
    plan_opencode_launch, AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult,
    RuntimeProviderRun,
};
use crate::error::DaemonError;

pub trait AgentEndpointAdapter: Send + Sync {
    fn key(&self) -> &'static str;
    fn connect(&self, request: &LaunchProviderRequest) -> Result<ProviderLaunchResult, DaemonError>;
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
            3
        }

        #[cfg(not(test))]
        {
            2
        }
    }

    pub fn resolve(&self, key: &str) -> Option<&'static dyn AgentEndpointAdapter> {
        match key {
            DevStubAdapter::KEY => Some(&DEV_STUB_ADAPTER),
            OpenCodeAdapter::KEY => Some(&OPENCODE_ADAPTER),
            #[cfg(test)]
            FailingPtyAdapter::KEY => Some(&FAILING_PTY_ADAPTER),
            _ => None,
        }
    }
}

#[derive(Debug, Default)]
struct DevStubAdapter;

impl DevStubAdapter {
    const KEY: &'static str = "dev-stub";
}

static DEV_STUB_ADAPTER: DevStubAdapter = DevStubAdapter;

impl AgentEndpointAdapter for DevStubAdapter {
    fn key(&self) -> &'static str {
        Self::KEY
    }

    fn connect(&self, request: &LaunchProviderRequest) -> Result<ProviderLaunchResult, DaemonError> {
        Ok(ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: format!(
                "dev-stub:{}:{}:{}",
                request.provider, request.account_profile, request.model
            ),
            pty_target: Some(format!("stub-pty:{}", request.session_id)),
            pty_program: Some("/bin/sh".to_string()),
            pty_args: vec!["-lc".to_string(), "cat".to_string()],
            working_directory: request.working_directory.clone(),
            structured_endpoint: None,
        })
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

    fn connect(&self, request: &LaunchProviderRequest) -> Result<ProviderLaunchResult, DaemonError> {
        let mut launch = plan_opencode_launch()?;
        launch.process_label = format!("opencode:{}:{}", request.provider, request.model);
        launch.pty_target = Some(format!("opencode-pty:{}", request.session_id));
        launch.working_directory = request.working_directory.clone();
        Ok(launch)
    }

    fn park(&self, _run: &RuntimeProviderRun) {}

    fn resume(&self, _run: &RuntimeProviderRun) {}

    fn terminate(&self, _run: &RuntimeProviderRun) {}
}

#[cfg(test)]
#[derive(Debug, Default)]
struct FailingPtyAdapter;

#[cfg(test)]
impl FailingPtyAdapter {
    const KEY: &'static str = "dev-invalid-pty";
}

#[cfg(test)]
static FAILING_PTY_ADAPTER: FailingPtyAdapter = FailingPtyAdapter;

#[cfg(test)]
impl AgentEndpointAdapter for FailingPtyAdapter {
    fn key(&self) -> &'static str {
        Self::KEY
    }

    fn connect(&self, request: &LaunchProviderRequest) -> Result<ProviderLaunchResult, DaemonError> {
        Ok(ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: format!(
                "dev-invalid-pty:{}:{}:{}",
                request.provider, request.account_profile, request.model
            ),
            pty_target: Some(format!("invalid-pty:{}", request.session_id)),
            pty_program: Some("/definitely/not/a/real/provider".to_string()),
            pty_args: Vec::new(),
            working_directory: request.working_directory.clone(),
            structured_endpoint: None,
        })
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
    fn opencode_adapter_resolves_override_and_uses_working_directory() {
        let executable = std::env::temp_dir().join(format!(
            "arroba-opencode-adapter-test-{}",
            std::process::id()
        ));
        fs::write(&executable, "#!/bin/sh\nsleep 60\n").expect("fixture executable should exist");
        std::env::set_var("ARROBA_OPENCODE_BIN", &executable);
        std::env::set_var("ARROBA_OPENCODE_PORT", "43112");

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

        std::env::remove_var("ARROBA_OPENCODE_BIN");
        std::env::remove_var("ARROBA_OPENCODE_PORT");
        let _ = fs::remove_file(&executable);

        let expected_program = executable.to_string_lossy().to_string();
        assert_eq!(launch_result.pty_program.as_deref(), Some(expected_program.as_str()));
        assert_eq!(launch_result.working_directory, Some(PathBuf::from("/tmp")));
        assert_eq!(launch_result.pty_args[0], "serve");
        assert_eq!(launch_result.pty_args[1], "--hostname");
        assert_eq!(launch_result.pty_args[2], "127.0.0.1");
        assert_eq!(launch_result.pty_args[3], "--port");
        let port = launch_result.pty_args[4]
            .parse::<u16>()
            .expect("port argument should be numeric");
        assert!(port >= 43112, "port should be >= 43112, got {}", port);
        let endpoint = format!("http://127.0.0.1:{port}");
        assert_eq!(
            launch_result.structured_endpoint.as_deref(),
            Some(endpoint.as_str())
        );
    }
}
