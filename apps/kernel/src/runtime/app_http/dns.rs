//! Numeric-only destinations after bounded asynchronous DNS. The per-lookup
//! driver set is explicitly aborted AND joined before resolve returns.
//!
//! Each attempt asks one name server. Hickory's own pool gives all servers a
//! single timeout and stops at an error code (REFUSED, NOTIMP, SERVFAIL), so
//! one bad server would fail the lookup; here it gives way to the next.
mod tasks;

use super::{policy::ApprovedTarget, HttpError, Result};
use hickory_resolver::{
    config::{LookupIpStrategy, ResolveHosts, ResolverConfig, ResolverOpts},
    lookup_ip::LookupIp,
    proto::rr::Name,
    Resolver,
};
use std::{net::SocketAddr, time::Duration};
use tokio::{sync::watch, time::Instant};

/// One attempt at one name server; Hickory retransmits UDP within it. A server
/// that drops the query, or answers an error code, gives way to the next.
const PER_SERVER: Duration = Duration::from_secs(2);
/// Every name server is asked at most this many times per lookup, in order.
const ROUNDS: usize = 2;
/// The whole lookup, across servers and attempts: well inside the request's
/// own 30-second deadline for connecting and receiving headers.
const BUDGET: Duration = Duration::from_secs(6);

#[cfg(test)]
pub(super) mod test_server;
#[cfg(test)]
mod tests;

/// The absolute DNS name the resolver looks up. A host that is not a DNS
/// name (a label starting with a hyphen, say) is an invalid request, not a
/// network failure; the policy refuses it before a stream exists.
pub(super) fn absolute_name(host: &str) -> Result<Name> {
    Name::from_utf8(format!("{}.", host.trim_end_matches('.'))).map_err(|_| HttpError::Invalid)
}

pub(super) struct DnsConfig {
    /// One configuration per name server, in the host's order.
    servers: Vec<ResolverConfig>,
    options: ResolverOpts,
}

/// What one server's attempt settled.
enum Answer {
    Addresses(LookupIp),
    /// The name does not exist, from a server whose negative answers count.
    Missing,
    /// No usable answer: the next server is asked.
    Next,
}

impl DnsConfig {
    /// Startup configuration only, from the host's trusted OS configuration.
    /// No public resolver fallback, App-provided resolver, search suffix, NSS,
    /// multicast DNS, or macOS per-domain scoped resolver is synthesized.
    pub(super) fn system() -> Result<Self> {
        let (config, options) =
            hickory_resolver::system_conf::read_system_conf().map_err(|_| HttpError::Network)?;
        Self::bounded(config, options)
    }

    /// A resolver fixed to one in-process test server, with the same bounds.
    #[cfg(test)]
    pub(super) fn fixture(server: SocketAddr) -> Self {
        Self::fixture_servers(&[server])
    }

    /// In-process test servers, asked in this order, with the same bounds.
    #[cfg(test)]
    pub(super) fn fixture_servers(servers: &[SocketAddr]) -> Self {
        use hickory_resolver::config::{ConnectionConfig, NameServerConfig};
        let config = ResolverConfig::from_name_servers(
            servers
                .iter()
                .map(|server| {
                    let mut udp = ConnectionConfig::udp();
                    udp.port = server.port();
                    NameServerConfig::new(server.ip(), true, vec![udp])
                })
                .collect(),
        );
        Self::bounded(config, ResolverOpts::default()).expect("test name servers")
    }

    fn bounded(config: ResolverConfig, mut options: ResolverOpts) -> Result<Self> {
        if config.name_servers().is_empty() || config.name_servers().len() > 8 {
            return Err(HttpError::Network);
        }
        // Hickory's timeout bounds one server's attempt; this module retries.
        options.timeout = PER_SERVER;
        options.attempts = 0;
        options.num_concurrent_reqs = 1;
        options.max_active_requests = 2;
        options.cache_size = 0;
        options.preserve_intermediates = false;
        options.use_hosts_file = ResolveHosts::Never;
        options.ip_strategy = LookupIpStrategy::Ipv4AndIpv6;
        let servers = config
            .name_servers()
            .iter()
            .map(|server| ResolverConfig::from_name_servers(vec![server.clone()]))
            .collect();
        Ok(Self { servers, options })
    }

    pub(super) async fn resolve(
        &self,
        target: &ApprovedTarget,
        deadline: Instant,
        mut cancelled: watch::Receiver<bool>,
        lease: super::limits::LifetimeLease,
    ) -> Result<Vec<SocketAddr>> {
        let budget = Instant::now() + BUDGET;
        if super::stopped(&cancelled) {
            return Err(HttpError::Cancelled);
        }
        if deadline <= Instant::now() {
            return Err(HttpError::Deadline);
        }
        let host = target.url().host().ok_or(HttpError::Invalid)?;
        let name = match host {
            url::Host::Ipv4(address) => {
                return target.validate_addresses([SocketAddr::new(address.into(), target.port())])
            }
            url::Host::Ipv6(address) => {
                return target.validate_addresses([SocketAddr::new(address.into(), target.port())])
            }
            url::Host::Domain(name) => absolute_name(name)?,
        };
        let attempts = self.servers.len() * ROUNDS;
        for server in self.servers.iter().cycle().take(attempts) {
            let now = Instant::now();
            if now >= budget {
                break;
            }
            let until = budget.min(now + PER_SERVER);
            match self
                .ask(server, &name, until, deadline, &mut cancelled, &lease)
                .await?
            {
                Answer::Addresses(answer) => {
                    return target.validate_addresses(
                        answer.iter().map(|ip| SocketAddr::new(ip, target.port())),
                    )
                }
                Answer::Missing => return Err(HttpError::Network),
                Answer::Next => {}
            }
        }
        Err(HttpError::Network)
    }

    /// One server, until `until`. Its drivers are aborted and joined before
    /// this returns, so no attempt outlives its turn.
    async fn ask(
        &self,
        server: &ResolverConfig,
        name: &Name,
        until: Instant,
        deadline: Instant,
        cancelled: &mut watch::Receiver<bool>,
        lease: &super::limits::LifetimeLease,
    ) -> Result<Answer> {
        let trusted = server
            .name_servers()
            .iter()
            .all(|server| server.trust_negative_responses);
        let tasks = tasks::TaskOwner::new(lease.clone());
        let mut builder = Resolver::builder_with_config(server.clone(), tasks.provider());
        *builder.options_mut() = self.options.clone();
        let result = match builder.build() {
            Err(_) => Err(HttpError::Network),
            Ok(resolver) => {
                let result = tokio::select! {
                    biased;
                    _ = super::cancelled(cancelled) => Err(HttpError::Cancelled),
                    _ = tokio::time::sleep_until(deadline) => Err(HttpError::Deadline),
                    _ = tokio::time::sleep_until(until) => Ok(Answer::Next),
                    result = resolver.lookup_ip(name.clone()) => Ok(match result {
                        Ok(answer) if answer.iter().next().is_some() => Answer::Addresses(answer),
                        Err(error) if trusted && error.is_nx_domain() => Answer::Missing,
                        _ => Answer::Next,
                    }),
                };
                drop(resolver);
                result
            }
        };
        let overloaded = tasks.overloaded();
        // Do not wrap this cleanup in another timeout that would detach tasks.
        tasks.finish().await;
        if overloaded {
            Err(HttpError::Limit)
        } else {
            result
        }
    }
}
