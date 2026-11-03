//! Outbound connections to dual-stack hosts.
//!
//! A host can publish AAAA and A records while its IPv6 path silently drops
//! traffic. The plain resolver order then spends most of a caller's budget on
//! unanswered IPv6 SYNs. Every kernel dial to a remote host goes through this
//! module: families alternate, and each attempt is short, so a working family
//! is reached in about one attempt time, as in curl and browsers.

use std::{
    collections::HashMap,
    io,
    net::SocketAddr,
    sync::Mutex,
    time::{Duration, Instant},
};

use tokio::{net::TcpStream, task::JoinSet, time::sleep};
use tokio_tungstenite::{
    client_async_tls_with_config,
    tungstenite::{client::IntoClientRequest, error::UrlError, handshake::client::Response, Error},
    MaybeTlsStream, WebSocketStream,
};

mod dns;

/// Budget for the connect phase of one blocking HTTP call. With several
/// addresses ureq gives the first one half of it.
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// Time one numeric address gets before the next one is tried.
pub(crate) const ADDRESS_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(1);
/// Delay before the next address is raced against attempts still in flight.
const RACE_STAGGER: Duration = Duration::from_millis(250);
/// Whole budget for a websocket TCP connect.
const SOCKET_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// How long the address that last connected to a host:port is tried first, so
/// only the first call over a dead path pays for it.
const ADDRESS_MEMORY: Duration = Duration::from_secs(10 * 60);

static CONNECTED_ADDRESSES: Mutex<Option<HashMap<String, (SocketAddr, Instant)>>> =
    Mutex::new(None);

pub(crate) type WebSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Alternates address families, keeping the resolver's order inside each
/// family and starting with the family it listed first.
pub(crate) fn interleave_families(addresses: Vec<SocketAddr>) -> Vec<SocketAddr> {
    let first_is_v6 = addresses.first().is_some_and(SocketAddr::is_ipv6);
    let (v6, v4): (Vec<_>, Vec<_>) = addresses.into_iter().partition(SocketAddr::is_ipv6);
    let (mut primary, mut secondary) = if first_is_v6 {
        (v6.into_iter(), v4.into_iter())
    } else {
        (v4.into_iter(), v6.into_iter())
    };
    let mut ordered = Vec::new();
    loop {
        match (primary.next(), secondary.next()) {
            (None, None) => return ordered,
            (first, second) => ordered.extend(first.into_iter().chain(second)),
        }
    }
}

fn remember_connected_address(netloc: String, address: SocketAddr) {
    let mut families = CONNECTED_ADDRESSES
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    families
        .get_or_insert_with(HashMap::new)
        .insert(netloc, (address, Instant::now()));
}

/// Interleaves families, starting with the address that last connected to
/// `netloc` (and then its family) while that memory is fresh.
fn order_addresses(netloc: &str, mut addresses: Vec<SocketAddr>) -> Vec<SocketAddr> {
    let remembered = CONNECTED_ADDRESSES
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
        .and_then(|families| families.get(netloc).copied())
        .filter(|(_, at)| at.elapsed() < ADDRESS_MEMORY);
    if let Some((connected, _)) = remembered {
        addresses.sort_by_key(|address| {
            (
                *address != connected,
                address.is_ipv6() != connected.is_ipv6(),
            )
        });
    }
    interleave_families(addresses)
}

fn request_netloc(url: &str) -> Option<String> {
    let url = url::Url::parse(url).ok()?;
    Some(format!(
        "{}:{}",
        url.host_str()?,
        url.port_or_known_default()?
    ))
}

#[allow(
    clippy::result_large_err,
    reason = "ureq's middleware signature fixes the error type"
)]
fn remember_address_of_response(
    request: ureq::Request,
    next: ureq::MiddlewareNext,
) -> Result<ureq::Response, ureq::Error> {
    let netloc = request_netloc(request.url());
    let result = next.handle(request);
    let connected = match &result {
        Ok(response) | Err(ureq::Error::Status(_, response)) => Some(response.remote_addr()),
        Err(ureq::Error::Transport(_)) => None,
    };
    if let (Some(netloc), Some(address)) = (netloc, connected) {
        remember_connected_address(netloc, address);
    }
    result
}

