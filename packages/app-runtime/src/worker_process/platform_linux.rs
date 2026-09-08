//! Linux preparation composes enrolled runtime, verified release, helper-owned
//! storage/code views and delegated cgroups before the native launch handshake.
#[cfg(target_os = "linux")]
mod cgroup;
#[cfg(target_os = "linux")]
mod domain;
#[cfg(target_os = "linux")]
mod factory;
#[cfg(target_os = "linux")]
pub(super) use factory::prepare;
#[cfg(target_os = "linux")]
mod inspection;
#[cfg(target_os = "linux")]
mod plan;
mod policy;
#[cfg(test)]
mod tests;

use super::WorkerError;
type Result<T> = std::result::Result<T, WorkerError>;
const MEMORY_BYTES: u64 = 512 * 1024 * 1024;
const TASKS: u64 = 64;
// This is the initial installer/hosted validation baseline, not a statement
// that other distributions or architectures have already passed acceptance.
const MINIMUM_KERNEL: (u64, u64) = (6, 1);
