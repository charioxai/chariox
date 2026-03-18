use super::{
    resolve_opencode_executable, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
};
use crate::error::DaemonError;

pub trait ProviderAdapter: Send + Sync {
    fn key(&self) -> &'static str;
    fn launch(&self, request: &LaunchProviderRequest) -> Result<ProviderLaunchResult, DaemonError>;
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

    pub fn resolve(&self, key: &str) -> Option<&'static dyn ProviderAdapter> {
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

impl ProviderAdapter for DevStubAdapter {
    fn key(&self) -> &'static str {
        Self::KEY
    }

    fn launch(&self, request: &LaunchProviderRequest) -> Result<ProviderLaunchResult, DaemonError> {
        Ok(ProviderLaunchResult {
            process_label: format!(
                "dev-stub:{}:{}:{}",
                request.provider, request.account_profile, request.model
            ),
            pty_target: Some(format!("stub-pty:{}", request.session_id)),
            pty_program: "/bin/sh".to_string(),
            pty_args: vec!["-lc".to_string(), "cat".to_string()],
            working_directory: request.working_directory.clone(),
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

impl ProviderAdapter for OpenCodeAdapter {
    fn key(&self) -> &'static str {
        Self::KEY
    }

    fn launch(&self, request: &LaunchProviderRequest) -> Result<ProviderLaunchResult, DaemonError> {
        let executable = resolve_opencode_executable()?;
        let mut pty_args = Vec::new();

        if !request.model.trim().is_empty() && request.model != "default" {
            pty_args.push("--model".to_string());
            pty_args.push(request.model.clone());
        }

        if let Some(working_directory) = request.working_directory.as_ref() {
            pty_args.push(working_directory.display().to_string());
        }

        Ok(ProviderLaunchResult {
            process_label: format!("opencode:{}:{}", request.provider, request.model),
            pty_target: Some(format!("opencode-pty:{}", request.session_id)),
            pty_program: executable.display().to_string(),
            pty_args,
            working_directory: request.working_directory.clone(),
        })
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
impl ProviderAdapter for FailingPtyAdapter {
    fn key(&self) -> &'static str {
        Self::KEY
    }

    fn launch(&self, request: &LaunchProviderRequest) -> Result<ProviderLaunchResult, DaemonError> {
        Ok(ProviderLaunchResult {
            process_label: format!(
                "dev-invalid-pty:{}:{}:{}",
                request.provider, request.account_profile, request.model
            ),
            pty_target: Some(format!("invalid-pty:{}", request.session_id)),
            pty_program: "/definitely/not/a/real/provider".to_string(),
            pty_args: Vec::new(),
            working_directory: request.working_directory.clone(),
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
        fs::write(&executable, "#!/bin/sh\ncat\n").expect("fixture executable should exist");
        std::env::set_var("ARROBA_OPENCODE_BIN", &executable);

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
            .launch(&request)
            .expect("opencode launch should resolve");

        std::env::remove_var("ARROBA_OPENCODE_BIN");
        let _ = fs::remove_file(&executable);

        assert_eq!(launch_result.pty_program, executable.display().to_string());
        assert_eq!(launch_result.working_directory, Some(PathBuf::from("/tmp")));
        assert_eq!(
            launch_result.pty_args,
            vec![
                "--model".to_string(),
                "anthropic/claude-sonnet-4".to_string(),
                "/tmp".to_string()
            ]
        );
    }
}
