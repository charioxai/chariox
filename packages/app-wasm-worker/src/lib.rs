//! One async component instance per installation, on the kernel App wire.

pub mod containment;
pub mod engine;

mod attribution;
mod bindings;
mod broker;
mod files;
mod host;
mod http;
pub mod ipc;
pub mod limits;
pub mod scope;
mod streams;
pub mod worker;
pub use host::Host;
pub mod cli;
