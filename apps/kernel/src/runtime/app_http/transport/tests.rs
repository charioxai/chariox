//! Real loopback HTTP sockets exercise the production codec only. The test's
//! fixed target is not a production connection capability; public-IP checking
//! is independently mandatory in HttpTransport::perform before exchange_io.
use super::*;
use crate::runtime::app_http::{
    dns::test_server::TestDns, limits::HttpLimits, policy::fixture_get,
};
use hickory_resolver::proto::rr::RecordType;
use std::sync::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn sockets() -> (TcpStream, TcpStream) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap());
    let (client, server) = tokio::join!(client, listener.accept());
    (client.unwrap(), server.unwrap().0)
}
async fn request_head(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        assert!(bytes.len() < 16 * 1024);
        bytes.push(stream.read_u8().await.unwrap());
    }
    String::from_utf8(bytes).unwrap()
}
async fn drive(
    io: TcpStream,
    mut exchange: Exchange,
    mut stop: watch::Receiver<bool>,
    lifetime: Duration,
) -> Result<()> {
    let progress = progress::Progress::new();
    let observed = progress::ObservedIo::new(io, progress.clone());
    exchange_io(
        observed,
        &super::super::policy::fixture_target(),
        &mut exchange,
        &mut stop,
        progress,
        Instant::now() + lifetime,
    )
    .await
}

#[tokio::test]
async fn sse_first_chunk_arrives_before_response_completion() {
    tokio::time::timeout(Duration::from_secs(3), async {
        let (client, mut server) = sockets().await;
        let (_upload, mut receive, exchange) = channels(false);
        let (_stop, signal) = watch::channel(false);
        let driver = tokio::spawn(drive(client, exchange, signal, Duration::from_secs(2)));
        let (continue_tx, continue_rx) = oneshot::channel();
        let server_task = tokio::spawn(async move {
            let head = request_head(&mut server).await;
            assert!(head.contains("host: api.example.com\r\n"));
            assert!(head.contains("accept-encoding: identity\r\n"));
            assert!(!head.contains("127.0.0.1"));
            server.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nSet-Cookie: hidden=secret\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\nC\r\ndata:first\n\n\r\n").await.unwrap();
            continue_rx.await.unwrap();
            server.write_all(b"D\r\ndata:second\n\n\r\n0\r\n\r\n").await.unwrap();
        });
        let head = receive.headers.await.unwrap().unwrap();
        assert_eq!(head.status, 200);
        assert_eq!(head.url, "http://api.example.com/fixture");
        assert!(!head.headers.iter().any(|(name, _)| name == "set-cookie"));
        assert_eq!(receive.chunks.recv().await.unwrap().unwrap(), "data:first\n\n");
        assert!(!driver.is_finished());
        continue_tx.send(()).unwrap();
        assert_eq!(receive.chunks.recv().await.unwrap().unwrap(), "data:second\n\n");
        assert!(receive.chunks.recv().await.is_none());
        driver.await.unwrap().unwrap(); server_task.await.unwrap();
    }).await.unwrap();
}

