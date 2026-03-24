mod opencode;
mod opencode_client;
mod registry;
mod service;
mod types;

pub use opencode::{opencode_catalog_endpoint, plan_opencode_launch, resolve_opencode_executable};
pub use opencode_client::{
    OpenCodeClient, OpenCodeEvent, OpenCodeEventSubscription, OpenCodeMessage,
    OpenCodeProviderCatalog, OpenCodeProviderInfo, OpenCodeProviderModel, OpenCodeSessionSnapshot,
};
pub use registry::{ProviderAdapter, ProviderRegistry};
pub use service::{OpenCodePollResult, ProviderProcessService};
pub use types::{
    LaunchProviderRequest, ProviderLaunchResult, ProviderRunState, RuntimeProviderRun,
};
