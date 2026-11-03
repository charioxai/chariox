//! MP-08 / MP-09 / MP-10 / MP-11: a provider that gives up on a `tools/call` (its tool timeout) sends
//! `notifications/cancelled`. The waiting call is dropped then, so whatever it left open, such as a popup
//! card only that call could answer, is withdrawn.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::Value;
use tokio::sync::Notify;

type CallKey = (String, String);

fn in_flight() -> &'static Mutex<HashMap<CallKey, Arc<Notify>>> {
    static CALLS: OnceLock<Mutex<HashMap<CallKey, Arc<Notify>>>> = OnceLock::new();
    CALLS.get_or_init(Default::default)
}

/// One running `tools/call`, registered under its run token and JSON-RPC id until it ends.
pub(super) struct InFlightCall {
    key: CallKey,
    pub(super) cancelled: Arc<Notify>,
}

impl InFlightCall {
    pub(super) fn begin(auth_token: &str, id: &Value) -> Self {
        let key = (auth_token.to_string(), id.to_string());
        let cancelled = Arc::new(Notify::new());
        in_flight()
            .lock()
            .unwrap()
            .insert(key.clone(), Arc::clone(&cancelled));
        Self { key, cancelled }
    }
}

impl Drop for InFlightCall {
    fn drop(&mut self) {
        in_flight().lock().unwrap().remove(&self.key);
    }
}

/// Handles `notifications/cancelled` for this run's call; unknown or finished ids are ignored.
pub(super) fn cancel(auth_token: &str, params: &Value) {
    let Some(request_id) = params.get("requestId") else {
        return;
    };
    let key = (auth_token.to_string(), request_id.to_string());
    if let Some(call) = in_flight().lock().unwrap().get(&key) {
        call.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancel_reaches_only_the_named_call_of_the_same_run() {
        let call = InFlightCall::begin("run-token", &serde_json::json!(7));
        let other = InFlightCall::begin("run-token", &serde_json::json!(8));
        cancel("other-run", &serde_json::json!({"requestId": 7}));
        cancel("run-token", &serde_json::json!({"requestId": 9}));
        cancel("run-token", &serde_json::json!({"requestId": 7}));
        tokio::time::timeout(std::time::Duration::from_secs(2), call.cancelled.notified())
            .await
            .expect("the named call is cancelled");
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(100),
                other.cancelled.notified()
            )
            .await
            .is_err(),
            "another call of the run keeps waiting"
        );
        drop(call);
        cancel("run-token", &serde_json::json!({"requestId": 7}));
    }
}
