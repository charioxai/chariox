//! Folder bytes cross the existing bounded broker channel; no host filesystem
//! capability is linked into the guest.
use crate::{
    bindings::chariox::app::{app_types::AppError, folders},
    broker::{context, number, text},
    host::{app_error, Host},
    ipc::error,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde_json::json;
use wasmtime::component::{Accessor, HasSelf};

fn protocol() -> AppError {
    app_error(error(
        "APP_BROKER_PROTOCOL",
        "invalid folder broker response",
    ))
}
impl folders::Host for Host {}
impl<T: Send> folders::HostWithStore<T> for HasSelf<Host> {
    async fn list(
        accessor: &Accessor<T, Self>,
        grant_id: String,
        path: String,
    ) -> Result<Vec<folders::Entry>, AppError> {
        let (ipc, scope) = context(accessor)?;
        let value = ipc
            .call(
                &scope,
                "folders.list",
                json!({"grantId":grant_id,"path":path}),
            )
            .await
            .map_err(app_error)?;
        value
            .as_array()
            .ok_or_else(protocol)?
            .iter()
            .map(|entry| {
                Ok(folders::Entry {
                    name: text(entry, "name")?,
                    kind: match entry["kind"].as_str() {
                        Some("file") => folders::EntryKind::File,
                        Some("directory") => folders::EntryKind::Directory,
                        _ => return Err(protocol()),
                    },
                    bytes: number(entry, "bytes")?,
                })
            })
            .collect()
    }
    async fn read(
        accessor: &Accessor<T, Self>,
        grant_id: String,
        path: String,
    ) -> Result<Vec<u8>, AppError> {
        let (ipc, scope) = context(accessor)?;
        let value = ipc
            .call(
                &scope,
                "folders.read",
                json!({"grantId":grant_id,"path":path}),
            )
            .await
            .map_err(app_error)?;
        B64.decode(text(&value, "contentsBase64")?)
            .map_err(|_| protocol())
    }
    async fn write(
        accessor: &Accessor<T, Self>,
        grant_id: String,
        path: String,
        contents: Vec<u8>,
    ) -> Result<u64, AppError> {
        if contents.len() > 512 * 1024 {
            return Err(app_error(error(
                "LIMIT_EXCEEDED",
                "one folder write is at most512KiB",
            )));
        }
        let (ipc, scope) = context(accessor)?;
        let value = ipc
            .call(
                &scope,
                "folders.write",
                json!({"grantId":grant_id,"path":path,"contentsBase64":B64.encode(contents)}),
            )
            .await
            .map_err(app_error)?;
        number(&value, "bytesWritten")
    }
}
