//! Periodic, retried, idempotent report of the byte counters to the hosted
//! control plane. A drained batch is written to a spool file before it is sent
//! and removed only after Cloud acknowledged it, so a failed send is retried
//! with the same batch id and a restart resumes the unacknowledged batch;
//! Cloud counts each batch id once. Only what was still in memory when the
//! process died (at most one interval) is lost.
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::meter::{UsageCounters, UsageMeter};

pub const DEFAULT_USAGE_REPORT_INTERVAL: Duration = Duration::from_secs(60);
const USAGE_SIGNATURE_DOMAIN: &str = "chariox-relay-usage-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageBatch {
    pub relay_id: String,
    pub batch_id: String,
    pub window_start_ms: u64,
    pub window_end_ms: u64,
    pub entries: Vec<UsageEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageEntry {
    pub realm_id: String,
    pub scope: String,
    #[serde(flatten)]
    pub counters: UsageCounters,
}

/// Why Cloud did not take a batch: a rejection that will not change by
/// retrying (the batch is dropped and logged) or one that may (it is kept).
#[derive(Debug, PartialEq, Eq)]
pub enum SendError {
    Rejected(String),
    Retry(String),
}

pub struct UsageReporter<S> {
    meter: UsageMeter,
    relay_id: String,
    spool: PathBuf,
    send: S,
    pending: Option<UsageBatch>,
    window_start_ms: u64,
}

impl<S: FnMut(&UsageBatch) -> Result<(), SendError>> UsageReporter<S> {
    /// Resumes the batch a previous process spooled and never got acknowledged.
    pub fn open(
        meter: UsageMeter,
        relay_id: String,
        spool: PathBuf,
        send: S,
        now_ms: u64,
    ) -> Result<Self, String> {
        let pending = match fs::read(&spool) {
            Ok(bytes) => Some(
                serde_json::from_slice::<UsageBatch>(&bytes).map_err(|error| {
                    format!(
                        "usage spool {} is not a usage batch ({error}); remove it to start over",
                        spool.display()
                    )
                })?,
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(format!("usage spool {}: {error}", spool.display())),
        };
        Ok(Self {
            meter,
            relay_id,
            spool,
            send,
            pending,
            window_start_ms: now_ms,
        })
    }

    /// One report round: send the spooled batch, or spool and send what was
    /// counted since the last one. Returns the batch Cloud acknowledged.
    pub fn report(&mut self, now_ms: u64) -> Result<Option<UsageBatch>, String> {
        if self.pending.is_none() {
            let drained = self.meter.drain();
            if drained.is_empty() {
                return Ok(None);
            }
            let batch = UsageBatch {
                relay_id: self.relay_id.clone(),
                batch_id: new_batch_id()?,
                window_start_ms: self.window_start_ms,
                window_end_ms: now_ms,
                entries: drained
                    .iter()
                    .map(|((realm_id, scope), counters)| UsageEntry {
                        realm_id: realm_id.clone(),
                        scope: scope.clone(),
                        counters: *counters,
                    })
                    .collect(),
            };
            if let Err(error) = write_spool(&self.spool, &batch) {
                self.meter.restore(drained);
                return Err(error);
            }
            self.window_start_ms = now_ms;
            self.pending = Some(batch);
        }
        let batch = self.pending.as_ref().expect("a batch is pending");
        match (self.send)(batch) {
            Ok(()) => {}
            Err(SendError::Retry(error)) => return Err(error),
            Err(SendError::Rejected(error)) => {
                eprintln!("relay usage batch {} dropped: {error}", batch.batch_id);
            }
        }
        remove_spool(&self.spool)?;
        Ok(self.pending.take())
    }
}

fn write_spool(path: &PathBuf, batch: &UsageBatch) -> Result<(), String> {
    let partial = path.with_extension("partial");
    let write = || -> std::io::Result<()> {
        let mut file = fs::File::create(&partial)?;
        file.write_all(&serde_json::to_vec(batch).map_err(std::io::Error::other)?)?;
        file.sync_all()?;
        fs::rename(&partial, path)
    };
    write().map_err(|error| format!("usage spool {}: {error}", path.display()))
}

fn remove_spool(path: &PathBuf) -> Result<(), String> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("usage spool {}: {error}", path.display()))
        }
        _ => Ok(()),
    }
}

