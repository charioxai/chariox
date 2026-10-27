//! Public process metadata for authenticated MP-10 product-route observation.

pub(crate) fn current() -> Option<crate::local::KernelRuntimeProcessIdentity> {
    #[cfg(target_os = "linux")]
    {
        let pid = std::process::id();
        let boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").ok()?;
        let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
        parse(pid, &boot_id, &stat)
    }
    #[cfg(not(target_os = "linux"))]
    None
}

#[cfg(any(target_os = "linux", test))]
fn parse(
    pid: u32,
    boot_id: &str,
    stat: &str,
) -> Option<crate::local::KernelRuntimeProcessIdentity> {
    let boot_id = boot_id.trim().to_ascii_lowercase();
    if pid == 0
        || boot_id.len() != 36
        || !boot_id.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
        || !stat.starts_with(&format!("{pid} ("))
    {
        return None;
    }
    let close = stat.rfind(')')?;
    let fields = stat[close + 1..].split_whitespace().collect::<Vec<_>>();
    if matches!(*fields.first()?, "Z" | "X") {
        return None;
    }
    let ticks = *fields.get(19)?;
    if ticks.is_empty() || !ticks.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(crate::local::KernelRuntimeProcessIdentity {
        pid,
        linux_boot_id: boot_id,
        start_time_ticks: ticks.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const BOOT: &str = "b4a8b0e7-0f5b-4fd8-bcd9-ccc1e8b3c5ac";
    fn stat(pid: u32, ticks: &str, state: &str) -> String {
        format!(
            "{pid} (kernel (named) process) {state} 1 {} {ticks}\n",
            vec!["0"; 17].join(" ")
        )
    }
    #[test]
    fn parses_native_process_metadata_with_spaces_and_parentheses() {
        let identity = parse(42, BOOT, &stat(42, "7712345", "S")).unwrap();
        assert_eq!(identity.pid, 42);
        assert_eq!(identity.linux_boot_id, BOOT);
        assert_eq!(identity.start_time_ticks, "7712345");
    }
    #[test]
    fn rejects_foreign_or_malformed_process_metadata() {
        assert!(parse(42, BOOT, &stat(43, "7712345", "S")).is_none());
        assert!(parse(42, "not-a-boot-id", &stat(42, "7712345", "S")).is_none());
        assert!(parse(42, BOOT, &stat(42, "invalid", "S")).is_none());
        assert!(parse(42, BOOT, &stat(42, "7712345", "Z")).is_none());
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn current_identity_is_this_native_linux_process() {
        let identity = current().expect("native Linux process metadata");
        assert_eq!(identity.pid, std::process::id());
        assert_eq!(identity, current().unwrap());
    }
}
