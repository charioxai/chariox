//! Services used by the kernel's App supervisor, never by untrusted App code.
//!
//! Transport identity comes from the supervisor's installation and generation.
//! A successful handshake is not proof of OS confinement. The launcher must
//! establish confinement before executing any App code.

/// Production binary entry points evaluate this in a const context. Checking
/// there permits release-mode tests to use the shared library's fixture feature
/// without allowing it into the executables built by release/packaging scripts.
#[doc(hidden)]
pub const fn assert_production_build() {
    #[cfg(feature = "test-fixtures")]
    panic!("test-fixtures is test-only and must not be enabled in production release builds");
}

pub mod app_catalog;
pub mod app_inbox;
pub mod app_outbox;
pub mod installation;
pub mod managed_state;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod package_upload;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod private_fs;
pub mod publisher_trust;
#[cfg(unix)]
pub mod release_store;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
pub mod runtime_enrollment;
#[cfg(all(test, target_os = "linux"))]
mod storage_drill_fixture;
pub mod wire;
mod wire_json;
pub mod worker_peer;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
pub mod worker_process;
pub mod worker_readiness;
