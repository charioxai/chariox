//! Build-time WASI compatibility. This never extends kernel admission.

mod compat;
pub use compat::prepare_component;