fn new_batch_id() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    fs::File::open("/dev/urandom")
        .and_then(|mut random| random.read_exact(&mut bytes))
        .map_err(|error| format!("usage batch id: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn usage_url(base_url: &str) -> String {
    format!("{}/relay/usage", base_url.trim_end_matches('/'))
}

/// The proof binds the relay, the batch id (Cloud's idempotency key) and the
/// time; the body travels over TLS like every other relay-to-Cloud call.
fn usage_authorization(relay_id: &str, batch_id: &str, secret: &str, now_ms: u64) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let timestamp = now_ms.to_string();
    let payload = serde_json::to_string(&[USAGE_SIGNATURE_DOMAIN, relay_id, batch_id, &timestamp])
        .expect("string array serializes");
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC key is valid");
    mac.update(payload.as_bytes());
    let signature: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("CharioxRelay {timestamp}:{signature}")
}

fn post_batch(url: &str, secret: &str, batch: &UsageBatch) -> Result<(), SendError> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(10))
        .build();
    let now_ms = crate::revocation_sync::current_unix_ms();
    match agent
        .post(url)
        .set(
            "authorization",
            &usage_authorization(&batch.relay_id, &batch.batch_id, secret, now_ms),
        )
        .set("content-type", "application/json")
        .send_string(&serde_json::to_string(batch).expect("a usage batch serializes"))
    {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(code, response)) => {
            let message = format!("usage report answered {code}: {}", response.status_text());
            if (400..500).contains(&code) && !matches!(code, 401 | 403 | 408 | 429) {
                Err(SendError::Rejected(message))
            } else {
                Err(SendError::Retry(message))
            }
        }
        Err(error) => Err(SendError::Retry(error.to_string())),
    }
}

/// Report every `interval` until shutdown, then once more so a stopped relay
/// leaves nothing unreported.
pub async fn run_usage_report(
    base_url: String,
    relay_id: String,
    secret: String,
    spool: PathBuf,
    meter: UsageMeter,
    interval: Duration,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let url = usage_url(&base_url);
    let send = move |batch: &UsageBatch| post_batch(&url, &secret, batch);
    let reporter = UsageReporter::open(
        meter,
        relay_id,
        spool,
        send,
        crate::revocation_sync::current_unix_ms(),
    );
    let mut reporter = match reporter {
        Ok(reporter) => reporter,
        Err(error) => {
            log_report_error(&error);
            return;
        }
    };
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker.tick().await;
    let mut stopping = false;
    while !stopping {
        tokio::select! {
            changed = shutdown_rx.changed() => {
                stopping = changed.is_err() || *shutdown_rx.borrow();
                if !stopping {
                    continue;
                }
            }
            _ = ticker.tick() => {}
        }
        // A stopping relay sends the batch it still had spooled and then
        // what was counted since.
        let rounds = if stopping { 2 } else { 1 };
        let (returned, outcome) = tokio::task::spawn_blocking(move || {
            let mut outcome = Ok(None);
            for _ in 0..rounds {
                outcome = reporter.report(crate::revocation_sync::current_unix_ms());
                if outcome.is_err() {
                    break;
                }
            }
            (reporter, outcome)
        })
        .await
        .expect("usage report task does not panic");
        reporter = returned;
        match outcome {
            Ok(Some(batch)) => log_report_sent(&batch),
            Ok(None) => {}
            Err(error) => log_report_error(&error),
        }
    }
}

fn log_report_sent(batch: &UsageBatch) {
    let sum = |select: fn(&UsageCounters) -> u64| {
        batch
            .entries
            .iter()
            .map(|entry| select(&entry.counters))
            .sum::<u64>()
    };
    eprintln!(
        "{}",
        serde_json::json!({
            "component": "chariox-relay",
            "level": "info",
            "event": "usage_report_acknowledged",
            "fields": {
                "batch_id": batch.batch_id,
                "entries": batch.entries.len(),
                "bytes_in": sum(|c| c.bytes_in),
                "bytes_out": sum(|c| c.bytes_out),
            },
        })
    );
}

