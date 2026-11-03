//! MP-08 / MP-10 / MP-11: cancellation and release I/O for an owned HTTP lease.
use super::*;

#[derive(Clone)]
pub(super) struct PendingCall {
    pub(super) id: serde_json::Value,
    pub(super) headers: HttpState,
    pub(super) cancelled: Arc<AtomicBool>,
}
pub(super) struct CallLease<'a> {
    pub(super) session: &'a HttpSession,
    pub(super) key: u64,
    pub(super) cancelled: Arc<AtomicBool>,
}
impl Drop for CallLease<'_> {
    fn drop(&mut self) {
        self.session
            .state
            .lock()
            .expect("HTTP MCP session mutex poisoned")
            .calls
            .remove(&self.key);
    }
}

impl HttpSession {
    pub(super) fn cancel_pending(&self) {
        let calls: Vec<_> = {
            let state = self.state.lock().expect("HTTP MCP session mutex poisoned");
            state
                .calls
                .values()
                .filter(|call| !call.cancelled.swap(true, Ordering::AcqRel))
                .cloned()
                .collect()
        };
        if calls.is_empty() {
            return;
        }
        let backing = self.backing.clone();
        let excluded_roots = self.excluded_roots.clone();
        std::thread::spawn(move || {
            for call in calls {
                let mut headers = call.headers;
                let result = forward_streamable_http_mcp_request(
                    &backing.transport,
                    serde_json::json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":call.id,"reason":"Chariox prompt cancelled"}}),
                    Some(1),
                    &mut headers,
                    &excluded_roots,
                    None,
                );
                if result.is_err() {
                    crate::logging::warn_with_fields(
                        "daemon.provider.mcp_proxy",
                        "HTTP MCP cancellation delivery failed; local cancellation retained",
                        serde_json::json!({"registry_ref":backing.name,"reason":"upstream_unreachable_or_rejected"}),
                    );
                }
            }
        });
    }
    pub(super) fn release_remote(&self, state: &HttpState) {
        if state.session_id.is_none() {
            return;
        }
        let outcome = http_request(
            &self.backing.transport,
            "DELETE",
            Duration::from_secs(1),
            state,
            &self.excluded_roots,
        )
        .map_err(|_| None)
        .and_then(|request| {
            request.call().map_err(|error| match error {
                ureq::Error::Status(status, _) => Some(status),
                _ => None,
            })
        });
        if let Err(status) = outcome {
            crate::logging::warn_with_fields(
                "daemon.provider.mcp_proxy",
                "HTTP MCP session release failed; local ownership retired",
                serde_json::json!({"registry_ref":self.backing.name,"reason":if status.is_some(){"upstream_rejected"}else{"upstream_unreachable"},"http_status":status}),
            );
        }
    }
}
