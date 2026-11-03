use crate::{
    bindings::chariox::app::{app_types::AppError, files},
    broker::{context, number, text},
    host::{app_error, Host},
    ipc::error,
    streams::Upload,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde_json::json;
use tokio::sync::mpsc;
use wasmtime::component::{Accessor, HasSelf, StreamReader};
fn protocol() -> AppError {
    app_error(error("APP_BROKER_PROTOCOL", "invalid file broker response"))
}
impl files::Host for Host {}
impl<T: Send + 'static> files::HostWithStore<T> for HasSelf<Host> {
    async fn atomic_replace(
        accessor: &Accessor<T, Self>,
        path: String,
        contents: StreamReader<u8>,
    ) -> Result<u64, AppError> {
        let (ipc, scope) = context(accessor)?;
        let (sender, mut receiver) = mpsc::channel(1);
        accessor
            .with(|mut access| contents.pipe(&mut access, Upload::new(sender, scope.clone())))
            .map_err(|_| protocol())?;
        let mut bytes = Vec::new();
        loop {
            let chunk = tokio::select! {
                biased;
                reason=scope.cancelled()=>return Err(app_error(error(reason,"file write cancelled"))),
                chunk=receiver.recv()=>chunk,
            };
            let Some(chunk) = chunk else {
                break;
            };
            if bytes.len() + chunk.len() > 512 * 1024 {
                return Err(app_error(error(
                    "APP_FILE_LIMIT",
                    "atomic replace exceeds the 512 KiB broker frame limit",
                )));
            }
            bytes.extend_from_slice(&chunk);
        }
        let value = ipc
            .call(
                &scope,
                "files.atomic_replace",
                json!({"path":path,"contentsBase64":B64.encode(bytes)}),
            )
            .await
            .map_err(app_error)?;
        number(&value, "bytesWritten")
    }
    async fn snapshot(
        accessor: &Accessor<T, Self>,
        name: String,
        consistency: files::Consistency,
    ) -> Result<files::SnapshotInfo, AppError> {
        let (ipc, scope) = context(accessor)?;
        let consistency_text = match consistency {
            files::Consistency::Quiescent => "quiescent",
            files::Consistency::CrashConsistent => "crash_consistent",
        };
        let value = ipc
            .call(
                &scope,
                "files.snapshot",
                json!({"name":name,"consistency":consistency_text}),
            )
            .await
            .map_err(app_error)?;
        Ok(files::SnapshotInfo {
            snapshot_id: text(&value, "snapshotId")?,
            consistency,
            files: number(&value, "files")?,
            bytes: number(&value, "bytes")?,
        })
    }
    async fn import(
        accessor: &Accessor<T, Self>,
        grant_id: String,
        destination: String,
    ) -> Result<(u64, String), AppError> {
        let (ipc, scope) = context(accessor)?;
        let value = ipc
            .call(
                &scope,
                "files.import",
                json!({"grantId":grant_id,"destination":destination}),
            )
            .await
            .map_err(app_error)?;
        Ok((number(&value, "bytesWritten")?, text(&value, "name")?))
    }
    async fn export(accessor: &Accessor<T, Self>, path: String) -> Result<String, AppError> {
        let (ipc, scope) = context(accessor)?;
        let value = ipc
            .call(&scope, "files.export", json!({"path":path}))
            .await
            .map_err(app_error)?;
        text(&value, "operationId")
    }
}
