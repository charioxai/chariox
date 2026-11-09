pub(crate) mod cloud_completion;
pub(crate) mod credential_free;
pub mod development;
pub(crate) mod empty;
pub(crate) mod event_connections;
pub mod kernel;
pub(crate) mod limits;
pub(crate) mod outbound;
pub(crate) mod outbound_service;
pub(crate) mod owner_authority;
pub mod owner_managed;
pub mod package;
pub(crate) mod portable_path;
pub(crate) mod scm;
pub(crate) mod target_vault;
pub(crate) mod transfer;

pub(crate) const MANAGED_CONTEXT_SOURCE_PROTOCOL_VERSION: u32 = 1;
