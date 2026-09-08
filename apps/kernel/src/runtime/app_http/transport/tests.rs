//! Real loopback HTTP sockets exercise the production codec only. The test's
//! fixed target is not a production connection capability; public-IP checking
//! is independently mandatory in HttpTransport::perform before exchange_io.
use super::*;
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
