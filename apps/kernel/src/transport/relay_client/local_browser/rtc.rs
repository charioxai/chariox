//! MP-08/MP-11 protocol 917: the direct carrier over a WebRTC data channel.
//!
//! The browser's SDP offer arrives inside an encrypted relay request from a
//! relay-verified identity and the answer returns the same way, so the DTLS
//! fingerprints that secure the data channel were exchanged end to end. On the
//! channel the browser then runs the same grant/key handshake as on the
//! loopback WebSocket, bound to the grant issued with its offer, followed by
//! the same encrypted relay client frames. The channel adds reachability (LAN,
//! public and Tailscale addresses), not authority.
//!
//! Every data-channel message is binary: one kind byte, then payload. Text
//! frames larger than one fragment span several `MORE` messages and a `LAST`.
//!
//! Besides its interface addresses the answer carries the public address each
//! socket maps to, learned from the STUN responder on the relay's host, so a
//! browser behind another NAT can reach a kernel behind NAT.

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::OnceLock;
use std::task::{Context, Poll};

use chariox_relay::stun;
use futures_channel::mpsc as channel_mpsc;
use futures_util::{Sink, Stream};
use str0m::change::SdpOffer;
use str0m::channel::ChannelId;
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc, RtcConfig};
use tokio::net::UdpSocket;
use tokio::sync::{oneshot, Notify};

use super::super::daemon_requests::request_limits::MAX_MESSAGE_BYTES;
use super::*;

pub(super) const UDP_PORT_ENV: &str = "CHARIOX_KERNEL_DIRECT_UDP_PORT";
const MAX_OFFER_BYTES: usize = 16 * 1024;
const MAX_PEERS: usize = 32;
/// A peer whose data channel has not opened by then is dropped with its grant.
const OPEN_DEADLINE: Duration = Duration::from_secs(15);
const HANDSHAKE_DEADLINE: Duration = Duration::from_secs(5);
const FRAGMENT_BYTES: usize = 16 * 1024;
/// SCTP bytes queued per peer before the session's writer waits.
const SEND_BUFFER_HIGH: usize = 1024 * 1024;
const INCOMING_QUEUE: usize = 256;
const OUTGOING_QUEUE: usize = 64;
const MAX_DATAGRAM: usize = 2048;
/// How long each STUN Binding request waits before it is sent once more; a
/// socket that gets no answer offers only its interface address.
const STUN_WAITS: [Duration; 2] = [Duration::from_millis(300), Duration::from_millis(400)];

/// An open channel carries a keepalive this often, ahead of queued frames, so
/// the browser tells a dead direct path from a quiet one in a few seconds and
/// answers it even while it only receives (TR-16).
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(1);
/// A browser that neither answered nor acknowledged anything for this long left
/// the path (UDP dropped, moved to the relay): its session ends and stops
/// feeding a dead carrier. On a slow link the keepalive can wait behind a
/// megabyte already in SCTP; the browser acknowledging that data is alive.
const SILENCE_LIMIT: Duration = Duration::from_secs(3);

const FRAME_LAST: u8 = 0;
const FRAME_MORE: u8 = 1;
const FRAME_PING: u8 = 2;
const FRAME_PONG: u8 = 3;

/// The kernel's UDP sockets, one per usable interface address (a socket bound
/// to one address knows the destination str0m matches candidates against).
pub(in crate::transport::relay_client) struct RtcEndpoint {
    port: Option<u16>,
    interfaces: Interfaces,
    state: Mutex<Option<Bound>>,
}

/// The machine's usable interface addresses right now.
pub(super) type Interfaces = Arc<dyn Fn() -> Vec<IpAddr> + Send + Sync>;

#[derive(Clone)]
struct Bound {
    epoch: String,
    /// The interface addresses these sockets were bound for; a different
    /// list means the machine changed network and the sockets are stale.
    interfaces: Vec<IpAddr>,
    sockets: Vec<Arc<UdpSocket>>,
    addresses: Vec<SocketAddr>,
    commands: mpsc::UnboundedSender<Command>,
    stun: Arc<StunWaits>,
}

enum Command {
    /// The sockets to read and send from from now on.
    Sockets(Vec<Arc<UdpSocket>>),
    Peer(Box<NewPeer>),
}

/// Binding requests in flight, by transaction id; the socket readers hand
/// the matching answers over instead of passing them to str0m.
type StunWaits = std::sync::Mutex<HashMap<[u8; 12], oneshot::Sender<SocketAddr>>>;

