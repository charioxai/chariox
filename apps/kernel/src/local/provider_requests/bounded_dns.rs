//! MP-08 / MP-10 / MP-11: owned OS DNS resolution, shared by health and MCP adapters.
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use std::time::Instant;
use zeroize::Zeroizing;

// MP-08 / MP-10 / MP-11: public address results have a short bounded cache.
// Validate the selected resolver before reuse, including workspace roots.
type DnsCache = std::collections::BTreeMap<(String, PathBuf), (Instant, Vec<std::net::IpAddr>)>;
fn dns_cache() -> &'static Mutex<DnsCache> {
    static CACHE: OnceLock<Mutex<DnsCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(DnsCache::new()))
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
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
    {
        return Err("invalid_probe_host");
    }
    // The OS resolver is an owned bounded utility; blocking getaddrinfo must
    // not leave unbounded detached resolver threads after a Check timeout.
    // Resolver selection is an OS operation, including MCP calls without a
    // Project trust context. Never select a workspace helper through PATH.
    #[cfg(target_os = "macos")]
    let path = PathBuf::from("/usr/bin/dscacheutil");
    #[cfg(all(unix, not(target_os = "macos")))]
    let path = PathBuf::from("/usr/bin/getent");
    #[cfg(windows)]
    let path = PathBuf::from(std::env::var_os("SystemRoot").ok_or("resolver_unavailable")?)
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");

    {
        // Vets the complete interpreter chain while preserving symlink argv[0].
        let mut command = crate::project_environment::check_executable::command(&path, roots)
            .map_err(|_| "resolver_unavailable")?;
        let canonical = path.canonicalize().map_err(|_| "resolver_unavailable")?;
        let key = (host.to_ascii_lowercase(), canonical);
        if let Some(addresses) = dns_cache().lock().ok().and_then(|cache| {
            cache
                .get(&key)
                .filter(|(at, _)| at.elapsed() < Duration::from_secs(30))
                .map(|(_, addresses)| addresses.clone())
        }) {
            return Ok(addresses);
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        command.args(["ahosts", host]);
        #[cfg(windows)]
        {
            if let Some(root) = std::env::var_os("SystemRoot") {
                command.env("SystemRoot", root);
            }
            // Host input is separately bound and restricted above; no profile or startup script.
            command.args(["-NoProfile", "-NonInteractive", "-Command", "& { param([string]$n) [System.Net.Dns]::GetHostAddresses($n) | ForEach-Object { $_.IPAddressToString } }", host]);
        }
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
            #[cfg(not(target_os = "macos"))]
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
        if !addresses.is_empty() {
            if let Ok(mut cache) = dns_cache().lock() {
                cache.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(30));
                if cache.len() >= 64 {
                    cache.clear();
                }
                cache.insert(key, (Instant::now(), addresses.clone()));
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
