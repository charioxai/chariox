//! SDK `log.write`: the App's own structured log, stored per installation for
//! `app logs`. Owner and installation come from the worker, never the App;
//! writes are validated and rate-limited so a noisy App cannot flood storage.
use crate::durable_state::DurableKernelStateStore;
use chariox_app_runtime::{wire::RemoteError, worker_peer::BrokerRequest};
use serde::Deserialize;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;

/// Writes admitted per second per worker.
const PER_SECOND: u32 = 50;

#[derive(Clone)]
pub(crate) struct AppLogBroker {
    store: DurableKernelStateStore,
    owner: String,
    installation: String,
    admission: Arc<Semaphore>,
    window: Arc<Mutex<(u64, u32)>>,
    /// Writes refused since the last admitted one.
    dropped: Arc<std::sync::atomic::AtomicU32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Write {
    level: String,
    message: String,
    #[serde(default)]
    fields: Option<Value>,
}

impl AppLogBroker {
    pub(crate) fn new(
        store: DurableKernelStateStore,
        owner: String,
        installation: String,
        admission: Arc<Semaphore>,
    ) -> Self {
        Self {
            store,
            owner,
            installation,
            admission,
            window: Arc::new(Mutex::new((0, 0))),
            dropped: Arc::default(),
        }
    }

    fn drop_write(&self, code: &str) -> RemoteError {
        self.dropped
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        error(code, true)
    }

    fn admit(&self, now_ms: u64) -> bool {
        let mut window = self.window.lock().unwrap_or_else(|e| e.into_inner());
        let second = now_ms / 1000;
        if window.0 != second {
            *window = (second, 0);
        }
        window.1 += 1;
        window.1 <= PER_SECOND
    }

    pub(crate) async fn dispatch(&self, request: BrokerRequest) -> Result<Value, RemoteError> {
        if request.method != "log.write" {
            return Err(error("METHOD_UNAVAILABLE", false));
        }
        let write: Write =
            serde_json::from_value(request.params).map_err(|_| error("INVALID_ARGUMENT", false))?;
        if !self.admit(crate::session::unix_epoch_ms()) {
            return Err(self.drop_write("RATE_LIMITED"));
        }
        let permit = self
            .admission
            .clone()
            .try_acquire_owned()
            .map_err(|_| self.drop_write("APP_BUSY"))?;
        let dropped = self.dropped.swap(0, std::sync::atomic::Ordering::Relaxed);
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let fields = write
                .fields
                .unwrap_or_else(|| Value::Object(Default::default()));
            service.store.append_app_log_after_drops(
                &service.owner,
                &service.installation,
                &write.level,
                &write.message,
                &fields,
                dropped,
            )
        })
        .await
        .map_err(|_| error("STORAGE_UNAVAILABLE", true))?
        .map_err(|code| error(code, code == "STORAGE_UNAVAILABLE"))?;
        Ok(Value::Null)
    }
}

fn error(code: &str, retryable: bool) -> RemoteError {
    let message = match code {
        "INVALID_ARGUMENT" => "Invalid log entry",
        "LIMIT_EXCEEDED" => "Log message or fields are too large",
        "RATE_LIMITED" => "Too many log writes; slow down",
        "APP_BUSY" => "App operation limit reached",
        _ => "Log write did not complete",
    };
    RemoteError {
        code: code.into(),
        message: message.into(),
        retryable: Some(retryable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_worker_is_limited_per_second() {
        let root =
            std::env::temp_dir().join(format!("chariox-log-broker-{:016x}", rand::random::<u64>()));
        std::fs::create_dir(&root).unwrap();
        let store = DurableKernelStateStore::open_owned(root.join("kernel.sqlite")).unwrap();
        let broker = AppLogBroker::new(
            store,
            "alice".into(),
            "todo".into(),
            Arc::new(Semaphore::new(1)),
        );
        assert!((0..PER_SECOND).all(|_| broker.admit(10_000)));
        assert!(!broker.admit(10_500));
        assert!(broker.admit(11_000), "a new second admits again");
        // Refused writes are noted before the next admitted one.
        broker.drop_write("RATE_LIMITED");
        broker.drop_write("APP_BUSY");
        let dropped = broker.dropped.swap(0, std::sync::atomic::Ordering::Relaxed);
        broker
            .store
            .append_app_log_after_drops(
                "alice",
                "todo",
                "info",
                "after",
                &serde_json::json!({}),
                dropped,
            )
            .unwrap();
        let entries = broker.store.app_logs("alice", "todo", 0, 10).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries[0].message.starts_with("2 log writes were dropped"));
        assert_eq!(entries[0].fields["dropped"], 2);
        assert_eq!(entries[1].message, "after");
        drop(broker);
        let _ = std::fs::remove_dir_all(root);
    }
}