struct NewPeer {
    rtc: Rtc,
    grant: String,
    direct: std::sync::Weak<LocalBrowserDirect>,
}

impl RtcEndpoint {
    pub(super) fn new(port: Option<u16>, interfaces: Interfaces) -> Self {
        Self {
            port,
            interfaces,
            state: Mutex::new(None),
        }
    }

    /// Answers the offer with this kernel's host and server-reflexive
    /// candidates and hands the peer to the driver. Returns the SDP answer and
    /// the endpoint epoch.
    pub(super) async fn accept(
        &self,
        direct: std::sync::Weak<LocalBrowserDirect>,
        offer: &str,
        grant: &str,
    ) -> Result<(String, String), RelayError> {
        if offer.len() > MAX_OFFER_BYTES {
            return Err(invalid_offer("WebRTC offer is too large"));
        }
        let offer = SdpOffer::from_sdp_string(offer)
            .map_err(|_| invalid_offer("WebRTC offer is not valid SDP"))?;
        let bound = self.bind(&direct).await?;
        let relay_url = direct
            .upgrade()
            .and_then(|direct| direct.router.relay_config_snapshot().relay_url);
        let reflexive = match relay_url {
            Some(relay_url) => bound.reflexive(&stun_servers(&relay_url).await).await,
            None => Vec::new(),
        };
        let mut rtc = RtcConfig::new()
            .set_crypto_provider(crypto_provider())
            .build(std::time::Instant::now());
        for address in &bound.addresses {
            if let Ok(candidate) = Candidate::host(*address, "udp") {
                rtc.add_local_candidate(candidate);
            }
        }
        for (mapped, base) in reflexive {
            if let Ok(candidate) = Candidate::server_reflexive(mapped, base, "udp") {
                rtc.add_local_candidate(candidate);
            }
        }
        let answer = rtc
            .sdp_api()
            .accept_offer(offer)
            .map_err(|_| invalid_offer("WebRTC offer has no usable data channel"))?;
        bound
            .commands
            .send(Command::Peer(Box::new(NewPeer {
                rtc,
                grant: grant.to_owned(),
                direct,
            })))
            .map_err(|_| unavailable("direct WebRTC endpoint is shutting down"))?;
        Ok((answer.to_sdp_string(), bound.epoch))
    }

    /// The endpoint for the interfaces the machine has now. A changed
    /// interface list (a laptop moving between networks) rebinds: sockets of
    /// addresses that remain keep serving their peers, vanished addresses are
    /// no longer offered and new ones are.
    async fn bind(
        &self,
        direct: &std::sync::Weak<LocalBrowserDirect>,
    ) -> Result<Bound, RelayError> {
        let mut state = self.state.lock().await;
        let interfaces = (self.interfaces)();
        if let Some(bound) = state
            .as_ref()
            .filter(|bound| bound.interfaces == interfaces)
        {
            return Ok(bound.clone());
        }
        let port = self
            .port
            .ok_or_else(|| unavailable("direct WebRTC connections are disabled on this kernel"))?;
        let mut sockets = Vec::new();
        for ip in &interfaces {
            let kept = state.as_ref().and_then(|bound| {
                bound
                    .sockets
                    .iter()
                    .find(|socket| socket.local_addr().is_ok_and(|address| address.ip() == *ip))
            });
            if let Some(kept) = kept {
                sockets.push(Arc::clone(kept));
                continue;
            }
            match UdpSocket::bind(SocketAddr::new(*ip, port)).await {
                Ok(socket) => sockets.push(Arc::new(socket)),
                Err(error) => crate::logging::warn_with_fields(
                    "daemon.local_browser",
                    "direct WebRTC address unavailable",
                    serde_json::json!({"address": ip.to_string(), "error": error.to_string()}),
                ),
            }
        }
        let addresses: Vec<SocketAddr> = sockets
            .iter()
            .filter_map(|socket| socket.local_addr().ok())
            .collect();
        if addresses.is_empty() {
            return Err(unavailable("direct WebRTC endpoint has no usable address"));
        }
        crate::logging::info_with_fields(
            "daemon.local_browser",
            "direct WebRTC endpoint listening",
            serde_json::json!({"addresses": addresses.iter().map(ToString::to_string).collect::<Vec<_>>()}),
        );
        let bound = match state.take() {
            Some(previous) => Bound {
                interfaces,
                sockets,
                addresses,
                ..previous
            },
            None => {
                let shutdown = direct
                    .upgrade()
                    .map(|direct| direct.shutdown.clone())
                    .ok_or_else(|| unavailable("direct WebRTC endpoint is shutting down"))?;
                let (commands, new_commands) = mpsc::unbounded_channel();
                let stun = Arc::new(StunWaits::default());
                tokio::spawn(drive(Arc::clone(&stun), new_commands, shutdown));
                Bound {
                    epoch: random_token(),
                    interfaces,
                    sockets,
                    addresses,
                    commands,
                    stun,
                }
            }
        };
        let handed = bound
            .commands
            .send(Command::Sockets(bound.sockets.clone()))
            .map_err(|_| unavailable("direct WebRTC endpoint is shutting down"));
        *state = Some(bound.clone());
        handed.map(|()| bound)
    }
}

