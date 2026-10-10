//! MP-08 / MP-10 / MP-11: bounded named TCP, HTTP and inventory health probes.
use super::check_inventory::{executable, inventory_args};
use super::*;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use zeroize::Zeroizing;
pub(super) fn health(
    probe: &EnvironmentProbe,
    expectation: &Option<EnvironmentHealthExpectation>,
    roots: &[PathBuf],
) -> Result<bool, &'static str> {
    match probe {
        EnvironmentProbe::Tcp {
            host,
            port,
            timeout_ms,
        } => {
            let timeout = Duration::from_millis((*timeout_ms).clamp(1, 5000) as u64);
            let began = Instant::now();
            let addresses = resolve_host(host, timeout, roots)?;
            for ip in addresses {
                let remaining = timeout.saturating_sub(began.elapsed());
                if remaining.is_zero() {
                    break;
                }
                if std::net::TcpStream::connect_timeout(
                    &std::net::SocketAddr::new(ip, *port),
                    remaining,
                )
                .is_ok()
                {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        EnvironmentProbe::Http {
            url,
            expected_status,
            timeout_ms,
        } => {
            let parsed = url::Url::parse(url).map_err(|_| "invalid_probe")?;
            if !matches!(parsed.scheme(), "http" | "https")
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.query().is_some()
                || parsed.fragment().is_some()
            {
                return Err("invalid_probe");
            }
            let expected = match expectation {
                Some(EnvironmentHealthExpectation::HttpStatus { status }) => *status,
                _ => *expected_status,
            };
            let agent = ureq::AgentBuilder::new()
                .redirects(0)
                .timeout(Duration::from_millis((*timeout_ms).clamp(1, 5000) as u64))
                .build();
            match agent.get(url).call() {
                Ok(r) => Ok(r.status() == expected),
                Err(ureq::Error::Status(s, _)) => Ok(s == expected),
                Err(_) => Ok(false),
            }
        }
        EnvironmentProbe::Executable {
            name,
            arguments,
            timeout_ms,
        } => {
            let Some(args) = inventory_args(name) else {
                return Err("worker_required");
            };
            if arguments.iter().map(String::as_str).collect::<Vec<_>>() != args {
                return Err("worker_required");
            }
            let Some(p) = executable(name) else {
                return Ok(false);
            };
            if roots.iter().any(|r| p.starts_with(r)) {
                return Err("worker_required");
            }
            let mut c = std::process::Command::new(p);
            c.args(args)
                .current_dir("/")
                .env_clear()
                .env("LANG", "C")
                .env("PATH", "/usr/local/bin:/usr/bin:/bin");
            let exit = crate::local::provider_requests::capture_inventory_probe(
                c,
                Duration::from_millis((*timeout_ms).clamp(1, 5000) as u64),
            )
            .map_err(|_| "probe_failed")?
            .status
            .code();
            Ok(exit
                == Some(match expectation {
                    Some(EnvironmentHealthExpectation::CommandExit { exit_code }) => *exit_code,
                    _ => 0,
                }))
        }
    }
}

fn resolve_host(
    host: &str,
    timeout: Duration,
    roots: &[PathBuf],
) -> Result<Vec<std::net::IpAddr>, &'static str> {
    if let Ok(ip) = host.parse() {
        return Ok(vec![ip]);
    }
    if host == "localhost" {
        return Ok(vec![std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)]);
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
        let Ok(output) = crate::local::provider_requests::capture_inventory_probe(
            command,
            timeout.min(Duration::from_secs(3)),
        ) else {
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
