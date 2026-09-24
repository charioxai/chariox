//! One HTTP/1 exchange over a directly owned socket. There is no connection
//! pool, environment proxy, ambient cookie jar, or implicit redirect/retry.
mod body;
mod progress;

use super::{
    dns::DnsConfig, limits::LifetimeLease, policy::ApprovedTarget, HttpError, Result, CHUNK_BYTES,
    MAX_RESPONSE_BYTES, NETWORK_INACTIVITY, STREAM_LIFETIME,
};
use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::{
    body::{Body, Incoming},
    client::conn::http1,
    header, Request,
};
use hyper_util::rt::TokioIo;
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
    sync::{mpsc, oneshot, watch},
    time::Instant,
};
use tokio_rustls::{
    rustls::{self, pki_types::ServerName},
    TlsConnector,
};

pub(super) use body::{UploadBody, UploadPort};

/// Metadata is bounded and contains no Set-Cookie or proxy-authentication data.
#[derive(Debug, Clone)]
pub(super) struct ResponseHead {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub url: String,
}
pub(super) struct ReceivePort {
    pub headers: oneshot::Receiver<Result<ResponseHead>>,
    pub chunks: mpsc::Receiver<Result<Bytes>>,
}
pub(super) struct Exchange {
    upload: UploadBody,
    headers: Option<oneshot::Sender<Result<ResponseHead>>>,
    chunks: mpsc::Sender<Result<Bytes>>,
}
#[cfg(test)]
impl Exchange {
    pub(super) fn fixture_parts(
        self,
    ) -> (
        UploadBody,
        oneshot::Sender<Result<ResponseHead>>,
        mpsc::Sender<Result<Bytes>>,
    ) {
        (self.upload, self.headers.unwrap(), self.chunks)
    }
}
pub(super) fn channels(has_body: bool) -> (UploadPort, ReceivePort, Exchange) {
    let (upload, body) = body::channel(has_body);
    let (send_head, headers) = oneshot::channel();
    let (send_chunks, chunks) = mpsc::channel(2);
    (
        upload,
        ReceivePort { headers, chunks },
        Exchange {
            upload: body,
            headers: Some(send_head),
            chunks: send_chunks,
        },
    )
}

pub(super) struct HttpTransport {
    dns: DnsConfig,
    tls: TlsConnector,
}
impl HttpTransport {
    pub(super) fn system() -> Result<Self> {
        let roots =
            rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let mut config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|_| HttpError::Tls)?
        .with_root_certificates(roots)
        .with_no_client_auth();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(Self {
            dns: DnsConfig::system()?,
            tls: TlsConnector::from(Arc::new(config)),
        })
    }

    /// The supervisor retains the task until completion and supplies the one
    /// kernel pool reservation. Only enqueue this after the existing writer's
    /// exact current installation/publisher approval fence succeeds. The lease
    /// grants capacity, not connection credentials or operation approval.
    pub(super) async fn run(
        &self,
        target: ApprovedTarget,
        mut exchange: Exchange,
        mut stopped: watch::Receiver<bool>,
        lease: LifetimeLease,
        admitted: Instant,
    ) -> Result<()> {
        let result = self
            .perform(&target, &mut exchange, &mut stopped, &lease, admitted)
            .await;
        if let Err(error) = result {
            if let Some(headers) = exchange.headers.take() {
                let _ = headers.send(Err(error));
            }
            // A blocked reader cannot keep a failed socket open. Error detail
            // is also retained by the task result; EOF is interpreted with it.
            let _ = exchange.chunks.try_send(Err(error));
        }
        // All direct socket futures have dropped. Any DNS panic fallback still
        // owns a clone until its aborted driver actually drops its socket.
        drop(lease);
        result
    }

    async fn perform(
        &self,
        target: &ApprovedTarget,
        exchange: &mut Exchange,
        stopped: &mut watch::Receiver<bool>,
        lease: &LifetimeLease,
        admitted: Instant,
    ) -> Result<()> {
        let initial_deadline = admitted + Duration::from_secs(30);
        let lifetime = admitted + STREAM_LIFETIME;
        let addresses = self
            .dns
            .resolve(target, initial_deadline, stopped.clone(), lease.clone())
            .await?;
        let mut connected = None;
        for selected in addresses {
            if super::stopped(stopped) {
                return Err(HttpError::Cancelled);
            }
            let attempt = initial_deadline.min(Instant::now() + Duration::from_secs(5));
            let socket = tokio::select! {
                biased;
                _ = super::cancelled(stopped) => return Err(HttpError::Cancelled),
                _ = tokio::time::sleep_until(attempt) => None,
                result = TcpStream::connect(selected) => result.ok(),
            };
            if let Some(socket) = socket {
                // This is the actual connected endpoint, before TLS/HTTP bytes.
                target.require_connected(
                    selected,
                    socket.peer_addr().map_err(|_| HttpError::Network)?,
                )?;
                socket.set_nodelay(true).map_err(|_| HttpError::Network)?;
                connected = Some(socket);
                break;
            }
            if Instant::now() >= initial_deadline {
                return Err(HttpError::Deadline);
            }
        }
        let socket = connected.ok_or(HttpError::Network)?;
        let progress = progress::Progress::new();
        let socket = progress::ObservedIo::new(socket, progress.clone());
        if target.url().scheme() == "https" {
            let name = match target.url().host().ok_or(HttpError::Invalid)? {
                url::Host::Domain(name) => {
                    ServerName::try_from(name.to_owned()).map_err(|_| HttpError::Invalid)?
                }
                url::Host::Ipv4(ip) => ServerName::IpAddress(ip.into()),
                url::Host::Ipv6(ip) => ServerName::IpAddress(ip.into()),
            };
            let tls = tokio::select! {
                biased;
                _ = super::cancelled(stopped) => return Err(HttpError::Cancelled),
                _ = tokio::time::sleep_until(initial_deadline) => return Err(HttpError::Deadline),
                result = self.tls.connect(name, socket) => result.map_err(|_| HttpError::Tls)?,
            };
            exchange_io(tls, target, exchange, stopped, progress, lifetime).await
        } else {
            exchange_io(socket, target, exchange, stopped, progress, lifetime).await
        }
    }
}