impl Bound {
    /// The public address each socket maps to, paired with the socket's own
    /// address; sockets the NAT does not translate are left out.
    async fn reflexive(&self, servers: &[SocketAddr]) -> Vec<(SocketAddr, SocketAddr)> {
        let asks = self.sockets.iter().filter_map(|socket| {
            let base = socket.local_addr().ok()?;
            let server = *servers
                .iter()
                .find(|server| server.is_ipv4() == base.is_ipv4())?;
            Some(self.ask(socket, base, server))
        });
        futures_util::future::join_all(asks)
            .await
            .into_iter()
            .flatten()
            .filter(|(mapped, base)| mapped != base)
            .collect()
    }

    async fn ask(
        &self,
        socket: &UdpSocket,
        base: SocketAddr,
        server: SocketAddr,
    ) -> Option<(SocketAddr, SocketAddr)> {
        let transaction: [u8; 12] = rand::random();
        let (answer, mut answered) = oneshot::channel();
        self.stun
            .lock()
            .expect("STUN waits poisoned")
            .insert(transaction, answer);
        let mut mapped = None;
        for wait in STUN_WAITS {
            let _ = socket
                .send_to(&stun::binding_request(transaction), server)
                .await;
            if let Ok(Ok(address)) = timeout(wait, &mut answered).await {
                mapped = Some(address);
                break;
            }
        }
        self.stun
            .lock()
            .expect("STUN waits poisoned")
            .remove(&transaction);
        mapped.map(|mapped| (mapped, base))
    }
}

/// The STUN responder runs on the relay's host at the STUN port.
async fn stun_servers(relay_url: &str) -> Vec<SocketAddr> {
    let Some(host) = url::Url::parse(relay_url).ok().and_then(|url| {
        url.host_str()
            .map(|host| host.trim_matches(['[', ']']).to_owned())
    }) else {
        return Vec::new();
    };
    let servers = timeout(
        STUN_WAITS[0],
        tokio::net::lookup_host((host.as_str(), stun::DEFAULT_PORT)),
    )
    .await;
    match servers {
        Ok(Ok(servers)) => servers.collect(),
        _ => Vec::new(),
    }
}

fn crypto_provider() -> Arc<str0m::config::CryptoProvider> {
    static PROVIDER: OnceLock<Arc<str0m::config::CryptoProvider>> = OnceLock::new();
    Arc::clone(PROVIDER.get_or_init(|| Arc::new(str0m::crypto::from_feature_flags())))
}

fn invalid_offer(message: &str) -> RelayError {
    relay_error("invalid_webrtc_offer", message, false)
}

