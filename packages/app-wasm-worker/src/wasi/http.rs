//! Both WASI versions share the kernel's capability-checked HTTP protocol.
//! The upstream socket/TLS request implementation is disabled at compile time.
use crate::{
    attribution::Attribution,
    ipc::{error, Ipc},
    scope::Scope,
    streams::{CHUNK_BYTES, READ_AHEAD},
};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use bytes::Bytes;
use http_body::{Body, Frame};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::{mpsc, watch};
use wasmtime_wasi_http::{Error, RequestOptions, WasiBody, WasiHttpHooks};

type Processing = Box<dyn Future<Output = Result<(), Error>> + Send>;
type Response = (http::Response<WasiBody>, Processing);
pub(super) struct Broker {
    ipc: Ipc,
    attribution: Attribution,
    slots: Arc<tokio::sync::Semaphore>,
}
impl Broker {
    pub(super) fn new(
        ipc: Ipc,
        attribution: Attribution,
        slots: Arc<tokio::sync::Semaphore>,
    ) -> Self {
        Self {
            ipc,
            attribution,
            slots,
        }
    }
}
impl WasiHttpHooks for Broker {
    fn set_host_header(&mut self) -> bool {
        // The kernel rejects caller-supplied Host and derives authority from
        // the approved URL when it opens the actual HTTP connection.
        false
    }
    fn send_request(
        &mut self,
        request: http::Request<WasiBody>,
        options: Option<RequestOptions>,
        processing: Processing,
    ) -> Box<dyn Future<Output = Result<Response, Error>> + Send> {
        let ipc = self.ipc.clone();
        let scope = self.attribution.current();
        let slot = self.slots.clone().try_acquire_owned();
        Box::new(async move {
            let scope = scope.ok_or(Error::HttpRequestDenied)?;
            let slot = slot.map_err(|_| Error::ConnectionLimitReached)?;
            if options.is_some_and(|options| options.connect_timeout.is_some()) {
                return Err(Error::ConfigurationError);
            }
            tokio::select! {
                biased;
                _=scope.cancelled()=>Err(Error::ConnectionTerminated),
                _=tokio::time::sleep_until(scope.deadline())=>Err(Error::HttpResponseTimeout),
                result=fetch(ipc,scope.clone(),slot,request,options.unwrap_or_default(),processing)=>result,
            }
        })
    }
    fn p2_outgoing_body_chunk_size(&mut self) -> usize {
        CHUNK_BYTES
    }
    fn p3_outgoing_body_chunk_size(&mut self) -> usize {
        CHUNK_BYTES
    }
}
fn remote(error: chariox_app_runtime::wire::RemoteError) -> Error {
    match error.code.as_str() {
        "APP_HTTP_DESTINATION_DENIED"
        | "APP_HTTP_CAPABILITY_DENIED"
        | "APP_CALL_CONTEXT"
        | "APP_HTTP_CONNECTION_UNSUPPORTED"
        | "APP_HTTP_EFFECT_UNSUPPORTED"
        | "VALIDATION_REQUIRED" => Error::HttpRequestDenied,
        "APP_BUSY" | "APP_RESOURCE_LIMIT" | "APP_HTTP_LIMIT" => Error::ConnectionLimitReached,
        "CANCELLED" | "APP_CALL_FINISHED" | "APP_INSTANCE_RESTARTED" => Error::ConnectionTerminated,
        "DEADLINE_EXCEEDED" | "APP_DEADLINE" => Error::HttpResponseTimeout,
        "APP_HTTP_TLS" => Error::TlsProtocolError,
        _ => Error::InternalError(Some(error.code)),
    }
}
fn protocol() -> Error {
    Error::InternalError(Some("APP_BROKER_PROTOCOL".into()))
}
fn text(value: &Value, key: &str) -> Result<String, Error> {
    value[key].as_str().map(String::from).ok_or_else(protocol)
}
async fn bounded<T>(
    limit: Option<Duration>,
    future: impl Future<Output = Result<T, Error>>,
) -> Result<T, Error> {
    match limit {
        Some(limit) => tokio::time::timeout(limit, future)
            .await
            .map_err(|_| Error::HttpResponseTimeout)?,
        None => future.await,
    }
}
struct Stream {
    ipc: Ipc,
    id: String,
    _slot: tokio::sync::OwnedSemaphorePermit,
}
impl Drop for Stream {
    fn drop(&mut self) {
        self.ipc.cleanup("http.cancel", json!({"streamId":self.id}));
    }
}
struct Cancel(watch::Sender<bool>);
impl Drop for Cancel {
    fn drop(&mut self) {
        self.0.send_replace(true);
    }
}
struct ResponseBody {
    chunks: mpsc::Receiver<Result<Frame<Bytes>, Error>>,
    _cancel: Cancel,
}
impl Body for ResponseBody {
    type Data = Bytes;
    type Error = Error;
    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Error>>> {
        self.get_mut().chunks.poll_recv(cx)
    }
}
async fn read(stream: &Stream, scope: &Scope) -> Result<Option<Frame<Bytes>>, Error> {
    loop {
        let value = stream
            .ipc
            .call(scope, "http.read", json!({"streamId":stream.id}))
            .await
            .map_err(remote)?;
        if value["pending"] == true {
            tokio::time::sleep(Duration::from_millis(10)).await;
            continue;
        }
        if value["done"] == true {
            return Ok(None);
        }
        let encoded = text(&value, "chunkBase64")?;
        if encoded.len() > CHUNK_BYTES.div_ceil(3) * 4 {
            return Err(protocol());
        }
        let bytes = B64.decode(encoded).map_err(|_| protocol())?;
        if bytes.is_empty() || bytes.len() > CHUNK_BYTES {
            return Err(protocol());
        }
        return Ok(Some(Frame::data(bytes.into())));
    }
}
async fn fetch(
    ipc: Ipc,
    scope: Scope,
    slot: tokio::sync::OwnedSemaphorePermit,
    request: http::Request<WasiBody>,
    options: RequestOptions,
    processing: Processing,
) -> Result<Response, Error> {
    let (parts, mut body) = request.into_parts();
    let has_body = if matches!(parts.method, http::Method::GET | http::Method::HEAD) {
        // WASIp3's empty body can still have a pending trailers future, so
        // is_end_stream alone would turn ordinary GETs into forbidden uploads.
        while let Some(frame) = body.frame().await {
            let frame = frame?;
            if frame.data_ref().is_some_and(|bytes| !bytes.is_empty())
                || frame
                    .trailers_ref()
                    .is_some_and(|headers| !headers.is_empty())
            {
                return Err(Error::HttpRequestDenied);
            }
        }
        false
    } else {
        !body.is_end_stream()
    };
    let headers: Vec<_> = parts
        .headers
        .iter()
        .map(|(name, value)| {
            Ok((
                name.as_str().to_owned(),
                value
                    .to_str()
                    .map_err(|_| Error::HttpRequestDenied)?
                    .to_owned(),
            ))
        })
        .collect::<Result<_, Error>>()?;
    let params = json!({"url":parts.uri.to_string(),"method":parts.method.as_str(),"headers":headers,"hasBody":has_body});
    let opened = ipc
        .call(&scope, "http.open", params)
        .await
        .map_err(remote)?;
    let stream = Arc::new(Stream {
        ipc,
        id: text(&opened, "streamId")?,
        _slot: slot,
    });
    let (cancel, mut cancelled_upload) = watch::channel(false);
    let mut cancelled_read = cancelled_upload.clone();
    let cancel = Cancel(cancel);
    let (upload_result, mut upload_status) = mpsc::channel(1);
    let uploading = stream.clone();
    let upload_scope = scope.clone();
    tokio::spawn(async move {
        let upload = async {
            if has_body {
                while let Some(frame) = body.frame().await {
                    let frame = frame?;
                    let data = match frame.into_data() {
                        Ok(data) => data,
                        Err(frame) => {
                            let trailers = frame
                                .into_trailers()
                                .map_err(|_| Error::HttpProtocolError)?;
                            if !trailers.is_empty() {
                                return Err(Error::HttpRequestDenied);
                            }
                            continue;
                        }
                    };
                    for chunk in data.chunks(CHUNK_BYTES) {
                        uploading.ipc.call(&upload_scope,"http.write",json!({"streamId":uploading.id,"bodyBase64":B64.encode(chunk),"end":false})).await.map_err(remote)?;
                    }
                }
                uploading
                    .ipc
                    .call(
                        &upload_scope,
                        "http.write",
                        json!({"streamId":uploading.id,"bodyBase64":"","end":true}),
                    )
                    .await
                    .map_err(remote)?;
            }
            Ok(())
        };
        let result = tokio::select! {
            biased;
            _=upload_scope.cancelled()=>Err(Error::ConnectionTerminated),
            _=cancelled_upload.changed()=>Err(Error::ConnectionTerminated),
            result=upload=>result,
        };
        let _ = upload_result.send(result).await;
    });
    let mut uploaded = false;
    let head = bounded(options.first_byte_timeout, async {
        loop {
            let headers=stream.ipc.call(&scope,"http.headers",json!({"streamId":stream.id}));
            tokio::pin!(headers);
            let value=loop {
                tokio::select! {
                    biased;
                    result=upload_status.recv(),if !uploaded=>{
                        result.ok_or_else(|| remote(error("APP_HTTP_STREAM_FAILED","upload stopped")))??;
                        uploaded=true;
                    },
                    result=&mut headers=>break result.map_err(remote)?,
                }
            };
            if value["pending"] == true {
                tokio::time::sleep(Duration::from_millis(10)).await;
                continue;
            }
            let status = value["status"]
                .as_u64()
                .and_then(|s| u16::try_from(s).ok())
                .ok_or_else(protocol)?;
            let headers: Vec<(String, String)> =
                serde_json::from_value(value["headers"].clone()).map_err(|_| protocol())?;
            let mut response = http::Response::builder().status(status);
            for (name, value) in headers {
                response = response.header(name, value);
            }
            break Ok(response);
        }
    })
    .await?;
    let (sender, receiver) = mpsc::channel(READ_AHEAD);
    let reading_scope = scope.clone();
    tokio::spawn(async move {
        loop {
            let next = async {
                let permit = sender.reserve().await.map_err(|_| ())?;
                match bounded(options.between_bytes_timeout, read(&stream, &reading_scope)).await {
                    Ok(Some(frame)) => {
                        permit.send(Ok(frame));
                        Ok::<bool, ()>(true)
                    }
                    Ok(None) => Ok(false),
                    Err(error) => {
                        permit.send(Err(error));
                        Ok(false)
                    }
                }
            };
            let keep = tokio::select! {
                biased;
                _=reading_scope.cancelled()=>false,
                _=cancelled_read.changed()=>false,
                _=sender.closed()=>false,
                result=next=>result.unwrap_or(false),
            };
            if !keep {
                break;
            }
        }
    });
    let body = ResponseBody {
        chunks: receiver,
        _cancel: cancel,
    }
    .boxed_unsync();
    let response = head.body(body).map_err(|_| protocol())?;
    let io: Processing = Box::new(async move {
        if !uploaded {
            upload_status
                .recv()
                .await
                .ok_or_else(|| remote(error("APP_HTTP_STREAM_FAILED", "upload stopped")))??;
        }
        Box::into_pin(processing).await
    });
    Ok((response, io))
}
