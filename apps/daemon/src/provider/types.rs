use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderRunState {
    Starting,
    Running,
    Parked,
    Ended,
}

impl fmt::Display for ProviderRunState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Parked => "parked",
            Self::Ended => "ended",
        };

        write!(f, "{value}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchProviderRequest {
    pub session_id: String,
    pub adapter_key: String,
    pub provider: String,
    pub account_profile: String,
    pub model: String,
    pub variant: Option<String>,
    pub working_directory: Option<PathBuf>,
}

impl LaunchProviderRequest {
    pub fn new(
        session_id: impl Into<String>,
        adapter_key: impl Into<String>,
        provider: impl Into<String>,
        account_profile: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            adapter_key: adapter_key.into(),
            provider: provider.into(),
            account_profile: account_profile.into(),
            model: model.into(),
            variant: None,
            working_directory: None,
        }
    }

    pub fn with_variant(mut self, variant: Option<String>) -> Self {
        self.variant = variant.and_then(|value| {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_string())
        });
        self
    }

    pub fn with_working_directory(mut self, working_directory: PathBuf) -> Self {
        self.working_directory = Some(working_directory);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderLaunchResult {
    pub process_label: String,
    pub pty_target: Option<String>,
    pub pty_program: String,
    pub pty_args: Vec<String>,
    pub working_directory: Option<PathBuf>,
    pub structured_endpoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeProviderRun {
    id: String,
    session_id: String,
    adapter_key: String,
    provider: String,
    account_profile: String,
    model: String,
    variant: Option<String>,
    usage_tokens_total: Option<u64>,
    state: ProviderRunState,
    process_label: String,
    pty_target: Option<String>,
    pty_program: String,
    pty_args: Vec<String>,
    working_directory: Option<PathBuf>,
    structured_endpoint: Option<String>,
}

impl RuntimeProviderRun {
    pub fn new(
        id: impl Into<String>,
        request: &LaunchProviderRequest,
        launch_result: ProviderLaunchResult,
    ) -> Self {
        Self {
            id: id.into(),
            session_id: request.session_id.clone(),
            adapter_key: request.adapter_key.clone(),
            provider: request.provider.clone(),
            account_profile: request.account_profile.clone(),
            model: request.model.clone(),
            variant: request.variant.clone(),
            usage_tokens_total: None,
            state: ProviderRunState::Starting,
            process_label: launch_result.process_label,
            pty_target: launch_result.pty_target,
            pty_program: launch_result.pty_program,
            pty_args: launch_result.pty_args,
            working_directory: launch_result.working_directory,
            structured_endpoint: launch_result.structured_endpoint,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    pub fn adapter_key(&self) -> &str {
        &self.adapter_key
    }
    pub fn provider(&self) -> &str {
        &self.provider
    }
    pub fn account_profile(&self) -> &str {
        &self.account_profile
    }
    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn variant(&self) -> Option<&str> {
        self.variant.as_deref()
    }

    pub fn set_model(&mut self, model: impl Into<String>) {
        self.model = model.into();
    }

    pub fn set_variant(&mut self, variant: Option<String>) {
        self.variant = variant.and_then(|value| {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_string())
        });
    }

    pub fn usage_tokens_total(&self) -> Option<u64> {
        self.usage_tokens_total
    }

    pub fn set_usage_tokens_total(&mut self, usage_tokens_total: Option<u64>) {
        self.usage_tokens_total = usage_tokens_total;
    }

    pub fn state(&self) -> ProviderRunState {
        self.state
    }
    pub fn process_label(&self) -> &str {
        &self.process_label
    }
    pub fn pty_target(&self) -> Option<&str> {
        self.pty_target.as_deref()
    }
    pub fn pty_program(&self) -> &str {
        &self.pty_program
    }
    pub fn pty_args(&self) -> &[String] {
        &self.pty_args
    }
    pub fn working_directory(&self) -> Option<&PathBuf> {
        self.working_directory.as_ref()
    }
    pub fn structured_endpoint(&self) -> Option<&str> {
        self.structured_endpoint.as_deref()
    }

    pub fn mark_running(&mut self) {
        self.state = ProviderRunState::Running;
    }

    pub fn mark_parked(&mut self) {
        self.state = ProviderRunState::Parked;
    }

    pub fn mark_ended(&mut self) {
        self.state = ProviderRunState::Ended;
    }
}
