//! Services used by the kernel's App supervisor, never by untrusted App code.
//!
//! Transport identity comes from the supervisor's installation and generation.
//! A successful handshake is not proof of OS confinement. The launcher must
//! establish confinement before executing any App code.

pub mod installation;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod package_upload;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod private_fs;
#[cfg(unix)]
pub mod release_store;
pub mod wire;
mod wire_json;