struct InterleavedResolver<R>(R);

impl<R> ureq::Resolver for InterleavedResolver<R>
where
    R: ureq::Resolver,
{
    fn resolve(&self, netloc: &str) -> io::Result<Vec<SocketAddr>> {
        self.0
            .resolve(netloc)
            .map(|addresses| order_addresses(netloc, addresses))
    }

    fn has_pending(&self, netloc: &str, known: &[SocketAddr]) -> bool {
        self.0.has_pending(netloc, known)
    }

    fn resolve_next(
        &self,
        netloc: &str,
        tried: &[SocketAddr],
        deadline: Instant,
    ) -> io::Result<Vec<SocketAddr>> {
        self.0
            .resolve_next(netloc, tried, deadline)
            .map(|addresses| order_addresses(netloc, addresses))
    }
}

impl ureq::Resolver for dns::Resolver {
    fn resolve(&self, netloc: &str) -> io::Result<Vec<SocketAddr>> {
        self.resolve(netloc)
    }
    fn has_pending(&self, netloc: &str, known: &[SocketAddr]) -> bool {
        self.has_pending(netloc, known)
    }
    fn resolve_next(
        &self,
        netloc: &str,
        tried: &[SocketAddr],
        deadline: Instant,
    ) -> io::Result<Vec<SocketAddr>> {
        self.resolve_next(netloc, tried, deadline)
    }
}

fn system_addresses(netloc: &str) -> io::Result<Vec<SocketAddr>> {
    dns::system().resolve(netloc)
}

/// Resolves `host:port` with both families looked up independently, for
/// callers that validate the addresses themselves.
pub(crate) fn resolve_addresses(netloc: &str) -> io::Result<Vec<SocketAddr>> {
    system_addresses(netloc)
}

/// The agent builder for every kernel HTTP call to a remote host.
pub(crate) fn agent_builder() -> ureq::AgentBuilder {
    agent_builder_with_resolver(dns::system().clone())
}

/// A default-configured agent for calls that set no other options.
pub(crate) fn agent() -> ureq::Agent {
    agent_builder().build()
}

pub(crate) fn agent_builder_with_resolver<R>(resolve: R) -> ureq::AgentBuilder
where
    R: ureq::Resolver + 'static,
{
    ureq::AgentBuilder::new()
        .resolver(InterleavedResolver(resolve))
        .middleware(remember_address_of_response)
        .timeout_connect(HTTP_CONNECT_TIMEOUT)
}

/// Opens a TCP socket to `host:port`, racing the resolved addresses with
/// alternating families. Slower attempts keep running while the next starts.
async fn connect_tcp_with_resolver(
    host: &str,
    port: u16,
    resolver: dns::Resolver,
) -> io::Result<TcpStream> {
    let netloc = format!("{host}:{port}");
    let lookup = netloc.clone();
    let initial_resolver = resolver.clone();
    let deadline = Instant::now() + SOCKET_CONNECT_TIMEOUT;
    let connect = async {
        let addresses = tokio::task::spawn_blocking(move || initial_resolver.resolve(&lookup))
            .await
            .map_err(io::Error::other)??;
        let mut known = addresses.clone();
        let mut races = JoinSet::new();
        let mut lookups = JoinSet::new();
        races.spawn(race(order_addresses(&netloc, addresses)));
        if resolver.has_pending(&netloc, &known) {
            start_pending_lookup(
                &mut lookups,
                resolver.clone(),
                netloc.clone(),
                known.clone(),
                deadline,
            );
        }
        let mut last_error = None;
        loop {
            if races.is_empty() && lookups.is_empty() {
                return Err(last_error.unwrap_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, "no address to connect to")
                }));
            }
            tokio::select! {
                Some(joined) = races.join_next(), if !races.is_empty() => match joined {
                    Ok(Ok(stream)) => {
                        if let Ok(address) = stream.peer_addr() { remember_connected_address(netloc, address); }
                        return Ok(stream);
                    }
                    Ok(Err(error)) => last_error = Some(error),
                    Err(error) => last_error = Some(io::Error::other(error)),
                },
                Some(joined) = lookups.join_next(), if !lookups.is_empty() => {
                    let addresses = joined.map_err(io::Error::other)??;
                    if !addresses.is_empty() {
                        known.extend(addresses.iter().copied());
                        races.spawn(race(order_addresses(&netloc, addresses)));
                    }
                    if Instant::now() < deadline && resolver.has_pending(&netloc, &known) {
                        start_pending_lookup(&mut lookups, resolver.clone(), netloc.clone(), known.clone(), deadline);
                    }
                }
            }
        }
    };
    tokio::time::timeout_at(deadline.into(), connect)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "connect timed out"))?
}

