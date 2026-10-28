//! MP-08 / MP-10 / MP-11: owned OS DNS resolution, shared by health and MCP adapters.
use std::path::PathBuf;
use std::time::Duration;
use zeroize::Zeroizing;
fn executable(name: &str) -> Option<PathBuf> {
    if name.contains(['/', '\\']) || name.is_empty() {
        return None;
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .filter(|p| p.is_absolute())
        .find_map(|p| {
            let p = p.join(name).canonicalize().ok()?;
            let m = p.metadata().ok()?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if m.permissions().mode() & 0o111 == 0 {
                    return None;
                }
            }
            m.is_file().then_some(p)
        })
}

pub(crate) fn resolve_bounded_host(
    host: &str,
    timeout: Duration,
    roots: &[PathBuf],
) -> Result<Vec<std::net::IpAddr>, &'static str> {
    if let Ok(ip) = host.parse() {
        return Ok(vec![ip]);
    }
    if host == "localhost" {
        return Ok(vec![
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
        ]);
    }
    if host.is_empty()
        || host.len() > 253
        || host.starts_with('-')
        || !host
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b".-".contains(&c))
    {
        return Err("invalid_probe_host");
    }
    // The OS resolver is an owned bounded utility; blocking getaddrinfo must
    // not leave unbounded detached resolver threads after a Check timeout.
    #[cfg(target_os = "linux")]
    let name = "getent";
    #[cfg(target_os = "macos")]
    let name = "dscacheutil";
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    return Err("resolver_unavailable");
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let path = executable(name).ok_or("resolver_unavailable")?;
        if roots.iter().any(|root| path.starts_with(root)) {
            return Err("resolver_unavailable");
        }
        let mut command = std::process::Command::new(path);
        command
            .current_dir("/")
            .env_clear()
            .env("LANG", "C")
            .env("PATH", "/usr/bin:/bin");
        #[cfg(target_os = "linux")]
        command.args(["ahosts", host]);
        #[cfg(target_os = "macos")]
        command.args(["-q", "host", "-a", "name", host]);
        let Ok(output) =
            super::capture_inventory_probe(command, timeout.min(Duration::from_secs(3)))
        else {
            return Ok(vec![]);
        };
        if !output.status.success() {
            return Ok(vec![]);
        }
        let bytes = Zeroizing::new(output.stdout);
        let text = Zeroizing::new(String::from_utf8_lossy(&bytes).into_owned());
        let mut addresses = vec![];
        for line in text.lines().take(64) {
            #[cfg(target_os = "linux")]
            let value = line.split_whitespace().next().unwrap_or("");
            #[cfg(target_os = "macos")]
            let value = line
                .strip_prefix("ip_address:")
                .or_else(|| line.strip_prefix("ipv6_address:"))
                .unwrap_or("")
                .trim();
            if let Ok(ip) = value.parse() {
                if !addresses.contains(&ip) {
                    addresses.push(ip);
                }
            }
        }
        Ok(addresses)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn localhost_keeps_both_loopback_families() {
        let addresses =
            super::resolve_bounded_host("localhost", std::time::Duration::from_millis(1), &[])
                .unwrap();
        assert!(addresses.contains(&std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)));
        assert!(addresses.contains(&std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)));
    }
}
