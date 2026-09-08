//! Services used by the kernel's App supervisor, never by untrusted App code.
//!
//! Transport identity comes from the supervisor's installation and generation.
//! A successful handshake is not proof of OS confinement. The launcher must
//! establish confinement before executing any App code.

#[cfg(all(feature = "test-fixtures", not(debug_assertions)))]
compile_error!("test-fixtures is test-only and must not be enabled in production release builds");

pub mod app_catalog;
pub mod app_outbox;
pub mod installation;
pub mod managed_state;
pub mod publisher_trust;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod package_upload;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod private_fs;
#[cfg(unix)]
pub mod release_store;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
pub mod runtime_enrollment;
pub mod wire;
pub mod worker_peer;
pub mod worker_readiness;
mod wire_json;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
pub mod worker_process;