async fn exchange_io<I: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
    io: I,
    target: &ApprovedTarget,
    exchange: &mut Exchange,
    stopped: &mut watch::Receiver<bool>,
    progress: progress::Progress,
    lifetime: Instant,
) -> Result<()> {
    let mut builder = http1::Builder::new();
    builder
        .max_headers(64)
        .max_buf_size(CHUNK_BYTES)
        .writev(false);
    let (sender, connection) = builder
        .handshake::<_, UploadBody>(TokioIo::new(io))
        .await
        .map_err(|_| HttpError::Network)?;
    let operation = exchange.perform(sender, target);
    tokio::pin!(operation, connection);
    let mut connection_done = false;
    loop {
        tokio::select! {
            biased;
            _ = super::cancelled(stopped) => return Err(HttpError::Cancelled),
            _ = tokio::time::sleep_until(lifetime) => return Err(HttpError::Deadline),
            _ = progress.inactive(NETWORK_INACTIVITY) => return Err(HttpError::Deadline),
            result = &mut operation => return result,
            result = &mut connection, if !connection_done => {
                result.map_err(|_| HttpError::Network)?;
                connection_done = true;
            }
        }
    }
}

impl Exchange {
    async fn perform(
        &mut self,
        mut sender: http1::SendRequest<UploadBody>,
        target: &ApprovedTarget,
    ) -> Result<()> {
        if matches!(*target.method(), hyper::Method::GET | hyper::Method::HEAD)
            && !self.upload.is_end_stream()
        {
            return Err(HttpError::Invalid);
        }
        let mut request = Request::builder()
            .method(target.method().clone())
            .uri(&target.url()[url::Position::BeforePath..url::Position::AfterQuery])
            .body(std::mem::replace(&mut self.upload, UploadBody::empty()))
            .map_err(|_| HttpError::Invalid)?;
        *request.headers_mut() = target.headers().clone();
        request.headers_mut().insert(
            header::HOST,
            hyper::header::HeaderValue::from_str(
                &target.url()[url::Position::BeforeHost..url::Position::AfterPort],
            )
            .map_err(|_| HttpError::Invalid)?,
        );
        request.headers_mut().insert(
            header::ACCEPT_ENCODING,
            hyper::header::HeaderValue::from_static("identity"),
        );
        request.headers_mut().insert(
            header::CONNECTION,
            hyper::header::HeaderValue::from_static("close"),
        );
        let response = sender
            .send_request(request)
            .await
            .map_err(|_| HttpError::Network)?;
        if response.status().as_u16() == 101 {
            return Err(HttpError::Invalid);
        }
        if response
            .body()
            .size_hint()
            .upper()
            .is_some_and(|bytes| bytes > MAX_RESPONSE_BYTES)
        {
            return Err(HttpError::Limit);
        }
        let mut headers = Vec::new();
        let mut count = 0usize;
        for (name, value) in response.headers() {
            count = count
                .checked_add(name.as_str().len())
                .and_then(|n| n.checked_add(value.as_bytes().len()))
                .ok_or(HttpError::Limit)?;
            if count > 16 * 1024 {
                return Err(HttpError::Limit);
            }
            if matches!(
                name.as_str(),
                "set-cookie" | "proxy-authenticate" | "proxy-authorization"
            ) {
                continue;
            }
            // Header values are Latin-1 bytes, not arbitrary UTF-8. Preserve
            // their byte values in the bounded SDK string representation.
            headers.push((
                name.to_string(),
                value.as_bytes().iter().map(|b| char::from(*b)).collect(),
            ));
        }
        let head = ResponseHead {
            status: response.status().as_u16(),
            headers,
            url: target.url().to_string(),
        };
        self.headers
            .take()
            .ok_or(HttpError::Invalid)?
            .send(Ok(head))
            .map_err(|_| HttpError::Cancelled)?;
        self.download(response.into_body()).await
    }
    async fn download(&self, mut body: Incoming) -> Result<()> {
        let mut received = 0u64;
        while let Some(frame) = body.frame().await {
            let frame = frame.map_err(|_| HttpError::Network)?;
            if let Ok(mut bytes) = frame.into_data() {
                received = received
                    .checked_add(bytes.len() as u64)
                    .ok_or(HttpError::Limit)?;
                if received > MAX_RESPONSE_BYTES {
                    return Err(HttpError::Limit);
                }
                while !bytes.is_empty() {
                    let next = bytes.split_to(bytes.len().min(CHUNK_BYTES));
                    self.chunks
                        .send(Ok(next))
                        .await
                        .map_err(|_| HttpError::Cancelled)?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
