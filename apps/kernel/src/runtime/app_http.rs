//! HTTP transport below the existing worker broker. This module does not mint
//! installation grants, connection credentials or critical-operation receipts.
mod broker;
mod decode;
mod dns;
mod encode;
#[cfg(test)]
pub(crate) mod fixture;
mod limits;
mod policy;
mod streams;
mod transport;
pub(crate) use broker::{AppHttpBroker, HttpContext};
pub(crate) use limits::HttpLimits;
pub(crate) use streams::HttpJob;

use thiserror::Error;

pub(crate) const MAX_STREAMS_PER_INSTALLATION: usize = 4;
pub(crate) const MAX_STREAMS_PER_KERNEL: usize = 32;
pub(crate) const CHUNK_BYTES: usize = 64 * 1024;
pub(crate) const MAX_REQUEST_BYTES: u64 = 64 * 1024 * 1024;
pub(crate) const MAX_RESPONSE_BYTES: u64 = 256 * 1024 * 1024;
pub(crate) const STREAM_LIFETIME: std::time::Duration = std::time::Duration::from_secs(3600);
pub(crate) const NETWORK_INACTIVITY: std::time::Duration = std::time::Duration::from_secs(120);

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HttpError {
    #[error("app_http_invalid_request")]
    Invalid,
    #[error("app_http_destination_denied")]
    Destination,
    #[error("app_http_connection_authority_unavailable")]
    ConnectionAuthority,
    #[error("app_http_protected_effect_unavailable")]
    ProtectedEffect,
    #[error("app_http_provenance")]
    Provenance,
    #[error("app_http_limit")]
    Limit,
    #[error("app_http_cancelled")]
    Cancelled,
    #[error("app_http_deadline")]
    Deadline,
    #[error("app_http_network_failure")]
    Network,
    #[error("app_http_tls_failure")]
    Tls,
    #[error("app_http_busy")]
    Busy,
}
type Result<T> = std::result::Result<T, HttpError>;

fn stopped(signal: &tokio::sync::watch::Receiver<bool>) -> bool {
    *signal.borrow() || signal.has_changed().is_err()
}
async fn cancelled(signal: &mut tokio::sync::watch::Receiver<bool>) {
    while !stopped(signal) {
        if signal.changed().await.is_err() {
            return;
        }
    }
}
