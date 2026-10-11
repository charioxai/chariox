//! MP-08 / MP-10 / MP-11: HTTP MCP transport state belongs to one runtime owner.
use super::*;
use crate::mcp::{CharioxMcpServerConfig, CharioxMcpTransportConfig};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Default)]
pub(super) struct HttpState {
    pub session_id: Option<zeroize::Zeroizing<String>>,
    pub protocol_version: Option<String>,
}
struct HttpSession {
    backing: CharioxMcpServerConfig,
    state: HttpState,
}
impl HttpSession {
    fn dispatch(&mut self, payload: serde_json::Value) -> Result<serde_json::Value, DaemonError> {
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
        forward_streamable_http_mcp_request(
            url,
            bearer_token_env_var.as_deref(),
            http_headers,
            env_http_headers,
            payload,
            self.backing.tool_timeout_sec,
            &mut self.state,
        )
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
        if self.state.session_id.is_some() {
            // Best-effort remote lease release. Local state is always retired;
            // servers may legitimately reject DELETE with 405. Bound cleanup.
            if let Ok(request) = http_request(
                url,
                "DELETE",
                bearer_token_env_var.as_deref(),
                http_headers,
                env_http_headers,
                Duration::from_secs(1),
                &self.state,
            ) {
                let _ = request.call();
            }
        }
    }
}
type OwnerKey = (String, String, String);
type Sessions = BTreeMap<OwnerKey, Arc<Mutex<HttpSession>>>;
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
            Arc::new(Mutex::new(HttpSession {
                backing: backing.clone(),
                state: HttpState::default(),
            }))
        })
        .clone();
    let result = session
        .lock()
        .expect("HTTP MCP session mutex poisoned")
        .dispatch(payload);
    result
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
    drop(released);
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
