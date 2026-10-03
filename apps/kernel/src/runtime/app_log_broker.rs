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
    /// Writes refused since the drop count was last noted.
    dropped: Arc<std::sync::atomic::AtomicU32>,
}

/// Hold extracted drops until their notice commits. Failed storage or a
/// panicking blocking task returns them to the shared counter.
struct PendingDrops {
    count: u32,
    counter: Arc<std::sync::atomic::AtomicU32>,
}

impl Drop for PendingDrops {
    fn drop(&mut self) {
        self.counter
            .fetch_add(self.count, std::sync::atomic::Ordering::Relaxed);
    }
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

    /// Whether a write is admitted, and whether it is the first of its second.
    fn admit(&self, now_ms: u64) -> (bool, bool) {
        let mut window = self.window.lock().unwrap_or_else(|e| e.into_inner());
        let second = now_ms / 1000;
        if window.0 != second {
            *window = (second, 0);
        }
        window.1 += 1;
        (window.1 <= PER_SECOND, window.1 == 1)
    }

    pub(crate) async fn dispatch(&self, request: BrokerRequest) -> Result<Value, RemoteError> {
        if request.method != "log.write" {
            return Err(error("METHOD_UNAVAILABLE", false));
        }
        self.write(request.params, crate::session::unix_epoch_ms())
            .await
    }

    async fn write(&self, params: Value, now_ms: u64) -> Result<Value, RemoteError> {
        let write: Write =
            serde_json::from_value(params).map_err(|_| error("INVALID_ARGUMENT", false))?;
        let (admitted, first) = self.admit(now_ms);
        if !admitted {
            return Err(self.drop_write("RATE_LIMITED"));
        }
        let permit = self
            .admission
            .clone()
            .try_acquire_owned()
            .map_err(|_| self.drop_write("APP_BUSY"))?;
        // Drops are noted at most once a second, before that second's first write.
        let dropped = if first {
            self.dropped.swap(0, std::sync::atomic::Ordering::Relaxed)
        } else {
            0
        };
        let mut pending_drops = PendingDrops {
            count: dropped,
            counter: self.dropped.clone(),
        };
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let fields = write
                .fields
                .unwrap_or_else(|| Value::Object(Default::default()));
            let result = service.store.append_app_log_after_drops(
                &service.owner,
                &service.installation,
                &write.level,
                &write.message,
                &fields,
                pending_drops.count,
            );
            if result.is_ok() {
                pending_drops.count = 0;
            }
            result
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

    #[tokio::test]
    async fn rejected_notice_write_retains_drops_for_the_next_successful_second() {
        let root =
            std::env::temp_dir().join(format!("chariox-log-drops-{:016x}", rand::random::<u64>()));
        std::fs::create_dir(&root).unwrap();
        let store = DurableKernelStateStore::open_owned(root.join("kernel.sqlite")).unwrap();
        let admission = Arc::new(Semaphore::new(1));
        let broker = AppLogBroker::new(store, "alice".into(), "todo".into(), admission.clone());
        let valid = serde_json::json!({"level":"info", "message":"after", "fields":{}});
        for _ in 0..PER_SECOND {
            broker.write(valid.clone(), 10_000).await.unwrap();
        }
        assert_eq!(
            broker.write(valid.clone(), 10_000).await.unwrap_err().code,
            "RATE_LIMITED"
        );
        let permit = admission.clone().acquire_owned().await.unwrap();
        assert_eq!(
            broker.write(valid.clone(), 11_000).await.unwrap_err().code,
            "APP_BUSY"
        );
        drop(permit);
        let oversized = serde_json::json!({"level":"info", "message":"rejected", "fields":{"big":"x".repeat(9000)}});
        assert_eq!(
            broker.write(oversized, 12_000).await.unwrap_err().code,
            "LIMIT_EXCEEDED"
        );
        // A later successful first write must recover both refused writes.
        broker.write(valid, 13_000).await.unwrap();
        let entries = broker.store.app_logs("alice", "todo", 0, 100).unwrap();
        assert_eq!(entries.len(), PER_SECOND as usize + 2);
        assert_eq!(entries[PER_SECOND as usize].fields["dropped"], 2);
        assert_eq!(entries[PER_SECOND as usize + 1].message, "after");
        drop(broker);
        std::fs::remove_dir_all(root).unwrap();
    }

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
        assert_eq!(broker.admit(10_000), (true, true));
        assert!((1..PER_SECOND).all(|_| broker.admit(10_000) == (true, false)));
        assert_eq!(broker.admit(10_500), (false, false));
        assert_eq!(
            broker.admit(11_000),
            (true, true),
            "a new second admits again"
        );
        // Refused writes are noted before the next second's first write.
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

    #[test]
    fn a_flood_of_secrets_is_limited_noted_and_stored_redacted() {
        let root = std::env::temp_dir().join(format!(
            "chariox-log-broker-flood-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir(&root).unwrap();
        let store = DurableKernelStateStore::open_owned(root.join("kernel.sqlite")).unwrap();
        let broker = AppLogBroker::new(
            store,
            "alice".into(),
            "todo".into(),
            Arc::new(Semaphore::new(1)),
        );
        let github = format!("ghp_{}", "a1B2c3".repeat(6));
        let message = format!("login password=hunter2 with {github}");
        let started = std::time::Instant::now();
        // 5000 writes in each of two seconds, as `dispatch` admits them.
        for now_ms in [20_000, 21_000] {
            for _ in 0..5000 {
                let (admitted, first) = broker.admit(now_ms);
                if !admitted {
                    broker.drop_write("RATE_LIMITED");
                    continue;
                }
                let dropped = if first {
                    broker.dropped.swap(0, std::sync::atomic::Ordering::Relaxed)
                } else {
                    0
                };
                broker
                    .store
                    .append_app_log_after_drops(
                        "alice",
                        "todo",
                        "info",
                        &message,
                        &serde_json::json!({"token": github, "note": message}),
                        dropped,
                    )
                    .unwrap();
            }
        }
        let elapsed = started.elapsed();
        let entries = broker.store.app_logs("alice", "todo", 0, 200).unwrap();
        assert_eq!(entries.len(), 2 * PER_SECOND as usize + 1);
        let notice = &entries[PER_SECOND as usize];
        assert!(notice.message.starts_with("4950 log writes were dropped"));
        assert_eq!(notice.fields["kernel"], true);
        let redacted = "login password=[redacted:password] with [redacted:github-token]";
        for entry in entries
            .iter()
            .filter(|entry| entry.fields.get("kernel").is_none())
        {
            assert_eq!(entry.message, redacted);
            assert_eq!(entry.fields["token"], "[redacted:token]");
            assert_eq!(entry.fields["note"], redacted);
        }
        assert!(elapsed < std::time::Duration::from_secs(10), "{elapsed:?}");
        drop(broker);
        let _ = std::fs::remove_dir_all(root);
    }
}
