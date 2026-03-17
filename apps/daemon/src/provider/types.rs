use std::fmt;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchProviderRequest {
    pub session_id: String,
    pub adapter_key: String,
    pub provider: String,
    pub account_profile: String,
    pub model: String,
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
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderLaunchResult {
    pub process_label: String,
    pub pty_target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeProviderRun {
    id: String,
    session_id: String,
    adapter_key: String,
    provider: String,
    account_profile: String,
    model: String,
    state: ProviderRunState,
    process_label: String,
    pty_target: Option<String>,
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
            state: ProviderRunState::Starting,
            process_label: launch_result.process_label,
            pty_target: launch_result.pty_target,
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
    pub fn state(&self) -> ProviderRunState {
        self.state
    }
    pub fn process_label(&self) -> &str {
        &self.process_label
    }
    pub fn pty_target(&self) -> Option<&str> {
        self.pty_target.as_deref()
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
