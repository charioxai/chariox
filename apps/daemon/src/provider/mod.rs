mod opencode;
mod opencode_client;
mod registry;
mod service;
mod types;

pub use opencode::{plan_opencode_launch, resolve_opencode_executable};
pub use opencode_client::{OpenCodeClient, OpenCodeMessage};
pub use registry::{ProviderAdapter, ProviderRegistry};
pub use service::ProviderProcessService;
pub use types::{
    LaunchProviderRequest, ProviderLaunchResult, ProviderRunState, RuntimeProviderRun,
};
