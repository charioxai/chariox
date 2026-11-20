//! Witness put in each provider launch and inherited by its tools.

pub(crate) const OWNER_ENV: &str = "CHARIOX_PROVIDER_OWNER_INSTANCE";

pub(crate) fn stamp(command: &mut std::process::Command) {
    match current_marker() {
        Some(marker) => command.env(OWNER_ENV, marker),
        None => command.env_remove(OWNER_ENV),
    };
}

#[cfg(target_os = "linux")]
pub(crate) fn current_marker() -> Option<String> {
    marker_for_pid(std::process::id())
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn current_marker() -> Option<String> {
    None
}

/// Match the actual kernel home across restarts, including when another
/// mount namespace presents a different copy at the same path.
#[cfg(target_os = "linux")]
pub(crate) fn home_identity() -> Option<String> {
    use std::os::unix::fs::MetadataExt as _;
    let home = std::fs::metadata(crate::credential::chariox_home()?).ok()?;
    Some(format!("{:x}-{:x}", home.dev(), home.ino()))
}

#[cfg(target_os = "linux")]
pub(crate) fn boot_id() -> Option<&'static str> {
    static BOOT: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    BOOT.get_or_init(|| {
        let id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").ok()?;
        Some(id.trim().to_owned())
    })
    .as_deref()
}

#[cfg(target_os = "linux")]
pub(crate) fn marker_for_pid(pid: u32) -> Option<String> {
    let start = crate::runtime::owned_process_signals::linux_start(pid).ok()??;
    let namespace = std::fs::read_link(format!("/proc/{pid}/ns/pid")).ok()?;
    Some(format!(
        "{pid}:{start}:{}:{}:{}",
        boot_id()?,
        namespace.to_str()?,
        home_identity()?
    ))
}