// Delay the blocking wait until a fast first connection has had a chance to finish.
// Pending TCP attempts keep their original deadline while late answers race them.
fn start_pending_lookup(
    lookups: &mut JoinSet<io::Result<Vec<SocketAddr>>>,
    resolver: dns::Resolver,
    netloc: String,
    known: Vec<SocketAddr>,
    deadline: Instant,
) {
    lookups.spawn(async move {
        sleep(RACE_STAGGER).await;
        tokio::task::spawn_blocking(move || resolver.resolve_next(&netloc, &known, deadline))
            .await
            .map_err(io::Error::other)?
    });
}

async fn race(addresses: Vec<SocketAddr>) -> io::Result<TcpStream> {
    let mut pending = addresses.into_iter();
    let mut attempts = JoinSet::new();
    let mut last_error = None;
    loop {
        match pending.next() {
            Some(address) => {
                attempts.spawn(TcpStream::connect(address));
            }
            None if attempts.is_empty() => {
                return Err(last_error.unwrap_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, "no address to connect to")
                }));
            }
            None => {}
        }
        // A failed attempt starts the next address at once; a slow one only after the stagger.
        tokio::select! {
            Some(joined) = attempts.join_next() => match joined {
                Ok(Ok(stream)) => return Ok(stream),
                Ok(Err(error)) => last_error = Some(error),
                Err(error) => last_error = Some(io::Error::other(error)),
            },
            () = sleep(RACE_STAGGER) => {}
        }
    }
}

/// Connects a websocket (TLS for `wss`) over the raced TCP socket with Nagle off.
#[allow(
    clippy::result_large_err,
    reason = "tungstenite fixes the handshake error type"
)]
pub(crate) async fn connect_websocket(
    request: impl IntoClientRequest,
) -> Result<(WebSocket, Response), Error> {
    connect_websocket_with_resolver(request, dns::system().clone()).await
}

#[allow(
    clippy::result_large_err,
    reason = "tungstenite fixes the handshake error type"
)]
async fn connect_websocket_with_resolver(
    request: impl IntoClientRequest,
    resolver: dns::Resolver,
) -> Result<(WebSocket, Response), Error> {
    let request = request.into_client_request()?;
    let uri = request.uri();
    let host = uri.host().ok_or(Error::Url(UrlError::NoHostName))?;
    let port = uri
        .port_u16()
        .or(match uri.scheme_str() {
            Some("wss") => Some(443),
            Some("ws") => Some(80),
            _ => None,
        })
        .ok_or(Error::Url(UrlError::UnsupportedUrlScheme))?;
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    let socket = connect_tcp_with_resolver(&host, port, resolver)
        .await
        .map_err(Error::Io)?;
    socket.set_nodelay(true)?;
    client_async_tls_with_config(request, socket, None, None).await
}

#[cfg(test)]
mod tests;