fn log_report_error(message: &str) {
    eprintln!(
        "{}",
        serde_json::json!({
            "component": "chariox-relay",
            "level": "error",
            "event": "usage_report_failed",
            "fields": { "message": message },
        })
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meter::Direction;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn spool(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("relay-usage-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join("spool.json")
    }

    type Log = Rc<RefCell<Vec<UsageBatch>>>;

    fn sender(
        log: &Log,
        fail: &Rc<RefCell<bool>>,
    ) -> impl FnMut(&UsageBatch) -> Result<(), SendError> {
        let (log, fail) = (log.clone(), fail.clone());
        move |batch| {
            log.borrow_mut().push(batch.clone());
            if *fail.borrow() {
                Err(SendError::Retry("cloud unreachable".into()))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn a_failed_send_is_retried_as_the_same_batch_and_a_restart_resumes_it() {
        let meter = UsageMeter::default();
        let path = spool("retry");
        let (log, fail) = (Log::default(), Rc::new(RefCell::new(true)));
        let mut first = UsageReporter::open(
            meter.clone(),
            "relay-eu".into(),
            path.clone(),
            sender(&log, &fail),
            1_000,
        )
        .unwrap();
        meter.record("realm-a", "control", Direction::In, 100);
        assert!(first.report(2_000).is_err());
        meter.record("realm-a", "control", Direction::In, 7);
        assert!(first.report(3_000).is_err());
        // The second round resent the first batch; the 7 bytes wait in memory.
        let sent = log.borrow().clone();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0], sent[1]);
        assert_eq!(sent[0].entries[0].counters.bytes_in, 100);

        // The relay restarts; a new process with a fresh meter resumes the spool.
        drop(first);
        let restarted = UsageMeter::default();
        *fail.borrow_mut() = false;
        let mut second = UsageReporter::open(
            restarted.clone(),
            "relay-eu".into(),
            path.clone(),
            sender(&log, &fail),
            4_000,
        )
        .unwrap();
        let acknowledged = second
            .report(5_000)
            .unwrap()
            .expect("the spooled batch is sent");
        assert_eq!(acknowledged, sent[0]);
        assert!(!path.exists());
        // Nothing is counted twice: the next round has nothing to send.
        assert_eq!(second.report(6_000).unwrap(), None);
    }

    #[test]
    fn a_batch_that_cannot_be_spooled_is_not_lost() {
        let meter = UsageMeter::default();
        let path = spool("nospool").join("missing-dir").join("spool.json");
        let (log, fail) = (Log::default(), Rc::new(RefCell::new(false)));
        let mut reporter = UsageReporter::open(
            meter.clone(),
            "relay-eu".into(),
            path,
            sender(&log, &fail),
            1_000,
        )
        .unwrap();
        meter.record("realm-a", "session", Direction::Out, 40);
        assert!(reporter.report(2_000).is_err());
        assert!(log.borrow().is_empty(), "an unspooled batch is never sent");
        assert_eq!(
            meter.snapshot().values().map(|c| c.bytes_out).sum::<u64>(),
            40
        );
    }

    #[test]
    fn a_rejected_batch_is_dropped_so_it_cannot_block_later_ones() {
        let meter = UsageMeter::default();
        let path = spool("reject");
        let mut calls = 0;
        let send = |_: &UsageBatch| {
            calls += 1;
            if calls == 1 {
                Err(SendError::Rejected("malformed".into()))
            } else {
                Ok(())
            }
        };
        let mut reporter =
            UsageReporter::open(meter.clone(), "relay-eu".into(), path.clone(), send, 1_000)
                .unwrap();
        meter.record("realm-a", "control", Direction::In, 1);
        reporter.report(2_000).unwrap();
        assert!(!path.exists());
        meter.record("realm-a", "control", Direction::In, 2);
        let next = reporter
            .report(3_000)
            .unwrap()
            .expect("later batches still go out");
        assert_eq!(next.entries[0].counters.bytes_in, 2);
    }

    #[test]
    fn the_proof_matches_the_cloud_fixture_and_binds_the_batch() {
        let proof = usage_authorization("relay-eu", "batch-1", "secret", 1_700_000_000_000);
        assert!(proof.starts_with("CharioxRelay 1700000000000:"));
        assert_ne!(
            proof,
            usage_authorization("relay-eu", "batch-2", "secret", 1_700_000_000_000)
        );
    }
}
