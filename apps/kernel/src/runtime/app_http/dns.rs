//! Numeric-only destinations after bounded asynchronous DNS. The per-lookup
//! driver set is explicitly aborted AND joined before resolve returns.
mod tasks;

use super::{policy::ApprovedTarget, HttpError, Result};
use hickory_resolver::{
    config::{LookupIpStrategy, ResolveHosts, ResolverConfig, ResolverOpts},
    Resolver,
};
use std::{net::SocketAddr, time::Duration};
use tokio::{sync::watch, time::Instant};

pub(super) struct DnsConfig {
    config: ResolverConfig,
    options: ResolverOpts,
}

impl DnsConfig {
    /// Startup configuration only, from the host's trusted OS configuration.
    /// No public resolver fallback, App-provided resolver, search suffix, NSS,
    /// multicast DNS, or macOS per-domain scoped resolver is synthesized.
    pub(super) fn system() -> Result<Self> {
        let (config, mut options) =
            hickory_resolver::system_conf::read_system_conf().map_err(|_| HttpError::Network)?;
        if config.name_servers().is_empty() || config.name_servers().len() > 8 {
            return Err(HttpError::Network);
        }
        options.timeout = Duration::from_secs(3);
        options.attempts = 1;
        options.num_concurrent_reqs = 1;
        options.max_active_requests = 2;
        options.cache_size = 0;
        options.preserve_intermediates = false;
        options.use_hosts_file = ResolveHosts::Never;
        options.ip_strategy = LookupIpStrategy::Ipv4AndIpv6;
        Ok(Self { config, options })
    }

    pub(super) async fn resolve(
        &self,
        target: &ApprovedTarget,
        deadline: Instant,
        mut cancelled: watch::Receiver<bool>,
        lease: super::limits::LifetimeLease,
    ) -> Result<Vec<SocketAddr>> {
        let deadline = deadline.min(Instant::now() + Duration::from_secs(10));
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
            url::Host::Domain(name) => format!("{}.", name.trim_end_matches('.')),
        };
        let tasks = tasks::TaskOwner::new(lease);
        let mut builder = Resolver::builder_with_config(self.config.clone(), tasks.provider());
        *builder.options_mut() = self.options.clone();
        let resolver = builder.build().map_err(|_| HttpError::Network)?;
        let result = tokio::select! {
            biased;
            _ = super::cancelled(&mut cancelled) => Err(HttpError::Cancelled),
            _ = tokio::time::sleep_until(deadline) => Err(HttpError::Deadline),
            result = resolver.lookup_ip(name) => match result {
                Ok(answer) => target.validate_addresses(answer.iter().map(|ip| SocketAddr::new(ip, target.port()))),
                Err(_) => Err(HttpError::Network),
            }
        };
        drop(resolver);
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
