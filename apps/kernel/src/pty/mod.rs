mod input_backlog;
mod manager;

#[cfg(all(test, target_os = "linux"))]
mod provider_restart_tests;
#[cfg(all(test, target_os = "linux"))]
mod sandbox_lifetime_tests;

pub use manager::{PtyAgent, PtyManager, PtyOutputChunk, PtyProcessState, PtySpawnRequest};
pub(crate) use manager::{PtyInputWriter, PtyOutputSignal};
