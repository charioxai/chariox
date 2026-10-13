//! MP-08 / MP-10 / MP-11: owned OS DNS resolution, shared by health and MCP adapters.
use std::path::PathBuf;
use std::time::Duration;
use zeroize::Zeroizing;

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
    #[cfg(target_os = "linux")]
    let name = "getent";
    #[cfg(target_os = "macos")]
    let name = "dscacheutil";
    #[cfg(windows)]
    let name = "powershell.exe";
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    let name = "getent";

    {
        let path = crate::project_environment::check_inventory::executable(name)
            .ok_or("resolver_unavailable")?;
        if roots.iter().any(|root| path.starts_with(root)) {
            return Err("resolver_unavailable");
        }
        let mut command = std::process::Command::new(path);
        command
            .current_dir("/")
            .env_clear()
            .env("LANG", "C")
            .env("PATH", "/usr/bin:/bin");
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
