//! Async App bindings. Host capabilities still come from the signed manifest
//! and the kernel's admission policy, never from this SDK.

pub mod bindings {
    wit_bindgen::generate!({
        world: "app",
        path: "../app-wit/wit",
        generate_all,
        pub_export_macro: true,
    });
}

pub use bindings::chariox::app::*;
pub use bindings::exports::chariox::app::handlers::Guest;
pub use wit_bindgen::{FutureReader, StreamReader, StreamResult};

use app_types::AppError;
use futures::future::{select, Either};
use std::future::{Future, IntoFuture};

pub fn error(code: impl Into<String>, message: impl Into<String>) -> AppError {
    AppError {
        code: code.into(),
        message: message.into(),
        retryable: None,
    }
}

/// Dropping the losing work future drops its pending imports and owned streams.
/// The worker remains responsible for deadlines and non-cooperative guest code.
pub async fn cancellable<T>(
    cancel: FutureReader<String>,
    work: impl Future<Output = Result<T, AppError>>,
) -> Result<T, AppError> {
    match select(std::pin::pin!(work), std::pin::pin!(cancel.into_future())).await {
        Either::Left((result, _)) => result,
        Either::Right((reason, _)) => Err(AppError {
            code: "CANCELLED".into(),
            message: reason,
            retryable: Some(false),
        }),
    }
}

/// A convenience for JSON-valued state; versions remain u64 for CAS checks.
pub async fn get_json(key: &str) -> Result<Option<(serde_json::Value, u64)>, AppError> {
    state::get(key.to_owned())
        .await?
        .map(|record| {
            serde_json::from_str(&record.value)
                .map(|value| (value, record.version))
                .map_err(|e| error("INVALID_JSON", e.to_string()))
        })
        .transpose()
}