#[tokio::test]
async fn streaming_upload_preserves_multipart_bytes_and_midbody_progress() {
    tokio::time::timeout(Duration::from_secs(3), async {
        let (client, server) = sockets().await;
        let (mut upload, receive, exchange) = channels(true);
        let (_stop, signal) = watch::channel(false);
        let driver = tokio::spawn(drive(client, exchange, signal, Duration::from_secs(2)));
        let (first_tx, first_rx) = oneshot::channel();
        let first_tx = Arc::new(std::sync::Mutex::new(Some(first_tx)));
        let expected = b"--fixture\r\nContent-Disposition: form-data; name=\"x\"\r\n\r\nvalue\r\n--fixture--\r\n";
        let server_task = tokio::spawn(async move {
            let service = hyper::service::service_fn(move |mut request: Request<Incoming>| {
                let first_tx = first_tx.clone();
                async move {
                    assert_eq!(request.headers()[header::CONTENT_TYPE], "multipart/form-data; boundary=fixture");
                    let mut body = Vec::new();
                    while let Some(frame) = request.body_mut().frame().await {
                        if let Ok(bytes) = frame.unwrap().into_data() {
                            assert!(body.len() + bytes.len() <= 1024);
                            body.extend_from_slice(&bytes);
                            if let Some(first_tx) = first_tx.lock().unwrap().take() { first_tx.send(()).unwrap(); }
                        }
                    }
                    assert_eq!(body, expected);
                    Ok::<_, std::convert::Infallible>(hyper::Response::new(http_body_util::Empty::<Bytes>::new()))
                }
            });
            hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(server), service).await.unwrap();
        });
        upload.write(Bytes::copy_from_slice(&expected[..20]), false).await.unwrap();
        first_rx.await.unwrap(); // service has consumed data before final upload
        upload.write(Bytes::copy_from_slice(&expected[20..]), true).await.unwrap();
        assert_eq!(receive.headers.await.unwrap().unwrap().status, 200);
        driver.await.unwrap().unwrap(); server_task.await.unwrap();
    }).await.unwrap();
}

