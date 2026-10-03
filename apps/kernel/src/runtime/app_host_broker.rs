//! Shared backend SDK and view-host path. An App can offer text/a URL, never
//! read a clipboard, launch a browser, or answer the human's kernel prompt.
use crate::{
    durable_state::{
        app_host_actions::{HostActionCommand, HostOffer, OFFER_MS},
        DurableKernelStateStore,
    },
    local::AppHostAction,
};
use chariox_app_runtime::{
    app_outbox::EventCatalog, wire::RemoteError, worker_peer::BrokerRequest,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Semaphore;

pub(crate) const MAX_CLIPBOARD_BYTES: usize = 256 * 1024;
pub(crate) const MAX_LINK_BYTES: usize = 8 * 1024;

#[derive(Clone)]
pub(crate) struct AppHostBroker {
    store: DurableKernelStateStore,
    owner: String,
    installation: String,
    generation: u64,
    clipboard_write: bool,
    admission: Arc<Semaphore>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Clipboard {
    text: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Link {
    url: String,
}

impl AppHostBroker {
    pub(crate) fn new(
        store: DurableKernelStateStore,
        owner: String,
        catalog: Arc<EventCatalog>,
        admission: Arc<Semaphore>,
    ) -> Self {
        Self {
            store,
            owner,
            installation: catalog.installation_id().to_owned(),
            generation: catalog.generation(),
            clipboard_write: catalog.app_catalog().allows_clipboard_write(),
            admission,
        }
    }

    pub(crate) async fn dispatch(&self, request: BrokerRequest) -> Result<Value, RemoteError> {
        self.request(&request.method, request.params).await
    }

    pub(crate) async fn request(&self, method: &str, params: Value) -> Result<Value, RemoteError> {
        if method == "host.clipboard_write" && !self.clipboard_write {
            return Err(error("CAPABILITY_REQUIRED"));
        }
        let action = parse_action(method, params)?;
        let permit = self
            .admission
            .clone()
            .try_acquire_owned()
            .map_err(|_| error("APP_BUSY"))?;
        let store = self.store.clone();
        let offer = HostOffer {
            operation_id: format!("host-action-{:032x}", rand::random::<u128>()),
            owner: self.owner.clone(),
            installation: self.installation.clone(),
            generation: self.generation,
            action,
            expires_ms: crate::session::unix_epoch_ms() + OFFER_MS,
        };
        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            store.app_host_action(HostActionCommand::Create(offer))
        })
        .await
        .map_err(|_| error("STORAGE_UNAVAILABLE"))?;
        let offer = result
            .map_err(error)?
            .ok_or_else(|| error("STORAGE_UNAVAILABLE"))?;
        Ok(
            json!({"operationId": offer.operation_id, "state": "pending", "expiresAtMs": offer.expires_ms}),
        )
    }
}

fn parse_action(method: &str, params: Value) -> Result<AppHostAction, RemoteError> {
    match method {
        "host.clipboard_write" => {
            let Clipboard { text } =
                serde_json::from_value(params).map_err(|_| error("INVALID_ARGUMENT"))?;
            if text.len() > MAX_CLIPBOARD_BYTES
                || text.escape_debug().to_string().len() > MAX_CLIPBOARD_BYTES
                || serde_json::to_string(&text)
                    .map_err(|_| error("INVALID_ARGUMENT"))?
                    .len()
                    > MAX_CLIPBOARD_BYTES + 2
            {
                return Err(error("LIMIT_EXCEEDED"));
            }
            Ok(AppHostAction::ClipboardWrite { text })
        }
        "host.open_link" => {
            let Link { url } =
                serde_json::from_value(params).map_err(|_| error("INVALID_ARGUMENT"))?;
            if url.len() > MAX_LINK_BYTES {
                return Err(error("LIMIT_EXCEEDED"));
            }
            // Preserve the exact URL. Refuse trimming, control/bidi characters
            // and parser repairs that could disguise what the human opens.
            if url.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c,
                '\u{00ad}' | '\u{061c}' | '\u{180e}' | '\u{200b}'..='\u{200f}' |
                '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}' | '\u{fff9}'..='\u{fffb}'))
                || url.contains('\\')
            { return Err(error("INVALID_ARGUMENT")); }
            let parsed = url::Url::parse(&url).map_err(|_| error("INVALID_ARGUMENT"))?;
            if !matches!(parsed.scheme(), "http" | "https")
                || parsed.host_str().is_none()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || !url
                    .to_ascii_lowercase()
                    .starts_with(&format!("{}://", parsed.scheme()))
                || url
                    .get(parsed.scheme().len() + 3..)
                    .is_none_or(|authority| authority.starts_with('/'))
            {
                return Err(error("INVALID_ARGUMENT"));
            }
            Ok(AppHostAction::OpenLink { url })
        }
        _ => Err(error("METHOD_UNAVAILABLE")),
    }
}

fn error(code: &str) -> RemoteError {
    RemoteError {
        code: code.into(),
        message: "App host request did not complete".into(),
        retryable: Some(matches!(code, "APP_BUSY" | "STORAGE_UNAVAILABLE")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_arguments_are_bounded_and_links_are_exact_http_urls() {
        for url in [
            "file:///tmp/a",
            "javascript:alert(1)",
            "data:text/plain,x",
            "mailto:a@b",
            "//example.org",
            "https:example.org",
            "https://trusted.example@evil.example/",
            "https://user:password@example.org/",
            "https://:password@example.org/",
            "https:///example.org",
            "https://example.org/\n",
            "https://example.org/\u{202e}abc",
            "https://example.org\\path",
        ] {
            assert_eq!(
                parse_action("host.open_link", json!({"url":url}))
                    .unwrap_err()
                    .code,
                "INVALID_ARGUMENT",
                "{url:?}"
            );
        }
        for url in [
            "https://example.org/a?b=%20#c",
            "HTTP://example.org",
            "http://localhost:8080",
            "https://bücher.example/path",
        ] {
            assert_eq!(
                parse_action("host.open_link", json!({"url":url})).unwrap(),
                AppHostAction::OpenLink { url: url.into() }
            );
        }
        assert!(parse_action(
            "host.clipboard_write",
            json!({"text":"é".repeat(MAX_CLIPBOARD_BYTES / 2)})
        )
        .is_ok());
        assert_eq!(
            parse_action(
                "host.clipboard_write",
                json!({"text":"é".repeat(MAX_CLIPBOARD_BYTES / 2 + 1)})
            )
            .unwrap_err()
            .code,
            "LIMIT_EXCEEDED"
        );
        assert_eq!(
            parse_action(
                "host.open_link",
                json!({"url": format!("https://example.org/{}", "x".repeat(MAX_LINK_BYTES))})
            )
            .unwrap_err()
            .code,
            "LIMIT_EXCEEDED"
        );
        assert_eq!(
            parse_action(
                "host.clipboard_write",
                json!({"text":"\u{1b}".repeat(MAX_CLIPBOARD_BYTES / 4)})
            )
            .unwrap_err()
            .code,
            "LIMIT_EXCEEDED"
        );
        assert!(parse_action("host.clipboard_write", json!({"text":"", "owner":"bob"})).is_err());
        assert!(parse_action("host.clipboard_read", json!({})).is_err());
    }
}