/// Unicast interface addresses a peer could reach: LAN, public and VPN
/// (Tailscale is an interface like any other). Loopback and link-local are
/// left out; a same-machine browser reaches the LAN address.
pub(super) fn host_addresses() -> Vec<IpAddr> {
    let mut addresses = Vec::new();
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills `list` with a linked list we free below.
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return addresses;
    }
    let mut cursor = list;
    while !cursor.is_null() {
        // SAFETY: `cursor` walks the list returned by getifaddrs.
        let entry = unsafe { &*cursor };
        cursor = entry.ifa_next;
        if entry.ifa_addr.is_null() || entry.ifa_flags & libc::IFF_UP as u32 == 0 {
            continue;
        }
        // SAFETY: the family tag says which sockaddr layout follows.
        let ip = unsafe {
            match i32::from((*entry.ifa_addr).sa_family) {
                libc::AF_INET => {
                    let addr = &*(entry.ifa_addr as *const libc::sockaddr_in);
                    IpAddr::from(u32::from_be(addr.sin_addr.s_addr).to_be_bytes())
                }
                libc::AF_INET6 => {
                    let addr = &*(entry.ifa_addr as *const libc::sockaddr_in6);
                    IpAddr::from(addr.sin6_addr.s6_addr)
                }
                _ => continue,
            }
        };
        let usable = match ip {
            IpAddr::V4(ip) => !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified(),
            IpAddr::V6(ip) => {
                !ip.is_loopback() && !ip.is_unspecified() && !ip.is_unicast_link_local()
            }
        };
        if usable && !addresses.contains(&ip) {
            addresses.push(ip);
        }
    }
    // SAFETY: `list` came from a successful getifaddrs.
    unsafe { libc::freeifaddrs(list) };
    addresses
}

#[cfg(test)]
pub(super) fn host_addresses_for_tests() -> Vec<IpAddr> {
    host_addresses()
}

/// A session's view of one data channel: whole frames in and out.
pub(super) struct ChannelSocket {
    incoming: channel_mpsc::Receiver<Message>,
    outgoing: channel_mpsc::Sender<Message>,
    wake: Arc<Notify>,
}

/// The data channel is gone; the session ends as on a closed WebSocket.
#[derive(Debug)]
pub(super) struct ChannelClosed;

impl Stream for ChannelSocket {
    type Item = Result<Message, ChannelClosed>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.incoming)
            .poll_next(cx)
            .map(|message| message.map(Ok))
    }
}

impl Sink<Message> for ChannelSocket {
    type Error = ChannelClosed;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.outgoing)
            .poll_ready(cx)
            .map_err(|_| ChannelClosed)
    }

    fn start_send(mut self: Pin<&mut Self>, message: Message) -> Result<(), Self::Error> {
        Pin::new(&mut self.outgoing)
            .start_send(message)
            .map_err(|_| ChannelClosed)?;
        self.wake.notify_one();
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.outgoing.close_channel();
        self.wake.notify_one();
        Poll::Ready(Ok(()))
    }
}

struct Peer {
    rtc: Rtc,
    grant: String,
    direct: std::sync::Weak<LocalBrowserDirect>,
    created: Instant,
    channel: Option<OpenChannel>,
}

struct OpenChannel {
    id: ChannelId,
    incoming: channel_mpsc::Sender<Message>,
    outgoing: channel_mpsc::Receiver<Message>,
    /// Encoded fragments waiting for SCTP buffer room.
    pending: VecDeque<Vec<u8>>,
    reassembly: Vec<u8>,
    session_done: bool,
    last_ping: std::time::Instant,
    /// When the browser last sent a frame or acknowledged queued data.
    last_heard: std::time::Instant,
    /// SCTP bytes queued when last looked at; less now means acknowledged.
    buffered: usize,
}

/// Reads one socket until the driver retires it; STUN answers go to their
/// waiting request, everything else to str0m.
async fn read_socket(
    socket: Arc<UdpSocket>,
    stun_waits: Arc<StunWaits>,
    datagrams: mpsc::Sender<(SocketAddr, SocketAddr, Vec<u8>)>,
) {
    let Ok(local) = socket.local_addr() else {
        return;
    };
    let mut buf = vec![0_u8; MAX_DATAGRAM];
    while let Ok((len, source)) = socket.recv_from(&mut buf).await {
        if let Some(answer) = stun::transaction(&buf[..len]).and_then(|transaction| {
            let waiting = stun_waits
                .lock()
                .expect("STUN waits poisoned")
                .remove(&transaction)?;
            Some((waiting, stun::mapped_address(&buf[..len], transaction)))
        }) {
            if let (waiting, Some(mapped)) = answer {
                let _ = waiting.send(mapped);
            }
            continue;
        }
        if datagrams
            .send((source, local, buf[..len].to_vec()))
            .await
            .is_err()
        {
            return;
        }
    }
}

