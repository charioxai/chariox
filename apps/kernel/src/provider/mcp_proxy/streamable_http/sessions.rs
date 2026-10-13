//! MP-08 / MP-10 / MP-11: HTTP MCP transport state belongs to one runtime owner.
use super::*;
use crate::mcp::{CharioxMcpServerConfig, CharioxMcpTransportConfig};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Default, Clone, PartialEq, Eq)]
pub(super) struct HttpState {
    pub session_id: Option<zeroize::Zeroizing<String>>,
    pub protocol_version: Option<String>,
}
struct HttpSession {
    backing: CharioxMcpServerConfig,
    state: Mutex<SessionState>,
}
#[derive(Default)]
struct SessionState {
    headers: HttpState,
    generation: u64,
}
impl HttpSession {
    fn dispatch(&self, payload: serde_json::Value) -> Result<serde_json::Value, DaemonError> {
        let CharioxMcpTransportConfig::StreamableHttp {
            url,
            bearer_token_env_var,
            http_headers,
            env_http_headers,
            ..
        } = &self.backing.transport
        else {
            unreachable!()
        };
        // Protect header state only. Tool POSTs and cancellation must run concurrently.
        let initialize =
            payload.get("method").and_then(serde_json::Value::as_str) == Some("initialize");
        let (before, generation) = {
            let mut state = self.state.lock().expect("HTTP MCP session mutex poisoned");
            if initialize {
                state.generation += 1;
                state.headers = HttpState::default()
            }
            (state.headers.clone(), state.generation)
        };
        let mut headers = before.clone();
        let result = forward_streamable_http_mcp_request(
            url,
            bearer_token_env_var.as_deref(),
            http_headers,
            env_http_headers,
            payload,
            self.backing.tool_timeout_sec,
            &mut headers,
        );
        if initialize || headers != before {
            let mut state = self.state.lock().expect("HTTP MCP session mutex poisoned");
            // An older 404/initialize must not erase or replace a newer handshake.
            if state.generation == generation && state.headers == before {
                state.headers = headers
            }
        }
        result
    }
}
impl Drop for HttpSession {
    fn drop(&mut self) {
        let CharioxMcpTransportConfig::StreamableHttp {
            url,
            bearer_token_env_var,
            http_headers,
            env_http_headers,
            ..
        } = &self.backing.transport
        else {
            return;
        };
        let state = &self
            .state
            .get_mut()
            .expect("HTTP MCP session mutex poisoned")
            .headers;
        if state.session_id.is_some() {
            // Best-effort remote lease release. Local state is always retired;
            // servers may legitimately reject DELETE with 405. Bound cleanup.
            if let Ok(request) = http_request(
                url,
                "DELETE",
                bearer_token_env_var.as_deref(),
                http_headers,
                env_http_headers,
                Duration::from_secs(1),
                state,
            ) {
                let _ = request.call();
            }
        }
    }
}
type OwnerKey = (String, String, String);
type Sessions = BTreeMap<OwnerKey, Arc<HttpSession>>;
fn sessions() -> &'static Mutex<Sessions> {
    static SESSIONS: OnceLock<Mutex<Sessions>> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(BTreeMap::new()))
}
pub(crate) fn dispatch(
    run: &str,
    owner: &str,
    backing: &CharioxMcpServerConfig,
    payload: serde_json::Value,
) -> Result<serde_json::Value, DaemonError> {
    let key = (run.to_owned(), owner.to_owned(), backing.definition_hash()?);
    let session = sessions()
        .lock()
        .expect("HTTP MCP sessions mutex poisoned")
        .entry(key)
        .or_insert_with(|| {
            Arc::new(HttpSession {
                backing: backing.clone(),
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
        std::thread::spawn(move || drop(released));
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
