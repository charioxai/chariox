//! MP-08 / MP-10 / MP-11: HTTP MCP transport state belongs to one runtime owner.
use super::*;
use crate::mcp::CharioxMcpServerConfig;
#[cfg(test)]
use crate::mcp::CharioxMcpTransportConfig;
#[path = "session_lifecycle.rs"]
mod lifecycle;
use lifecycle::{CallLease, PendingCall};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, OnceLock,
};

#[derive(Default, Clone, PartialEq, Eq)]
pub(super) struct HttpState {
    pub session_id: Option<zeroize::Zeroizing<String>>,
    pub protocol_version: Option<String>,
}
struct HttpSession {
    backing: CharioxMcpServerConfig,
    excluded_roots: Vec<std::path::PathBuf>,
    state: Mutex<SessionState>,
}
#[derive(Default)]
struct SessionState {
    headers: HttpState,
    generation: u64,
    retired: bool,
    next_call: u64,
    calls: BTreeMap<u64, PendingCall>,
}
impl HttpSession {
    fn dispatch(&self, payload: serde_json::Value) -> Result<serde_json::Value, DaemonError> {
        // Protect header state only. Tool POSTs and cancellation must run concurrently.
        let initialize =
            payload.get("method").and_then(serde_json::Value::as_str) == Some("initialize");
        let (before, generation, call) = {
            let mut state = self.state.lock().expect("HTTP MCP session mutex poisoned");
            if state.retired {
                return Err(cancelled_error("MCP owner retired"));
            }
            if payload.get("method").and_then(serde_json::Value::as_str)
                == Some("notifications/cancelled")
            {
                for call in state
                    .calls
                    .values()
                    .filter(|call| payload.pointer("/params/requestId") == Some(&call.id))
                {
                    call.cancelled.store(true, Ordering::Release);
                }
            }
            if initialize {
                state.generation += 1;
                state.headers = HttpState::default()
            }
            let call = if !initialize && payload.get("method").is_some() {
                payload.get("id").cloned().map(|id| {
                    state.next_call += 1;
                    let key = state.next_call;
                    let cancelled = Arc::new(AtomicBool::new(false));
                    let headers = state.headers.clone();
                    state.calls.insert(
                        key,
                        PendingCall {
                            id,
                            headers,
                            cancelled: cancelled.clone(),
                        },
                    );
                    CallLease {
                        session: self,
                        key,
                        cancelled,
                    }
                })
            } else {
                None
            };
            (state.headers.clone(), state.generation, call)
        };
        let mut headers = before.clone();
        let result = forward_streamable_http_mcp_request(
            &self.backing.transport,
            payload,
            self.backing.tool_timeout_sec,
            &mut headers,
            &self.excluded_roots,
            call.as_ref().map(|call| call.cancelled.as_ref()),
        );
        if initialize || headers != before {
            let mut state = self.state.lock().expect("HTTP MCP session mutex poisoned");
            if state.retired {
                // Initialization can finish after retirement. Its newly assigned
                // upstream lease must still be released, without resurrecting it.
                drop(state);
                if initialize {
                    self.release_remote(&headers);
                }
                return Err(cancelled_error("MCP owner retired"));
            }
            // An older 404/initialize must not erase or replace a newer handshake.
            if state.generation == generation && state.headers == before {
                state.headers = headers
            }
        }
        result
    }
    fn begin_retire(&self) -> HttpState {
        let mut state = self.state.lock().expect("HTTP MCP session mutex poisoned");
        state.retired = true;
        for call in state.calls.values() {
            call.cancelled.store(true, Ordering::Release);
        }
        std::mem::take(&mut state.headers)
    }
}
impl Drop for HttpSession {
    fn drop(&mut self) {
        let state = std::mem::take(
            &mut self
                .state
                .get_mut()
                .expect("HTTP MCP session mutex poisoned")
                .headers,
        );
        self.release_remote(&state);
    }
}
type OwnerKey = (String, String, String, Vec<std::path::PathBuf>);
type Sessions = BTreeMap<OwnerKey, Arc<HttpSession>>;
fn sessions() -> &'static Mutex<Sessions> {
    static SESSIONS: OnceLock<Mutex<Sessions>> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(BTreeMap::new()))
}
pub(crate) fn dispatch_with_excluded_roots(
    run: &str,
    owner: &str,
    backing: &CharioxMcpServerConfig,
    payload: serde_json::Value,
    excluded_roots: &[std::path::PathBuf],
) -> Result<serde_json::Value, DaemonError> {
    let key = (
        run.to_owned(),
        owner.to_owned(),
        backing.definition_hash()?,
        excluded_roots.to_vec(),
    );
    let session = sessions()
        .lock()
        .expect("HTTP MCP sessions mutex poisoned")
        .entry(key)
        .or_insert_with(|| {
            Arc::new(HttpSession {
                backing: backing.clone(),
                excluded_roots: excluded_roots.to_vec(),
                state: Mutex::new(SessionState::default()),
            })
        })
        .clone();
    session.dispatch(payload)
}
pub(crate) fn release_session(owner: &str) {
    release(|key| key.1 == owner);
}
pub(crate) fn release_run(run: &str) {
    release(|key| key.0 == run);
}
pub(crate) fn cancel_run(run: &str) {
    let owned: Vec<_> = sessions()
        .lock()
        .expect("HTTP MCP sessions mutex poisoned")
        .iter()
        .filter(|(key, _)| key.0 == run)
        .map(|(_, session)| session.clone())
        .collect();
    for session in owned {
        session.cancel_pending();
    }
}
fn release(matches: impl Fn(&OwnerKey) -> bool) {
    // Drop final leases after releasing the registry lock: remote I/O must not
    // serialize unrelated sessions. In-flight requests retain their own Arc.
    let released: Vec<_> = {
        let mut sessions = sessions().lock().expect("HTTP MCP sessions mutex poisoned");
        let keys: Vec<_> = sessions
            .keys()
            .filter(|key| matches(key))
            .cloned()
            .collect();
        keys.into_iter()
            .filter_map(|key| sessions.remove(&key))
            .collect()
    };
    if !released.is_empty() {
        // Callers may hold the provider-service lock. DELETE is best effort and
        // belongs on a blocking cleanup thread, after all local leases retire.
        let released: Vec<_> = released
            .into_iter()
            .map(|session| {
                let headers = session.begin_retire();
                (session, headers)
            })
            .collect();
        std::thread::spawn(move || {
            for (session, headers) in released {
                session.release_remote(&headers);
            }
        });
    }
}
fn cancelled_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "mcp.proxy.http.cancelled",
        message: message.into(),
    }
}

#[cfg(test)]
fn dispatch(
    run: &str,
    owner: &str,
    backing: &CharioxMcpServerConfig,
    payload: serde_json::Value,
) -> Result<serde_json::Value, DaemonError> {
    dispatch_with_excluded_roots(run, owner, backing, payload, &[])
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
