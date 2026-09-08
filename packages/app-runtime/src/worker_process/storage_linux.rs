//! Root-owned persistent App storage. The privileged helper and client use this
//! production core; actual mount/formatter acceptance runs on a dedicated host.
mod cgroup;
mod client;
mod code_model;
mod code_mounts;
mod code_sources;
mod files;
mod formatter;
#[cfg(test)]
mod hosted;
mod loop_device;
mod model;
mod mount;
mod provision;
mod server;
mod store;
mod wire;

pub(super) use client::Lease;

pub(super) fn cgroup_path() -> Result<std::path::PathBuf> {
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };
    if uid == 0 {
        return Err(Error::Identity);
    }
    let config = server::configuration()?;
    let owner = config
        .owners
        .into_iter()
        .find(|owner| owner.uid == uid && owner.gid == gid)
        .ok_or(Error::Identity)?;
    Ok(owner.cgroup_root.into())
}

use std::result::Result as StdResult;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(super) enum Error {
    #[error("app_storage_invalid")]
    Invalid,
    #[error("app_storage_identity")]
    Identity,
    #[error("app_storage_capacity")]
    Capacity,
    #[error("app_storage_busy")]
    Busy,
    #[error("app_storage_io")]
    Io,
    #[error("app_storage_recovery_required")]
    RecoveryRequired,
}
impl From<std::io::Error> for Error {
    fn from(_: std::io::Error) -> Self {
        Self::Io
    }
}
impl From<crate::private_fs::FsError> for Error {
    fn from(_: crate::private_fs::FsError) -> Self {
        Self::Identity
    }
}
type Result<T> = StdResult<T, Error>;

const DATA_BYTES: u64 = 512 * 1024 * 1024;
const TMP_BYTES: u64 = 64 * 1024 * 1024;
const MAX_INSTALLATIONS: usize = 64;
const MAX_RESERVED_BYTES: u64 = 32 * 1024 * 1024 * 1024;
const HOST_RESERVE_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const ROOT: &str = "/var/lib/chariox-app-storage";
const CONFIG: &str = "/etc/chariox/app-storage.json";
const SOCKET_ROOT: &str = "/run/chariox-app-storage";

pub(super) fn run(arguments: Vec<String>) -> Result<()> {
    server::run(arguments)
}
