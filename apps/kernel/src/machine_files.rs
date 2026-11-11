//! Files on the machines of one owner. A machine's files are listed, read in pages and copied
//! between machines by the kernel: the destination pulls from the source over the relay, so a
//! client or an agent only names the two ends. The kernel's own state and Vault are never part
//! of it.
mod jobs;
mod place;
mod source;
mod state_guard;

#[cfg(test)]
mod tests;

pub(crate) use jobs::TransferJobs;
pub(crate) use place::Machine;
pub(crate) use source::{list_path, serve, FileSource, PeerFiles, PeerLink};
pub(crate) use state_guard::StateGuard;