/// One task owns every `Rtc`: str0m is sans-IO, so this loop feeds datagrams
/// and timeouts in and writes transmits and channel data out.
async fn drive(
    stun_waits: Arc<StunWaits>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    mut shutdown: watch::Receiver<bool>,
) {
    let wake = Arc::new(Notify::new());
    let (datagrams_tx, mut datagrams) = mpsc::channel::<(SocketAddr, SocketAddr, Vec<u8>)>(1024);
    let mut sockets: Vec<Arc<UdpSocket>> = Vec::new();
    let mut readers: HashMap<SocketAddr, tokio::task::AbortHandle> = HashMap::new();
    let mut tasks = tokio::task::JoinSet::new();
    let mut peers: Vec<Peer> = Vec::new();
    loop {
        let now = std::time::Instant::now();
        let mut next = now + Duration::from_millis(500);
        for peer in &mut peers {
            if let Some(deadline) = poll_peer(peer, &sockets, &wake).await {
                next = next.min(deadline);
            }
        }
        peers.retain_mut(|peer| {
            let alive = peer.rtc.is_alive()
                && (peer.channel.is_some() || peer.created.elapsed() < OPEN_DEADLINE)
                && !peer
                    .channel
                    .as_ref()
                    .is_some_and(|channel| channel.session_done && channel.pending.is_empty());
            if !alive {
                peer.rtc.disconnect();
                forget_grant(peer);
            }
            alive
        });
        let sleep_for = next.saturating_duration_since(std::time::Instant::now());
        tokio::select! {
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    for peer in &mut peers {
                        peer.rtc.disconnect();
                    }
                    return;
                }
            }
            command = commands.recv() => match command {
                Some(Command::Sockets(current)) => {
                    // A socket that is no longer offered stops being read and
                    // is closed once its last peer lets go of it.
                    readers.retain(|address, reader| {
                        let kept = current
                            .iter()
                            .any(|socket| socket.local_addr().ok() == Some(*address));
                        if !kept {
                            reader.abort();
                        }
                        kept
                    });
                    for socket in &current {
                        if let Ok(address) = socket.local_addr() {
                            readers.entry(address).or_insert_with(|| {
                                tasks.spawn(read_socket(
                                    Arc::clone(socket),
                                    Arc::clone(&stun_waits),
                                    datagrams_tx.clone(),
                                ))
                            });
                        }
                    }
                    while tasks.try_join_next().is_some() {}
                    sockets = current;
                }
                Some(Command::Peer(peer)) if peers.len() < MAX_PEERS => peers.push(Peer {
                    rtc: peer.rtc,
                    grant: peer.grant,
                    direct: peer.direct,
                    created: Instant::now(),
                    channel: None,
                }),
                Some(Command::Peer(mut peer)) => {
                    peer.rtc.disconnect();
                    if let Some(direct) = peer.direct.upgrade() {
                        direct.forget_grant(&peer.grant);
                    }
                }
                None => return,
            },
            datagram = datagrams.recv() => {
                let Some((source, destination, contents)) = datagram else { return };
                let Ok(contents) = contents.as_slice().try_into() else { continue };
                let input = Input::Receive(
                    std::time::Instant::now(),
                    Receive { proto: Protocol::Udp, source, destination, contents },
                );
                if let Some(peer) = peers.iter_mut().find(|peer| peer.rtc.accepts(&input)) {
                    if peer.rtc.handle_input(input).is_err() {
                        peer.rtc.disconnect();
                    }
                }
            }
            _ = wake.notified() => {}
            _ = sleep(sleep_for) => {
                let now = std::time::Instant::now();
                for peer in &mut peers {
                    if peer.rtc.handle_input(Input::Timeout(now)).is_err() {
                        peer.rtc.disconnect();
                    }
                }
            }
        }
    }
}

/// Moves queued session frames into SCTP, then drains str0m's output.
/// Returns when str0m next wants a timeout.
async fn poll_peer(
    peer: &mut Peer,
    sockets: &[Arc<UdpSocket>],
    wake: &Arc<Notify>,
) -> Option<std::time::Instant> {
    let keepalive = keep_alive(peer);
    flush_outgoing(peer);
    loop {
        let output = match peer.rtc.poll_output() {
            Ok(output) => output,
            Err(_) => {
                peer.rtc.disconnect();
                return None;
            }
        };
        match output {
            Output::Timeout(deadline) => {
                return Some(keepalive.map_or(deadline, |keepalive| keepalive.min(deadline)))
            }
            Output::Transmit(transmit) => {
                if let Some(socket) = sockets
                    .iter()
                    .find(|socket| socket.local_addr().ok() == Some(transmit.source))
                {
                    let _ = socket
                        .send_to(&transmit.contents, transmit.destination)
                        .await;
                }
            }
            Output::Event(event) => match event {
                Event::IceConnectionStateChange(IceConnectionState::Disconnected) => {
                    peer.rtc.disconnect();
                }
                Event::ChannelOpen(id, _) if peer.channel.is_none() => {
                    open_channel(peer, id, wake);
                }
                Event::ChannelData(data) => {
                    if !receive(peer, data.id, data.binary, &data.data) {
                        peer.rtc.disconnect();
                    }
                }
                Event::ChannelClose(id) => {
                    if peer
                        .channel
                        .as_ref()
                        .is_some_and(|channel| channel.id == id)
                    {
                        peer.rtc.disconnect();
                    }
                }
                Event::ChannelBufferedAmountLow(_) => flush_outgoing(peer),
                _ => {}
            },
        }
    }
}

