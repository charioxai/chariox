//! Bounded relay decoding and retained request memory, shared by every connection.
use super::{relay_error, LocalDaemonRequest, ParsedRelayClientMessage};
use chariox_relay::protocol::{EncryptedRelayPayload, RelayError};
use std::sync::{Arc, OnceLock};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub(super) const SMALL_REQUEST_BYTES: usize = 768 * 1024;
// A 5 MiB attachment takes 6.7 MiB after inline base64. Twelve MiB leaves
// room for the prompt and metadata, well below the relay's 64 MiB message cap.
pub(super) const PROMPT_REQUEST_BYTES: usize = 12 * 1024 * 1024;
pub(in crate::transport::relay_client) const MAX_ENCODED_REQUEST_BYTES: usize =
    PROMPT_REQUEST_BYTES.div_ceil(3) * 4;
// Account encoded bytes across all connections, including each spawned dispatch.
// This is stricter than a per-connection budget; opening sockets cannot evade it.
const RETAINED_ENCODED_BYTES: u32 = 64 * 1024 * 1024;
static RETAINED: OnceLock<Arc<Semaphore>> = OnceLock::new();
pub(super) type RequestMemoryGuard = OwnedSemaphorePermit;

pub(super) fn reserve(payload: &EncryptedRelayPayload) -> Result<RequestMemoryGuard, RelayError> {
    let encoded_bytes = payload.ciphertext.len();
    // Length alone rejects an oversized body, even when its base64 or key is
    // invalid. Do not decode it merely to report that it exceeds the limit.
    if encoded_bytes > MAX_ENCODED_REQUEST_BYTES {
        return Err(too_large("Prompt and attachments exceed the 12 MiB relay request limit. Attach smaller files or use file.transfer."));
    }
    let retained =
        RETAINED.get_or_init(|| Arc::new(Semaphore::new(RETAINED_ENCODED_BYTES as usize)));
    reserve_from(retained, encoded_bytes as u32)
}

fn reserve_from(
    retained: &Arc<Semaphore>,
    encoded_bytes: u32,
) -> Result<RequestMemoryGuard, RelayError> {
    Arc::clone(retained)
        .try_acquire_many_owned(encoded_bytes)
        .map_err(|_| {
            relay_error(
                "request_busy",
                "Relay request memory is full. Wait for pending requests and try again.",
                true,
            )
        })
}

pub(super) fn validate_kind(
    message: &ParsedRelayClientMessage,
    plaintext_bytes: usize,
) -> Result<(), RelayError> {
    let attachments = match message {
        ParsedRelayClientMessage::Request(request) => match &request.request {
            LocalDaemonRequest::SubmitPrompt(request) => !request.attachments.is_empty(),
            LocalDaemonRequest::SubmitPrompts(request) => request
                .prompts
                .iter()
                .any(|prompt| !prompt.attachments.is_empty()),
            LocalDaemonRequest::SteerActivePrompt(request) => !request.attachments.is_empty(),
            _ => false,
        },
        _ => false,
    };
    if !attachments && plaintext_bytes.saturating_add(16) > SMALL_REQUEST_BYTES {
        return Err(too_large("This relay request exceeds the 768 KiB limit. Only prompt requests with attachments may be larger."));
    }
    Ok(())
}

fn too_large(message: &str) -> RelayError {
    relay_error("request_too_large", message, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn request_memory_stays_reserved_until_dispatch_drops_it() {
        let retained = Arc::new(Semaphore::new(RETAINED_ENCODED_BYTES as usize));
        let request = reserve_from(&retained, RETAINED_ENCODED_BYTES).unwrap();
        let error = reserve_from(&retained, 1).unwrap_err();
        assert_eq!(error.code, "request_busy");
        assert!(error.retryable);
        drop(request);
        assert_eq!(
            retained.available_permits(),
            RETAINED_ENCODED_BYTES as usize
        );
        assert!(reserve_from(&retained, RETAINED_ENCODED_BYTES).is_ok());
    }
    #[test]
    fn non_prompt_request_keeps_its_small_bound() {
        let message =
            super::super::parse_relay_client_request(br#"{"ListSessions":null}"#).unwrap();
        assert!(validate_kind(&message, SMALL_REQUEST_BYTES - 16).is_ok());
        assert_eq!(
            validate_kind(&message, SMALL_REQUEST_BYTES - 15)
                .unwrap_err()
                .code,
            "request_too_large"
        );
    }
}
