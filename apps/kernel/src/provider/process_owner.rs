//! Witness put in each provider launch and inherited by its tools.

pub(crate) const OWNER_ENV: &str = "CHARIOX_PROVIDER_OWNER_INSTANCE";

pub(crate) fn stamp(command: &mut std::process::Command) {
    match current_marker() {
        Some(marker) => command.env(OWNER_ENV, marker),
        None => command.env_remove(OWNER_ENV),
    };
}

#[cfg(target_os = "linux")]
pub(crate) fn current_marker() -> Option<&'static str> {
    static MARKER: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    MARKER
        .get_or_init(|| marker_for_pid(std::process::id()))
        .as_deref()
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn current_marker() -> Option<&'static str> {
    None
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
        "{pid}:{start}:{}:{}",
        boot_id()?,
        namespace.to_str()?
    ))
}