fn open_channel(peer: &mut Peer, id: ChannelId, wake: &Arc<Notify>) {
    let Some(direct) = peer.direct.upgrade() else {
        peer.rtc.disconnect();
        return;
    };
    if let Some(mut channel) = peer.rtc.channel(id) {
        channel.set_buffered_amount_low_threshold(SEND_BUFFER_HIGH / 2);
    }
    let (incoming_tx, incoming) = channel_mpsc::channel(INCOMING_QUEUE);
    let (outgoing, outgoing_rx) = channel_mpsc::channel(OUTGOING_QUEUE);
    peer.channel = Some(OpenChannel {
        id,
        incoming: incoming_tx,
        outgoing: outgoing_rx,
        pending: VecDeque::new(),
        reassembly: Vec::new(),
        session_done: false,
        last_ping: std::time::Instant::now(),
        last_heard: std::time::Instant::now(),
        buffered: 0,
    });
    let socket = ChannelSocket {
        incoming,
        outgoing,
        wake: Arc::clone(wake),
    };
    let grant = peer.grant.clone();
    tokio::spawn(admit(direct, socket, grant));
}

/// The loopback handshake over the channel, bound to the offer's own grant.
async fn admit(direct: Arc<LocalBrowserDirect>, mut socket: ChannelSocket, grant: String) {
    let Some(origin) = direct
        .authority
        .borrow()
        .as_ref()
        .map(|authority| authority.origin.clone())
    else {
        return;
    };
    let Some(epoch) = direct
        .rtc
        .state
        .lock()
        .await
        .as_ref()
        .map(|bound| bound.epoch.clone())
    else {
        return;
    };
    let admitted = timeout(
        HANDSHAKE_DEADLINE,
        super::admission::handshake(&direct, &epoch, &mut socket, &origin, Some(&grant)),
    )
    .await
    .unwrap_or(Err("direct WebRTC handshake timed out"));
    match admitted {
        Ok(grant) => super::session::run(direct, socket, grant).await,
        Err(reason) => {
            direct.forget_grant(&grant);
            crate::logging::warn_with_fields(
                "daemon.local_browser",
                "direct WebRTC connection refused",
                serde_json::json!({"reason": reason}),
            );
        }
    }
}

fn forget_grant(peer: &Peer) {
    if peer.channel.is_none() {
        if let Some(direct) = peer.direct.upgrade() {
            direct.forget_grant(&peer.grant);
        }
    }
}

/// Reassembles fragments; returns false when the peer broke the framing.
fn receive(peer: &mut Peer, id: ChannelId, binary: bool, data: &[u8]) -> bool {
    let Some(channel) = peer.channel.as_mut().filter(|channel| channel.id == id) else {
        return true;
    };
    channel.last_heard = std::time::Instant::now();
    let Some((&kind, payload)) = data.split_first().filter(|_| binary) else {
        return false;
    };
    let message = match kind {
        FRAME_PONG => Message::Pong(Vec::new().into()),
        FRAME_PING => {
            channel.pending.push_back(vec![FRAME_PONG]);
            return true;
        }
        FRAME_MORE | FRAME_LAST => {
            if channel.reassembly.len() + payload.len() > MAX_MESSAGE_BYTES {
                return false;
            }
            channel.reassembly.extend_from_slice(payload);
            if kind == FRAME_MORE {
                return true;
            }
            match String::from_utf8(std::mem::take(&mut channel.reassembly)) {
                Ok(text) => Message::Text(text.into()),
                Err(_) => return false,
            }
        }
        _ => return false,
    };
    // A full queue means the session stopped reading: end the carrier.
    channel.incoming.try_send(message).is_ok()
}