#[tokio::test]
async fn an_approved_effect_sends_exactly_the_kernel_body_and_the_app_cannot_add_bytes() {
    tokio::time::timeout(Duration::from_secs(3), async {
        let (client, server) = sockets().await;
        let (mut upload, receive, exchange) =
            fixed_channels(Bytes::from_static(br#"{"amount":5,"to":"x"}"#));
        assert!(matches!(
            upload.write(Bytes::from_static(b"more"), true).await,
            Err(HttpError::Invalid)
        ));
        let (_stop, signal) = watch::channel(false);
        let driver = tokio::spawn(drive(client, exchange, signal, Duration::from_secs(2)));
        let server_task = tokio::spawn(async move {
            let service = hyper::service::service_fn(|request: Request<Incoming>| async move {
                let body = request.into_body().collect().await.unwrap().to_bytes();
                assert_eq!(&body[..], br#"{"amount":5,"to":"x"}"#);
                Ok::<_, std::convert::Infallible>(hyper::Response::new(http_body_util::Empty::<
                    Bytes,
                >::new()))
            });
            hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(server), service)
                .await
                .unwrap();
        });
        assert_eq!(receive.headers.await.unwrap().unwrap().status, 200);
        driver.await.unwrap().unwrap();
        server_task.await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cancellation_closes_socket_with_backpressured_response() {
    tokio::time::timeout(Duration::from_secs(3), async {
        let (client, mut server) = sockets().await;
        let (_upload, receive, exchange) = channels(false);
        let (stop, signal) = watch::channel(false);
        let driver = tokio::spawn(drive(client, exchange, signal, Duration::from_secs(2)));
        request_head(&mut server).await;
        server
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 262144\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let sender = tokio::spawn(async move {
            let _ = server.write_all(&vec![b'x'; 4 * CHUNK_BYTES]).await;
            let mut byte = [0u8; 1];
            server.read(&mut byte).await
        });
        receive.headers.await.unwrap().unwrap();
        // Keep the chunks receiver alive without polling it: bounded output
        // must not prevent cancellation and physical socket closure.
        stop.send_replace(true);
        assert_eq!(driver.await.unwrap(), Err(HttpError::Cancelled));
        let closed = sender.await.unwrap();
        assert!(matches!(closed, Ok(0) | Err(_)));
        drop(receive.chunks);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn interrupted_upload_is_not_replayable_and_all_queues_are_bounded() {
    let (mut upload, _body) = body::channel(true);
    upload.write(Bytes::from_static(b"a"), false).await.unwrap();
    upload.write(Bytes::from_static(b"b"), false).await.unwrap();
    assert!(tokio::time::timeout(
        Duration::from_millis(20),
        upload.write(Bytes::from_static(b"c"), true)
    )
    .await
    .is_err());
    assert_eq!(
        upload.write(Bytes::from_static(b"retry"), true).await,
        Err(HttpError::Invalid)
    );
    let (mut upload, _body) = body::channel(true);
    assert_eq!(
        upload
            .write(Bytes::from(vec![0; CHUNK_BYTES + 1]), true)
            .await,
        Err(HttpError::Invalid)
    );
}

#[tokio::test]
async fn huge_declared_body_and_fixed_lifetime_are_rejected_without_buffering() {
    tokio::time::timeout(Duration::from_secs(3), async {
        let (client, mut server) = sockets().await;
        let (_upload, _receive, exchange) = channels(false);
        let (_stop, signal) = watch::channel(false);
        let driver = tokio::spawn(drive(client, exchange, signal, Duration::from_secs(2)));
        request_head(&mut server).await;
        server
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 268435457\r\n\r\n")
            .await
            .unwrap();
        assert_eq!(driver.await.unwrap(), Err(HttpError::Limit));
        let (client, mut server) = sockets().await;
        let (_upload, _receive, exchange) = channels(false);
        let (_stop, signal) = watch::channel(false);
        let driver = tokio::spawn(drive(client, exchange, signal, Duration::from_millis(30)));
        request_head(&mut server).await;
        assert_eq!(driver.await.unwrap(), Err(HttpError::Deadline));
        assert_eq!(
            server.read_u8().await.unwrap_err().kind(),
            std::io::ErrorKind::UnexpectedEof
        );
    })
    .await
    .unwrap();
}

/// One whole request through `HttpTransport::run`: a test name server, the
/// production address policy and peer check, and the given dialer.
async fn run_once(transport: &HttpTransport, url: &str) -> Result<()> {
    let limits = HttpLimits::default();
    let (_upload, _receive, exchange) = channels(false);
    let (_stop, stopped) = watch::channel(false);
    transport
        .run(
            fixture_get(url),
            exchange,
            stopped,
            limits.acquire("alice", "app").unwrap(),
            Instant::now(),
        )
        .await
}

#[tokio::test]
async fn a_rebinding_name_is_resolved_once_and_only_its_checked_address_is_dialed() {
    // Public on the first A query, loopback on every later one. A client that
    // resolved the name again to connect would reach the loopback answer.
    let dns = TestDns::start(|name, kind, sequence| {
        Some(match (name, kind) {
            ("rebind.test.", RecordType::A) if sequence == 0 => vec!["8.8.8.8".parse().unwrap()],
            ("rebind.test.", RecordType::A) => vec!["127.0.0.1".parse().unwrap()],
            _ => vec![],
        })
    })
    .await;
    let dialed = Arc::new(Mutex::new(Vec::new()));
    let seen = dialed.clone();
    let transport = HttpTransport::fixture(
        dns.config(),
        Arc::new(move |address: SocketAddr| -> Dialing {
            seen.lock().unwrap().push(address);
            Box::pin(async { Err(io::Error::from(io::ErrorKind::ConnectionRefused)) })
        }),
    );
    // The checked public answer is the only address dialed, and its failed
    // connection neither resolves the name again nor tries another address.
    assert_eq!(
        run_once(&transport, "https://rebind.test/").await,
        Err(HttpError::Network)
    );
    assert_eq!(
        *dialed.lock().unwrap(),
        vec!["8.8.8.8:443".parse::<SocketAddr>().unwrap()]
    );
    assert_eq!(dns.queries("rebind.test.", RecordType::A), 1);
    // The next request sees the rebound answer and is refused before any
    // socket exists.
    assert_eq!(
        run_once(&transport, "https://rebind.test/").await,
        Err(HttpError::Destination)
    );
    assert_eq!(dialed.lock().unwrap().len(), 1);
    assert_eq!(dns.queries("rebind.test.", RecordType::A), 2);
}

#[tokio::test]
async fn a_socket_whose_peer_is_not_the_checked_address_is_closed_before_tls() {
    // The dialer lands on a local listener instead of the checked public
    // address, as a hostile route or NAT would. The actual peer is refused
    // before the TLS ClientHello.
    let local = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let elsewhere = local.local_addr().unwrap();
    let dns = TestDns::start(|_, kind, _| {
        Some(match kind {
            RecordType::A => vec!["8.8.8.8".parse().unwrap()],
            _ => vec![],
        })
    })
    .await;
    let transport = HttpTransport::fixture(
        dns.config(),
        Arc::new(move |_: SocketAddr| -> Dialing { Box::pin(TcpStream::connect(elsewhere)) }),
    );
    let server = tokio::spawn(async move {
        let (mut socket, _) = local.accept().await.unwrap();
        let mut received = Vec::new();
        socket.read_to_end(&mut received).await.unwrap();
        received
    });
    assert_eq!(
        run_once(&transport, "https://pinned.test/").await,
        Err(HttpError::Destination)
    );
    let received = tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
    assert!(
        received.is_empty(),
        "{} bytes reached the wrong peer",
        received.len()
    );
}

/// The origin reads the whole request, then its connection ends with no
/// reply: a protected effect's outcome is unknown, an ordinary request's is a
/// plain network failure.
async fn reply_lost(effect: bool) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(3), async {
        let (client, mut server) = sockets().await;
        let body = Bytes::from_static(br#"{"amount":5,"to":"x"}"#);
        let (_upload, _receive, exchange) = if effect {
            fixed_channels(body.clone())
        } else {
            channels(false)
        };
        let (_stop, signal) = watch::channel(false);
        let driver = tokio::spawn(drive(client, exchange, signal, Duration::from_secs(2)));
        let head = request_head(&mut server).await;
        if effect {
            let mut received = vec![0; body.len()];
            server.read_exact(&mut received).await.unwrap();
            assert_eq!(received, body);
        }
        assert!(head.starts_with("POST /fixture"));
        drop(server);
        driver.await.unwrap()
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn an_effect_whose_reply_is_lost_after_sending_has_an_uncertain_outcome() {
    assert!(matches!(
        reply_lost(true).await,
        Err(HttpError::OutcomeUncertain)
    ));
    assert!(matches!(reply_lost(false).await, Err(HttpError::Network)));
}

#[tokio::test]
async fn an_effect_answered_by_a_gateway_error_is_a_response() {
    tokio::time::timeout(Duration::from_secs(3), async {
        let (client, mut server) = sockets().await;
        let (_upload, mut receive, exchange) =
            fixed_channels(Bytes::from_static(br#"{"amount":5,"to":"x"}"#));
        let (_stop, signal) = watch::channel(false);
        let driver = tokio::spawn(drive(client, exchange, signal, Duration::from_secs(2)));
        request_head(&mut server).await;
        server
            .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 15\r\nConnection: close\r\n\r\nerror code: 502")
            .await
            .unwrap();
        drop(server);
        assert_eq!(receive.headers.await.unwrap().unwrap().status, 502);
        assert_eq!(
            receive.chunks.recv().await.unwrap().unwrap(),
            "error code: 502"
        );
        driver.await.unwrap().unwrap();
    })
    .await
    .unwrap();
}

#[test]
fn only_an_effect_sent_and_unanswered_is_uncertain() {
    use std::sync::atomic::AtomicU8;
    for (effect, phase, expected) in [
        (true, UNSENT, HttpError::Deadline),
        (true, SENT, HttpError::OutcomeUncertain),
        (true, ANSWERED, HttpError::Deadline),
        (false, SENT, HttpError::Deadline),
    ] {
        assert_eq!(
            lost_reply(effect, &AtomicU8::new(phase), HttpError::Deadline),
            expected
        );
    }
}
