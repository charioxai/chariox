use super::{LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun};

pub trait ProviderAdapter: Send + Sync {
    fn key(&self) -> &'static str;
    fn launch(&self, request: &LaunchProviderRequest) -> ProviderLaunchResult;
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
        1
    }

    pub fn resolve(&self, key: &str) -> Option<&'static dyn ProviderAdapter> {
        match key {
            DevStubAdapter::KEY => Some(&DEV_STUB_ADAPTER),
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

    fn launch(&self, request: &LaunchProviderRequest) -> ProviderLaunchResult {
        ProviderLaunchResult {
            process_label: format!(
                "dev-stub:{}:{}:{}",
                request.provider, request.account_profile, request.model
            ),
            pty_target: Some(format!("stub-pty:{}", request.session_id)),
        }
    }

    fn park(&self, _run: &RuntimeProviderRun) {}

    fn resume(&self, _run: &RuntimeProviderRun) {}

    fn terminate(&self, _run: &RuntimeProviderRun) {}
}
