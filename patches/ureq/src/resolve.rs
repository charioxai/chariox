use std::fmt;
use std::io;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::Arc;
use std::time::Instant;

/// A custom resolver to override the default DNS behavior.
pub trait Resolver: Send + Sync {
    fn resolve(&self, netloc: &str) -> io::Result<Vec<SocketAddr>>;

    /// Whether an address family is still being looked up. A pending answer
    /// shares the connect budget with the addresses already returned.
    fn has_pending(&self, _netloc: &str, _known: &[SocketAddr]) -> bool {
        false
    }

    /// Wait for untried answers after connection attempts fail, bounded by
    /// the original connect deadline. No HTTP request has been sent yet.
    fn resolve_next(
        &self,
        _netloc: &str,
        _tried: &[SocketAddr],
        _deadline: Instant,
    ) -> io::Result<Vec<SocketAddr>> {
        Ok(Vec::new())
    }
}

#[derive(Debug)]
pub(crate) struct StdResolver;

impl Resolver for StdResolver {
    fn resolve(&self, netloc: &str) -> io::Result<Vec<SocketAddr>> {
        ToSocketAddrs::to_socket_addrs(netloc).map(|iter| iter.collect())
    }
}

impl<F> Resolver for F
where
    F: Fn(&str) -> io::Result<Vec<SocketAddr>>,
    F: Send + Sync,
{
    fn resolve(&self, netloc: &str) -> io::Result<Vec<SocketAddr>> {
        self(netloc)
    }
}

#[derive(Clone)]
pub(crate) struct ArcResolver(Arc<dyn Resolver>);

impl<R> From<R> for ArcResolver
where
    R: Resolver + 'static,
{
    fn from(r: R) -> Self {
        Self(Arc::new(r))
    }
}

impl fmt::Debug for ArcResolver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ArcResolver(...)")
    }
}

impl std::ops::Deref for ArcResolver {
    type Target = dyn Resolver;

    fn deref(&self) -> &Self::Target {
        self.0.as_ref()
    }
}
