//! Kernel-list questions asked over the relay connection the kernel already holds, so a
//! renewal does not wait behind a new TLS handshake on a link a bulk transfer has filled.

use super::*;

/// The kernel's established relay connection, as the route of metadata requests.
#[derive(Clone)]
pub(crate) struct MainConnection {
    state: Arc<RwLock<RelayClientState>>,
}

impl MainConnection {
    pub(crate) fn new(state: Arc<RwLock<RelayClientState>>) -> Self {
        Self { state }
    }

    /// Sends `request` (a `ClientMetadataRequest`) on the priority lane and waits for the
    /// response with the same id. `Ok(None)` when no connection to `relay_url` is open, so
    /// the caller opens its own socket. The state lock is never held while waiting.
    pub(crate) async fn query(
        &self,
        relay_url: &str,
        request: RelayEnvelope,
        wait: Duration,
    ) -> Result<Option<RelayEnvelope>, DaemonError> {
        let RelayEnvelope::ClientMetadataRequest { request_id, .. } = &request else {
            return Err(DaemonError::LocalTransport {
                operation: "send relay metadata request",
                message: "only a metadata request goes over the connection".into(),
            });
        };
        let request_id = request_id.clone();
        let (response_tx, response_rx) = oneshot::channel();
        let outgoing_tx = {
            let mut guard = self.state.write().await;
            let Some(outgoing_tx) = guard.outgoing_sender() else {
                return Ok(None);
            };
            if guard.connected_relay_url().as_deref() != Some(relay_url) {
                return Ok(None);
            }
            guard
                .pending_metadata_requests
                .insert(request_id.clone(), response_tx);
            outgoing_tx
        };
        let answer = async {
            send_outgoing_envelope(&outgoing_tx, request).map_err(|error| {
                DaemonError::LocalTransport {
                    operation: "write relay metadata request",
                    message: error.to_string(),
                }
            })?;
            match timeout(wait, response_rx).await {
                Ok(Ok(response)) => Ok(response),
                Ok(Err(_)) => Err(DaemonError::LocalTransport {
                    operation: "read relay metadata response",
                    message: "relay closed metadata connection".into(),
                }),
                Err(_) => Err(DaemonError::LocalTransport {
                    operation: "read relay metadata response",
                    message: format!("timed out after {}ms", wait.as_millis()),
                }),
            }
        }
        .await;
        self.state
            .write()
            .await
            .pending_metadata_requests
            .remove(&request_id);
        answer.map(Some)
    }
}

/// Hands a `ClientMetadataResponse` that arrived on the connection to its waiting question.
pub(super) async fn resolve_pending_metadata_response(
    state: &Arc<RwLock<RelayClientState>>,
    response: RelayEnvelope,
) {
    let RelayEnvelope::ClientMetadataResponse { request_id, .. } = &response else {
        return;
    };
    let waiting = state
        .write()
        .await
        .pending_metadata_requests
        .remove(request_id);
    if let Some(waiting) = waiting {
        let _ = waiting.send(response);
    }
}
