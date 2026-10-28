//! MP-08 / MP-10 / MP-11: bounded named TCP, HTTP and inventory health probes.
use super::check_inventory::{executable, inventory_args};
use super::*;
use crate::local::provider_requests::resolve_bounded_host as resolve_host;
use std::path::PathBuf;
use std::time::{Duration, Instant};
pub(super) fn health(
    probe: &EnvironmentProbe,
    expectation: &Option<EnvironmentHealthExpectation>,
    roots: &[PathBuf],
) -> Result<bool, &'static str> {
    health_until(
        probe,
        expectation,
        roots,
        Instant::now() + Duration::from_secs(5),
    )
}
pub(super) fn health_until(
    probe: &EnvironmentProbe,
    expectation: &Option<EnvironmentHealthExpectation>,
    roots: &[PathBuf],
    deadline: Instant,
) -> Result<bool, &'static str> {
    match probe {
        EnvironmentProbe::Tcp {
            host,
            port,
            timeout_ms,
        } => {
            let timeout = super::check_inventory::remaining(
                deadline,
                Duration::from_millis((*timeout_ms).clamp(1, 5000) as u64),
            )?;
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
            let timeout = super::check_inventory::remaining(
                deadline,
                Duration::from_millis((*timeout_ms).clamp(1, 5000) as u64),
            )?;
            let began = Instant::now();
            let addresses = match parsed.host().ok_or("invalid_probe_host")? {
                url::Host::Domain(host) => resolve_host(host, timeout, roots)?,
                url::Host::Ipv4(ip) => vec![ip.into()],
                url::Host::Ipv6(ip) => vec![ip.into()],
            };
            let port = parsed.port_or_known_default().ok_or("invalid_probe")?;
            let addresses: Vec<_> = addresses
                .into_iter()
                .map(|ip| std::net::SocketAddr::new(ip, port))
                .collect();
            let remaining = timeout.saturating_sub(began.elapsed());
            if remaining.is_zero() || addresses.is_empty() {
                return Ok(false);
            }
            // Resolve before ureq, preserving the URL's Host header and TLS
            // server name. Redirects and environment proxies cannot escape to
            // a second unbounded hostname lookup.
            let agent = ureq::AgentBuilder::new()
                .try_proxy_from_env(false)
                .resolver(move |_: &str| Ok(addresses.clone()))
                .redirects(0)
                .timeout(remaining)
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
            if p.canonicalize()
                .is_ok_and(|canonical| roots.iter().any(|r| canonical.starts_with(r)))
            {
                return Err("worker_required");
            }
            let mut c = super::check_executable::command(&p, roots)?;
            c.args(args);
            let exit = crate::local::provider_requests::capture_inventory_probe(
                c,
                super::check_inventory::remaining(
                    deadline,
                    Duration::from_millis((*timeout_ms).clamp(1, 5000) as u64),
                )?,
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