/// Puts a keepalive ahead of the queued fragments every second and ends a peer
/// that went silent; returns when the next check is due.
fn keep_alive(peer: &mut Peer) -> Option<std::time::Instant> {
    note_acknowledged(peer);
    let channel = peer.channel.as_mut()?;
    if channel.last_heard.elapsed() >= SILENCE_LIMIT {
        peer.rtc.disconnect();
        return None;
    }
    if channel.last_ping.elapsed() >= KEEPALIVE_INTERVAL {
        channel.pending.push_front(vec![FRAME_PING]);
        channel.last_ping = std::time::Instant::now();
    }
    Some((channel.last_ping + KEEPALIVE_INTERVAL).min(channel.last_heard + SILENCE_LIMIT))
}

/// Counts SCTP data the browser acknowledged since the last look as hearing
/// from it while more data is still queued, since a keepalive may wait behind
/// that data. Only acknowledgements shrink the buffered amount; once all is
/// acknowledged the browser has the keepalive and must answer it.
fn note_acknowledged(peer: &mut Peer) {
    let Some(channel) = peer.channel.as_mut() else {
        return;
    };
    let Some(mut writer) = peer.rtc.channel(channel.id) else {
        return;
    };
    let buffered = writer.buffered_amount();
    if buffered < channel.buffered && buffered > 0 {
        channel.last_heard = std::time::Instant::now();
    }
    channel.buffered = buffered;
}

/// Encodes queued session frames while SCTP has room.
fn flush_outgoing(peer: &mut Peer) {
    note_acknowledged(peer);
    let Some(channel) = peer.channel.as_mut() else {
        return;
    };
    loop {
        if channel.pending.is_empty() {
            match channel.outgoing.try_recv() {
                Ok(message) => encode(message, &mut channel.pending),
                Err(channel_mpsc::TryRecvError::Closed) => {
                    channel.session_done = true;
                    break;
                }
                Err(channel_mpsc::TryRecvError::Empty) => break,
            }
        }
        let Some(fragment) = channel.pending.front() else {
            continue;
        };
        let Some(mut writer) = peer.rtc.channel(channel.id) else {
            break;
        };
        if writer.buffered_amount() >= SEND_BUFFER_HIGH {
            break;
        }
        match writer.write(true, fragment) {
            Ok(true) => {
                channel.pending.pop_front();
            }
            Ok(false) => break,
            Err(_) => {
                peer.rtc.disconnect();
                break;
            }
        }
    }
    // Writes only grow the buffered amount: start the next comparison here.
    note_acknowledged(peer);
}

fn encode(message: Message, pending: &mut VecDeque<Vec<u8>>) {
    let text = match message {
        Message::Text(text) => text,
        Message::Ping(_) => {
            pending.push_back(vec![FRAME_PING]);
            return;
        }
        _ => return,
    };
    let bytes = text.as_bytes();
    let mut chunks = bytes.chunks(FRAGMENT_BYTES).peekable();
    if chunks.peek().is_none() {
        pending.push_back(vec![FRAME_LAST]);
    }
    while let Some(chunk) = chunks.next() {
        let kind = if chunks.peek().is_some() {
            FRAME_MORE
        } else {
            FRAME_LAST
        };
        let mut fragment = Vec::with_capacity(chunk.len() + 1);
        fragment.push(kind);
        fragment.extend_from_slice(chunk);
        pending.push_back(fragment);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mp08_text_frames_fragment_and_mark_the_last_piece() {
        let mut pending = VecDeque::new();
        let text = "x".repeat(FRAGMENT_BYTES * 2 + 5);
        encode(Message::Text(text.clone().into()), &mut pending);
        assert_eq!(pending.len(), 3);
        assert_eq!(pending[0][0], FRAME_MORE);
        assert_eq!(pending[1][0], FRAME_MORE);
        assert_eq!(pending[2][0], FRAME_LAST);
        let joined: Vec<u8> = pending
            .iter()
            .flat_map(|piece| piece[1..].to_vec())
            .collect();
        assert_eq!(joined, text.as_bytes());
    }

    #[test]
    fn mp08_host_addresses_leave_out_loopback() {
        assert!(host_addresses().iter().all(|ip| !ip.is_loopback()));
    }
}
