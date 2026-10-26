use crate::{
    attribution::Attribution,
    bindings::chariox::app::{app_types::AppError, log, registry},
    ipc::{error, Ipc},
    limits::Memory,
    scope::Scope,
};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Default, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Registration {
    pub tools: BTreeSet<String>,
    pub events: BTreeSet<String>,
    pub lifecycle: BTreeSet<String>,
}
pub struct Host {
    pub ipc: Ipc,
    pub attribution: Attribution,
    pub memory: Memory,
    pub registration: Registration,
    pub sealed: bool,
    pub subscriptions: crate::streams::Subscriptions,
    pub stream_slots: std::sync::Arc<tokio::sync::Semaphore>,
}
impl Host {
    pub fn scope(&self) -> Result<Scope, AppError> {
        self.attribution.current().ok_or_else(|| {
            app_error(error(
                "APP_CALL_CONTEXT",
                "broker operation outside an invocation",
            ))
        })
    }
}
pub fn app_error(error: chariox_app_runtime::wire::RemoteError) -> AppError {
    AppError {
        code: error.code,
        message: error.message,
        retryable: error.retryable,
    }
}
pub fn remote_error(error: AppError) -> chariox_app_runtime::wire::RemoteError {
    chariox_app_runtime::wire::RemoteError {
        code: error.code,
        message: error.message,
        retryable: error.retryable,
    }
}
pub fn json(text: &str) -> Result<Value, AppError> {
    // Reuse the wire boundary's depth, number and duplicate-key rules.
    chariox_app_runtime::wire::decode(
        format!(
            r#"{{"kind":"response","version":1,"generation":"1","id":"json","result":{text}}}"#
        )
        .as_bytes(),
        "1",
        chariox_app_runtime::wire::Sender::Worker,
    )
    .map_err(|_| app_error(error("INVALID_ARGUMENT", "invalid App JSON")))
    .map(|message| match message {
        chariox_app_runtime::wire::Message::Response {
            outcome: chariox_app_runtime::wire::Outcome::Success(result),
            ..
        } => result.result,
        _ => unreachable!(),
    })
}
impl registry::Host for Host {
    fn tool(&mut self, name: String) -> wasmtime::Result<()> {
        register(&mut self.registration.tools, self.sealed, name)
    }
    fn event(&mut self, name: String) -> wasmtime::Result<()> {
        register(&mut self.registration.events, self.sealed, name)
    }
    fn lifecycle(&mut self, name: String) -> wasmtime::Result<()> {
        register(&mut self.registration.lifecycle, self.sealed, name)
    }
}
fn register(names: &mut BTreeSet<String>, sealed: bool, name: String) -> wasmtime::Result<()> {
    wasmtime::ensure!(!sealed, "registration is sealed");
    wasmtime::ensure!(
        names.len() < 256 && !name.is_empty() && name.len() <= 128,
        "invalid registration"
    );
    wasmtime::ensure!(names.insert(name), "duplicate registration");
    Ok(())
}
impl log::Host for Host {
    fn write(
        &mut self,
        level: log::Level,
        message: String,
        fields: String,
    ) -> wasmtime::Result<()> {
        wasmtime::ensure!(
            message.len() <= 4096 && fields.len() <= 16_384,
            "App log exceeds 16 KiB"
        );
        let fields = json(&fields).map_err(|_| wasmtime::Error::msg("invalid log JSON"))?;
        let level = match level {
            log::Level::Debug => "debug",
            log::Level::Info => "info",
            log::Level::Warn => "warn",
            log::Level::Error => "error",
        };
        self.ipc.cleanup(
            "log.write",
            serde_json::json!({ "level": level, "message": message, "fields": fields }),
        );
        Ok(())
    }
}
impl crate::bindings::chariox::app::app_types::Host for Host {}
