//! Name resolution that never lets one address family hold up the other.
//!
//! The system resolver asked for both families at once (`AF_UNSPEC`) waits for
//! the slower answer. When a network lists a dead IPv6 nameserver first, every
//! lookup then takes the resolver's 5 s timeout even though the A answer came
//! back at once. Here each family is looked up on its own thread; a call
//! returns as soon as one family has answered and gives the other only a short
//! stagger. Answers are cached per host, so only the first call after a
//! lookup went stale can wait at all, and slow lookups finish in the
//! background for later calls.

use std::{
    collections::HashMap,
    io,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Condvar, Mutex, OnceLock},
    time::{Duration, Instant},
};

/// How long the second family may take after the first one has answered.
const SECOND_FAMILY_STAGGER: Duration = Duration::from_millis(50);
/// How long an answer is used.
const ANSWER_TTL: Duration = Duration::from_secs(60);
/// Age at which a used answer is refreshed in the background.
const REFRESH_AFTER: Duration = Duration::from_secs(30);
/// Pause before a family without an answer is looked up again.
const RETRY_AFTER: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Family {
    V4,
    V6,
}

pub(crate) type Lookup =
    dyn Fn(&str, u16, Family) -> io::Result<Vec<SocketAddr>> + Send + Sync + 'static;

#[derive(Default)]
struct FamilyState {
    addresses: Vec<SocketAddr>,
    answered_at: Option<Instant>,
    attempted_at: Option<Instant>,
    in_flight: bool,
    last_error: Option<(io::ErrorKind, String)>,
}

impl FamilyState {
    fn has_fresh_answer(&self) -> bool {
        self.answered_at.is_some_and(|at| at.elapsed() < ANSWER_TTL)
    }

    fn due_for_lookup(&self, host_has_answer: bool) -> bool {
        let wait = if self.has_fresh_answer() {
            REFRESH_AFTER
        } else if host_has_answer {
            RETRY_AFTER
        } else {
            Duration::ZERO
        };
        !self.in_flight && self.attempted_at.is_none_or(|at| at.elapsed() >= wait)
    }
}

#[derive(Default)]
struct HostState {
    v4: FamilyState,
    v6: FamilyState,
}

impl HostState {
    fn family(&mut self, family: Family) -> &mut FamilyState {
        match family {
            Family::V4 => &mut self.v4,
            Family::V6 => &mut self.v6,
        }
    }

    fn has_fresh_answer(&self) -> bool {
        self.v4.has_fresh_answer() || self.v6.has_fresh_answer()
    }

    fn lookups_in_flight(&self) -> bool {
        self.v4.in_flight || self.v6.in_flight
    }

    /// Fresh addresses, with the family the system lists first (IPv6) first.
    fn addresses(&self) -> Vec<SocketAddr> {
        [&self.v6, &self.v4]
            .into_iter()
            .filter(|state| state.has_fresh_answer())
            .flat_map(|state| state.addresses.iter().copied())
            .collect()
    }

    fn error(&self) -> io::Error {
        let (kind, message) = self
            .v4
            .last_error
            .as_ref()
            .or(self.v6.last_error.as_ref())
            .cloned()
            .unwrap_or((io::ErrorKind::NotFound, "no address found".to_string()));
        io::Error::new(kind, message)
    }
}

struct Shared {
    lookup: Box<Lookup>,
    hosts: Mutex<HashMap<String, HostState>>,
    answered: Condvar,
}

pub(crate) struct Resolver {
    shared: Arc<Shared>,
}

