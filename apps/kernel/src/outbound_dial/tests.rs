use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    os::fd::{FromRawFd, OwnedFd},
    time::{Duration, Instant},
};

use super::*;

/// A loopback listener whose accept queue is full, so new SYNs go unanswered
/// like packets sent into a dead IPv6 path.
struct UnansweredAddress {
    address: SocketAddr,
    _listener: OwnedFd,
    _queued: Vec<TcpStream>,
}

fn unanswered_address() -> UnansweredAddress {
    // SAFETY: plain libc socket calls on a descriptor owned by the returned value.
    let (listener, address) = unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
        assert!(fd >= 0, "socket");
        let mut sockaddr: libc::sockaddr_in = std::mem::zeroed();
        sockaddr.sin_family = libc::AF_INET as libc::sa_family_t;
        sockaddr.sin_addr.s_addr = u32::from_be_bytes([127, 0, 0, 1]).to_be();
        let size = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
        assert_eq!(
            libc::bind(fd, (&sockaddr as *const libc::sockaddr_in).cast(), size),
            0
        );
        assert_eq!(libc::listen(fd, 0), 0);
        let mut bound = size;
        assert_eq!(
            libc::getsockname(
                fd,
                (&mut sockaddr as *mut libc::sockaddr_in).cast(),
                &mut bound
            ),
            0
        );
        let address = SocketAddr::from((
            std::net::Ipv4Addr::LOCALHOST,
            u16::from_be(sockaddr.sin_port),
        ));
        (OwnedFd::from_raw_fd(fd), address)
    };
    let queued = (0..3)
        .filter_map(|_| TcpStream::connect_timeout(&address, Duration::from_millis(200)).ok())
        .collect();
    UnansweredAddress {
        address,
        _listener: listener,
        _queued: queued,
    }
}

fn answering_server() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok");
        }
    });
    address
}

#[test]
fn interleave_alternates_families_starting_with_the_first_listed() {
    let v6 = |last: u16| {
        SocketAddr::from((std::net::Ipv6Addr::new(0x2606, 0, 0, 0, 0, 0, 0, last), 443))
    };
    let v4 = |last: u8| SocketAddr::from(([104, 16, 0, last], 443));
    assert_eq!(
        interleave_families(vec![v6(1), v6(2), v4(1), v4(2)]),
        vec![v6(1), v4(1), v6(2), v4(2)]
    );
    assert_eq!(
        interleave_families(vec![v4(1), v4(2), v4(3), v6(1)]),
        vec![v4(1), v6(1), v4(2), v4(3)]
    );
    assert!(interleave_families(Vec::new()).is_empty());
}

// Dead IPv6 path first, working address second: the shape of linux-lan's Cloud calls.
#[test]
fn http_call_reaches_the_working_address_behind_an_unanswered_one() {
    let stalled = unanswered_address();
    let working = answering_server();
    let addresses = vec![stalled.address, working];
    let agent = agent_builder_with_resolver(move |_| Ok(addresses.clone()))
        .timeout(Duration::from_secs(20))
        .build();
    let started = Instant::now();
    let body = agent
        .get("http://dual-stack-first.invalid/relay/token")
        .call()
        .unwrap()
        .into_string()
        .unwrap();
    let elapsed = started.elapsed();
    assert_eq!(body, "ok");
    assert!(
        elapsed < Duration::from_millis(1500),
        "connected after {elapsed:?}"
    );
}

// Only the first call over a dead path pays for it; later calls start with the family that worked.
#[test]
fn later_http_calls_skip_the_family_that_did_not_answer() {
    let stalled = unanswered_address();
    let working = answering_server();
    let addresses = vec![stalled.address, working];
    let agent = agent_builder_with_resolver(move |_| Ok(addresses.clone())).build();
    for call in 0..2 {
        let started = Instant::now();
        agent
            .get("http://dual-stack-later.invalid/")
            .call()
            .unwrap();
        let elapsed = started.elapsed();
        let budget = if call == 0 {
            Duration::from_millis(1500)
        } else {
            Duration::from_millis(300)
        };
        assert!(elapsed < budget, "call {call} took {elapsed:?}");
    }
}

#[test]
fn a_fresh_address_memory_reorders_the_resolver_list() {
    let v6 = SocketAddr::from((std::net::Ipv6Addr::LOCALHOST, 443));
    let v4 = SocketAddr::from(([203, 0, 113, 7], 443));
    assert_eq!(
        order_addresses("memory.invalid:443", vec![v6, v4]),
        vec![v6, v4]
    );
    remember_connected_address("memory.invalid:443".to_string(), v4);
    assert_eq!(
        order_addresses("memory.invalid:443", vec![v6, v4]),
        vec![v4, v6]
    );
    assert_eq!(
        order_addresses("other.invalid:443", vec![v6, v4]),
        vec![v6, v4]
    );
}

#[tokio::test]
async fn tcp_race_reaches_the_working_address_behind_an_unanswered_one() {
    let stalled = unanswered_address();
    let working = answering_server();
    let started = Instant::now();
    let stream = race(vec![stalled.address, working]).await.unwrap();
    assert_eq!(stream.peer_addr().unwrap(), working);
    assert!(
        started.elapsed() < Duration::from_millis(1000),
        "raced for {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn tcp_race_reports_the_last_error_when_nothing_answers() {
    let closed = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let error = race(vec![closed]).await.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::ConnectionRefused);
    assert_eq!(
        race(Vec::new()).await.unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
}

#[tokio::test]
async fn websocket_connects_over_the_raced_socket() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        tokio_tungstenite::accept_async(stream).await.unwrap()
    });
    let (socket, _) = connect_websocket(url.as_str()).await.unwrap();
    let MaybeTlsStream::Plain(stream) = socket.get_ref() else {
        panic!("loopback fixture must use plain TCP");
    };
    assert!(stream.nodelay().unwrap());
    drop(server.await.unwrap());
}

// A nameserver that never answers AAAA (the dead-IPv6-resolver shape on a home
// network): the A answer must start the connection at once.
#[test]
fn http_call_does_not_wait_for_an_unanswered_aaaa_lookup() {
    let working = answering_server();
    let resolver = dns::Resolver::new(move |_, _, family| match family {
        dns::Family::V4 => Ok(vec![working]),
        dns::Family::V6 => {
            std::thread::sleep(Duration::from_secs(5));
            Ok(Vec::new())
        }
    });
    let agent = agent_builder_with_resolver(move |netloc| resolver.resolve(netloc))
        .timeout(Duration::from_secs(20))
        .build();
    let started = Instant::now();
    let body = agent
        .get("http://slow-aaaa.invalid/relay/token")
        .call()
        .unwrap()
        .into_string()
        .unwrap();
    let elapsed = started.elapsed();
    assert_eq!(body, "ok");
    assert!(
        elapsed < Duration::from_millis(300),
        "answered after {elapsed:?}"
    );
}
