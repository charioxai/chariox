use crate::{
    bindings::chariox::app::{app_types::AppError, http},
    broker::{context, number, text},
    host::{app_error, Host},
    ipc::{error, Ipc},
    scope::Scope,
    streams::{Bytes, Upload, CHUNK_BYTES, READ_AHEAD},
};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use tokio::sync::{mpsc, watch};
use wasmtime::component::{Accessor, HasSelf, StreamReader};

// The response owns a fetch's lifetime, independently of its invocation.
// Dropping it must also stop an upload waiting for a broker acknowledgement.
struct CancelUpload(watch::Sender<bool>);
impl Drop for CancelUpload {
    fn drop(&mut self) {
        self.0.send_replace(true);
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
fn protocol() -> AppError {
    app_error(error("APP_BROKER_PROTOCOL", "invalid HTTP broker response"))
}
async fn read(stream: &Stream, scope: &Scope) -> Result<Option<Vec<u8>>, AppError> {
    loop {
        let value = stream
            .ipc
            .call(scope, "http.read", json!({"streamId":stream.id}))
            .await
            .map_err(app_error)?;
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
        if bytes.len() > CHUNK_BYTES {
            return Err(protocol());
        }
        if bytes.is_empty() {
            return Err(protocol());
        }
        return Ok(Some(bytes));
    }
}
impl http::Host for Host {}
impl<T: Send + 'static> http::HostWithStore<T> for HasSelf<Host> {
    async fn fetch(
        accessor: &Accessor<T, Self>,
        request: http::Request,
        body: Option<StreamReader<u8>>,
    ) -> Result<(http::ResponseHead, StreamReader<u8>), AppError> {
        let (ipc, scope) = context(accessor)?;
        let slot = accessor
            .with(|mut access| access.get().stream_slots.clone().try_acquire_owned())
            .map_err(|_| app_error(error("APP_RESOURCE_LIMIT", "too many open HTTP streams")))?;
        let mut params = json!({"url":request.url,"method":request.method,"headers":request.headers,"hasBody":body.is_some()});
        if let Some(value) = request.connection_id {
            params["connectionId"] = value.into();
        }
        if let Some(value) = request.operation_id {
            params["operationId"] = value.into();
        }
        let opened = ipc
            .call(&scope, "http.open", params)
            .await
            .map_err(app_error)?;
        let stream = Arc::new(Stream {
            ipc,
            id: text(&opened, "streamId")?,
            _slot: slot,
        });
        let (cancel_upload, mut upload_cancelled) = watch::channel(false);
        let cancel_upload = CancelUpload(cancel_upload);
        let (upload_result, mut upload_status) = mpsc::channel(1);
        if let Some(body) = body {
            let (chunks, mut receiver) = mpsc::channel::<Vec<u8>>(1);
            accessor
                .with(|mut access| body.pipe(&mut access, Upload::new(chunks, scope.clone())))
                .map_err(|_| protocol())?;
            let stream = stream.clone();
            let scope = scope.clone();
            tokio::spawn(async move {
                let upload = async {
                    while let Some(chunk) = receiver.recv().await {
                        stream.ipc.call(&scope,"http.write",json!({"streamId":stream.id,"bodyBase64":B64.encode(chunk),"end":false})).await?;
                    }
                    stream
                        .ipc
                        .call(
                            &scope,
                            "http.write",
                            json!({"streamId":stream.id,"bodyBase64":"","end":true}),
                        )
                        .await?;
                    Ok::<_, chariox_app_runtime::wire::RemoteError>(())
                };
                tokio::select! {
                    biased;
                    _=scope.cancelled()=>{},
                    _=upload_cancelled.changed()=>{},
                    result=upload=>{ let _=upload_result.send(result).await; },
                }
            });
        } else {
            drop(upload_result);
        }
        let head = loop {
            let value = stream
                .ipc
                .call(&scope, "http.headers", json!({"streamId":stream.id}))
                .await
                .map_err(app_error)?;
            if let Ok(Err(error)) = upload_status.try_recv() {
                return Err(app_error(error));
            }
            if value["pending"] == true {
                tokio::time::sleep(Duration::from_millis(10)).await;
                continue;
            }
            break http::ResponseHead {
                status: number(&value, "status")?
                    .try_into()
                    .map_err(|_| protocol())?,
                headers: serde_json::from_value(value["headers"].clone())
                    .map_err(|_| protocol())?,
                url: text(&value, "url")?,
            };
        };
        let (sender, receiver) = mpsc::channel(READ_AHEAD);
        let reading_scope = scope.clone();
        tokio::spawn(async move {
            let _cancel_upload = cancel_upload;
            loop {
                let next = async {
                    let permit = sender.reserve().await.map_err(|_| ())?;
                    match read(&stream, &reading_scope).await {
                        Ok(Some(bytes)) => {
                            permit.send(Ok(bytes));
                            Ok::<bool, ()>(true)
                        }
                        Ok(None) => Ok(false),
                        Err(error) => {
                            permit.send(Err(format!("{}: {}", error.code, error.message)));
                            Ok(false)
                        }
                    }
                };
                let keep = tokio::select! {
                    biased;
                    _=reading_scope.cancelled()=>false,
                    _=sender.closed()=>false,
                    result=next=>result.unwrap_or(false),
                };
                if !keep {
                    break;
                }
            }
        });
        let reader = accessor
            .with(|mut access| StreamReader::new(&mut access, Bytes::new(receiver, scope)))
            .map_err(|_| protocol())?;
        Ok((head, reader))
    }
}