impl Resolver {
    pub(crate) fn new(
        lookup: impl Fn(&str, u16, Family) -> io::Result<Vec<SocketAddr>> + Send + Sync + 'static,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                lookup: Box::new(lookup),
                hosts: Mutex::new(HashMap::new()),
                answered: Condvar::new(),
            }),
        }
    }

    /// Resolves `host:port` (`[v6]:port` for literals) to socket addresses.
    pub(crate) fn resolve(&self, netloc: &str) -> io::Result<Vec<SocketAddr>> {
        if let Ok(address) = netloc.parse::<SocketAddr>() {
            return Ok(vec![address]);
        }
        let (host, port) = split_netloc(netloc)?;
        if let Ok(ip) = host.parse::<IpAddr>() {
            return Ok(vec![SocketAddr::new(ip, port)]);
        }
        self.resolve_name(netloc, host, port)
    }

    fn resolve_name(&self, netloc: &str, host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
        let mut hosts = self.lock_hosts();
        let state = hosts.entry(netloc.to_string()).or_default();
        let answered_before_call = state.has_fresh_answer();
        for family in [Family::V6, Family::V4] {
            if state.family(family).due_for_lookup(answered_before_call) {
                self.start_lookup(state.family(family), netloc, host, port, family);
            }
        }
        let mut stagger_ends_at = None;
        loop {
            let state = hosts.entry(netloc.to_string()).or_default();
            if state.has_fresh_answer() {
                if answered_before_call || !state.lookups_in_flight() {
                    return Ok(state.addresses());
                }
                let ends_at =
                    *stagger_ends_at.get_or_insert_with(|| Instant::now() + SECOND_FAMILY_STAGGER);
                let Some(remaining) = ends_at.checked_duration_since(Instant::now()) else {
                    return Ok(state.addresses());
                };
                hosts = self
                    .shared
                    .answered
                    .wait_timeout(hosts, remaining)
                    .unwrap_or_else(|error| error.into_inner())
                    .0;
            } else if !state.lookups_in_flight() {
                return Err(state.error());
            } else {
                hosts = self
                    .shared
                    .answered
                    .wait(hosts)
                    .unwrap_or_else(|error| error.into_inner());
            }
        }
    }

    fn start_lookup(
        &self,
        state: &mut FamilyState,
        netloc: &str,
        host: &str,
        port: u16,
        family: Family,
    ) {
        state.in_flight = true;
        state.attempted_at = Some(Instant::now());
        let shared = Arc::clone(&self.shared);
        let (netloc, host) = (netloc.to_string(), host.to_string());
        let spawned = std::thread::Builder::new()
            .name("dns-lookup".into())
            .spawn(move || {
                let result = (shared.lookup)(&host, port, family);
                let mut hosts = shared
                    .hosts
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                let state = hosts.entry(netloc).or_default().family(family);
                state.in_flight = false;
                match result {
                    Ok(addresses) if !addresses.is_empty() => {
                        state.addresses = addresses;
                        state.answered_at = Some(Instant::now());
                        state.last_error = None;
                    }
                    Ok(_) => {
                        state.last_error = Some((
                            io::ErrorKind::NotFound,
                            format!("`{host}` has no {family:?} address"),
                        ));
                    }
                    Err(error) => state.last_error = Some((error.kind(), error.to_string())),
                }
                shared.answered.notify_all();
            });
        if let Err(error) = spawned {
            state.in_flight = false;
            state.last_error = Some((error.kind(), format!("cannot start DNS lookup: {error}")));
        }
    }

    fn lock_hosts(&self) -> std::sync::MutexGuard<'_, HashMap<String, HostState>> {
        self.shared
            .hosts
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
}

fn split_netloc(netloc: &str) -> io::Result<(&str, u16)> {
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("`{netloc}` is not host:port"),
        )
    };
    let (host, port) = netloc.rsplit_once(':').ok_or_else(invalid)?;
    let port = port.parse().map_err(|_| invalid())?;
    Ok((host.trim_start_matches('[').trim_end_matches(']'), port))
}

/// The process-wide resolver backed by the system's `getaddrinfo`.
pub(crate) fn system() -> &'static Resolver {
    static SYSTEM: OnceLock<Resolver> = OnceLock::new();
    SYSTEM.get_or_init(|| Resolver::new(system_lookup))
}

#[cfg(unix)]
fn system_lookup(host: &str, port: u16, family: Family) -> io::Result<Vec<SocketAddr>> {
    use std::ffi::CString;

    let host = CString::new(host)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "host contains a NUL byte"))?;
    // SAFETY: `hints` is zeroed and then set field by field, `host` outlives the
    // call, and the list `getaddrinfo` returns is read only before `freeaddrinfo`.
    unsafe {
        let mut hints: libc::addrinfo = std::mem::zeroed();
        hints.ai_family = match family {
            Family::V4 => libc::AF_INET,
            Family::V6 => libc::AF_INET6,
        };
        hints.ai_socktype = libc::SOCK_STREAM;
        let mut list: *mut libc::addrinfo = std::ptr::null_mut();
        let status = libc::getaddrinfo(host.as_ptr(), std::ptr::null(), &hints, &mut list);
        if status != 0 {
            let message = std::ffi::CStr::from_ptr(libc::gai_strerror(status)).to_string_lossy();
            return Err(io::Error::other(format!(
                "cannot resolve {family:?} address: {message}"
            )));
        }
        let mut addresses = Vec::new();
        let mut entry = list;
        while !entry.is_null() {
            let info = &*entry;
            match i32::from((*info.ai_addr).sa_family) {
                libc::AF_INET => {
                    let sockaddr = &*(info.ai_addr as *const libc::sockaddr_in);
                    let ip = std::net::Ipv4Addr::from(u32::from_be(sockaddr.sin_addr.s_addr));
                    addresses.push(SocketAddr::new(ip.into(), port));
                }
                libc::AF_INET6 => {
                    let sockaddr = &*(info.ai_addr as *const libc::sockaddr_in6);
                    let ip = std::net::Ipv6Addr::from(sockaddr.sin6_addr.s6_addr);
                    addresses.push(SocketAddr::new(ip.into(), port));
                }
                _ => {}
            }
            entry = info.ai_next;
        }
        libc::freeaddrinfo(list);
        Ok(addresses)
    }
}

#[cfg(not(unix))]
fn system_lookup(host: &str, port: u16, family: Family) -> io::Result<Vec<SocketAddr>> {
    use std::net::ToSocketAddrs;

    Ok((host, port)
        .to_socket_addrs()?
        .filter(|address| address.is_ipv6() == (family == Family::V6))
        .collect())
}

#[cfg(test)]
mod tests;
