use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    os::fd::AsRawFd,
    time::{Duration, Instant},
};

use super::*;

/// A loopback listener whose accept queue is full, so new SYNs go unanswered
/// like packets sent into a dead IPv6 path.
struct UnansweredAddress {
    address: SocketAddr,
    _listener: TcpListener,
    _queued: Vec<TcpStream>,
}

fn unanswered_address() -> UnansweredAddress {
    unanswered_address_on(false)
}

fn unanswered_address_on(ipv6: bool) -> UnansweredAddress {
    let listener = TcpListener::bind(if ipv6 { "[::1]:0" } else { "127.0.0.1:0" }).unwrap();
    let address = listener.local_addr().unwrap();
    // SAFETY: the descriptor remains owned by listener; reset its empty accept
    // queue to the smallest backlog so subsequent connections stop answering.
    assert_eq!(unsafe { libc::listen(listener.as_raw_fd(), 0) }, 0);
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
    answering_server_on(false)
}

fn answering_server_on(ipv6: bool) -> SocketAddr {
    let listener = TcpListener::bind(if ipv6 { "[::1]:0" } else { "127.0.0.1:0" }).unwrap();
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
    let agent = agent_builder_with_resolver(move |_: &str| Ok(addresses.clone()))
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
    let agent = agent_builder_with_resolver(move |_: &str| Ok(addresses.clone())).build();
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
    let agent = agent_builder_with_resolver(resolver)
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

// The reviewer's case: AAAA answers at once with a path that drops SYNs and the
// working A answer arrives 100 ms later. The call must still reach the A host.
#[test]
fn http_call_reaches_a_late_a_answer_when_the_aaaa_path_is_dead() {
    let dead = unanswered_address();
    let dead_address = dead.address;
    let working = answering_server();
    let resolver = dns::Resolver::new(move |_, _, family| match family {
        dns::Family::V6 => Ok(vec![dead_address]),
        dns::Family::V4 => {
            std::thread::sleep(Duration::from_millis(100));
            Ok(vec![working])
        }
    });
    let agent = agent_builder_with_resolver(resolver)
        .timeout(Duration::from_secs(20))
        .build();

    let body = agent
        .get("http://late-a.invalid/relay/token")
        .call()
        .unwrap()
        .into_string()
        .unwrap();

    assert_eq!(body, "ok");
}

#[test]
fn http_connect_keeps_a_shorter_agent_deadline() {
    assert_short_deadline(false);
}

#[test]
fn http_connect_keeps_a_shorter_request_deadline() {
    assert_short_deadline(true);
}

fn assert_short_deadline(request_timeout: bool) {
    let dead = unanswered_address();
    {
        let builder = agent_builder();
        let agent = if request_timeout {
            builder
        } else {
            builder.timeout(Duration::from_millis(100))
        }
        .build();
        let request = agent.get(&format!("http://{}/", dead.address));
        let request = if request_timeout {
            request.timeout(Duration::from_millis(100))
        } else {
            request
        };
        let started = Instant::now();
        assert!(request.call().is_err());
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "short request held for {:?}",
            started.elapsed()
        );
    }
}

#[test]
fn http_call_reaches_a_late_aaaa_answer_after_the_stagger() {
    assert_http_reaches_late_family(dns::Family::V6, 150);
}

#[test]
fn http_call_reaches_an_a_answer_after_the_previous_wait() {
    assert_http_reaches_late_family(dns::Family::V4, 1200);
}

fn assert_http_reaches_late_family(late_family: dns::Family, delay_ms: u64) {
    let dead = unanswered_address_on(late_family == dns::Family::V4);
    let dead_address = dead.address;
    let working = answering_server_on(late_family == dns::Family::V6);
    let resolver = dns::Resolver::new(move |_, _, family| {
        if family == late_family {
            std::thread::sleep(Duration::from_millis(delay_ms));
            Ok(vec![working])
        } else {
            Ok(vec![dead_address])
        }
    });
    let agent = agent_builder_with_resolver(resolver)
        .timeout(Duration::from_secs(4))
        .build();
    let started = Instant::now();
    let result = agent
        .get(&format!("http://late-{late_family:?}.invalid/relay/token"))
        .call();
    assert!(
        result.is_ok(),
        "late {late_family:?} answer lost after {:?}: {result:?}",
        started.elapsed()
    );
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn websocket_reaches_a_late_aaaa_answer_after_the_stagger() {
    assert_websocket_reaches_late_family(dns::Family::V6, 150).await;
}

#[tokio::test]
async fn websocket_reaches_an_a_answer_after_the_previous_wait() {
    assert_websocket_reaches_late_family(dns::Family::V4, 1200).await;
}

async fn assert_websocket_reaches_late_family(late_family: dns::Family, delay_ms: u64) {
    let dead = unanswered_address_on(late_family == dns::Family::V4);
    let dead_address = dead.address;
    let listener = tokio::net::TcpListener::bind(if late_family == dns::Family::V6 {
        "[::1]:0"
    } else {
        "127.0.0.1:0"
    })
    .await
    .unwrap();
    let working = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        tokio_tungstenite::accept_async(stream).await.unwrap()
    });
    let resolver = dns::Resolver::new(move |_, _, family| {
        if family == late_family {
            std::thread::sleep(Duration::from_millis(delay_ms));
            Ok(vec![working])
        } else {
            Ok(vec![dead_address])
        }
    });
    let started = Instant::now();
    let result =
        connect_websocket_with_resolver(format!("ws://late-ws-{late_family:?}.invalid/"), resolver)
            .await;
    assert!(
        result.is_ok(),
        "late {late_family:?} answer lost after {:?}: {result:?}",
        started.elapsed()
    );
    assert!(started.elapsed() < Duration::from_secs(3));
    drop(result);
    drop(server.await.unwrap());
}

#[test]
fn http_connect_keeps_the_deadline_after_resolution() {
    let dead = unanswered_address();
    let address = dead.address;
    let agent = agent_builder_with_resolver(move |_: &str| {
        std::thread::sleep(Duration::from_millis(80));
        Ok(vec![address])
    })
    .timeout(Duration::from_millis(100))
    .build();
    let started = Instant::now();
    assert!(agent.get("http://slow-resolution.invalid/").call().is_err());
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "resolution reset the connect deadline: {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn websocket_keeps_connecting_while_a_syn_is_retried() {
    let delayed = unanswered_address();
    let queued = delayed._queued.len();
    let listener = delayed._listener.try_clone().unwrap();
    listener.set_nonblocking(true).unwrap();
    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    let server = tokio::spawn(async move {
        sleep(Duration::from_millis(1200)).await;
        for _ in 0..queued {
            drop(listener.accept().await.unwrap());
        }
        let (stream, _) = listener.accept().await.unwrap();
        tokio_tungstenite::accept_async(stream).await.unwrap()
    });
    let started = Instant::now();
    let result = connect_websocket(format!("ws://{}/", delayed.address)).await;
    assert!(
        result.is_ok(),
        "a single reachable path lost its TCP retry: {result:?}"
    );
    assert!(started.elapsed() >= Duration::from_secs(1));
    assert!(started.elapsed() < SOCKET_CONNECT_TIMEOUT);
    drop(result);
    drop(server.await.unwrap());
}
