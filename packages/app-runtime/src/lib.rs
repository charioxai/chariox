//! Services used by the kernel's App supervisor, never by untrusted App code.
//!
//! Transport identity comes from the supervisor's installation and generation.
//! A successful handshake is not proof of OS confinement. The launcher must
//! establish confinement before executing any App code.

pub mod installation;
pub mod wire;
mod wire_json;
