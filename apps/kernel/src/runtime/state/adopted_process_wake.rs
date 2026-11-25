//! Row 554: a command an agent left running in the background (`nohup cmd &`, a shell loop that
//! polls) is not watched by anything: the turn ends, the agent is idle, and nothing will ever
//! start its next turn. At turn end the kernel adopts each such command as a wake of the agent,
//! so the command's end wakes it.
//!
//! The kernel did not start these commands, so it never supervises or signals them. It only
//! looks: a command is the process it was when adopted while its PID still has the start time
//! the kernel read then (a reused PID is another process), and it has ended once it is gone or a
//! zombie. Its exit code is not known.
use super::wake_driver::{Cancel, Liveness, Recovery, WakeDriver, WakeOccurrence, WakeSink};
use super::*;
use crate::durable_state::agent_lifecycle::{AgentWake, WakeEnd};
use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) const KIND: &str = "adopted_process";
/// How often a live command is looked at.
const POLL: std::time::Duration = std::time::Duration::from_secs(1);
const COMMIT_RETRY_AFTER: std::time::Duration = std::time::Duration::from_millis(500);
/// Commands adopted at one turn end; a runaway fork loop never becomes a wake each.
pub(super) const MAX_ADOPTED: usize = 8;
const COMMAND_CHARS: usize = 160;

/// What an adopted wake watches, kept in the wake's `detail`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct AdoptedDetail {
    pub pid: u32,
    /// The process's start time in clock ticks since boot: with the PID, its identity.
    pub start_ticks: u64,
    pub command: String,
}

/// A background command the agent's turn left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Background {
    pub pid: u32,
    pub start_ticks: u64,
    pub command: String,
}

struct Stat {
    parent: u32,
    start_ticks: u64,
    ended: bool,
}

#[cfg(target_os = "linux")]
fn stat(pid: u32) -> Option<Stat> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields: Vec<&str> = text[text.rfind(')')? + 1..].split_whitespace().collect();
    Some(Stat {
        parent: fields.get(1)?.parse().ok()?,
        start_ticks: fields.get(19)?.parse().ok()?,
        ended: matches!(fields.first().copied(), Some("Z" | "X")),
    })
}

#[cfg(not(target_os = "linux"))]
fn stat(_pid: u32) -> Option<Stat> {
    None
}

/// Whether `pid` is still the process that had `start_ticks`.
fn running(pid: u32, start_ticks: u64) -> bool {
    stat(pid).is_some_and(|s| s.start_ticks == start_ticks && !s.ended)
}

fn environment_has(pid: u32, name: &str, value: Option<&str>) -> bool {
    let Ok(environ) = std::fs::read(format!("/proc/{pid}/environ")) else {
        return false;
    };
    let prefix = format!("{name}=");
    environ
        .split(|byte| *byte == 0)
        .filter_map(|entry| entry.strip_prefix(prefix.as_bytes()))
        .any(|found| value.is_none_or(|value| found == value.as_bytes()))
}

fn command_line(pid: u32) -> String {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
    let words: Vec<String> = raw
        .split(|byte| *byte == 0)
        .filter(|word| !word.is_empty())
        .map(|word| String::from_utf8_lossy(word).into_owned())
        .collect();
    let line = crate::secret_redaction::redact_secrets(&words.join(" ")).into_owned();
    match line.char_indices().nth(COMMAND_CHARS) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None if line.is_empty() => format!("process {pid}"),
        None => line,
    }
}

/// The processes of provider run `run` that the turn left running and nothing watches: they carry
/// the run's marker (every process the provider started does, however it was detached), they are
/// not the provider process or anything still below it (its helpers, and the commands it tracks
/// itself), and no wake of the kernel's covers them (`skip`, or a watched process's own marker).
pub(super) fn left_running(
    run: &str,
    provider_pids: &BTreeSet<u32>,
    skip: &BTreeSet<u32>,
) -> Vec<Background> {
    let Ok(listing) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let own = std::process::id();
    let pids: Vec<u32> = listing
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
        .collect();
    let parents: HashMap<u32, u32> = pids
        .iter()
        .filter_map(|pid| Some((*pid, stat(*pid)?.parent)))
        .collect();
    let below_provider = |mut pid: u32| {
        for _ in 0..64 {
            if provider_pids.contains(&pid) {
                return true;
            }
            match parents.get(&pid) {
                Some(parent) if *parent > 1 => pid = *parent,
                _ => return false,
            }
        }
        true
    };
    let mut found: Vec<Background> = pids
        .into_iter()
        .filter(|pid| *pid != own && !skip.contains(pid) && !provider_pids.contains(pid))
        .filter_map(|pid| {
            let identity = stat(pid).filter(|s| !s.ended)?;
            (environment_has(pid, "CHARIOX_PROVIDER_RUN_ID", Some(run))
                && !environment_has(pid, super::agent_process_group::WAKE_MARKER_ENV, None)
                && !below_provider(pid))
            .then(|| Background {
                pid,
                start_ticks: identity.start_ticks,
                command: command_line(pid),
            })
        })
        .collect();
    found.sort_by_key(|background| (background.start_ticks, background.pid));
    found
}

pub(super) struct AdoptedProcessDriver {
    sink: WakeSink,
    /// The wakes being looked at, each with the flag that ends the look.
    watching: std::sync::Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl AdoptedProcessDriver {
    fn detail(wake: &AgentWake) -> Result<AdoptedDetail, DaemonError> {
        serde_json::from_value(wake.detail.clone()).map_err(|_| {
            crate::durable_state::agent_lifecycle::error(
                "adopted process wake detail is unreadable",
            )
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<AtomicBool>>> {
        self.watching.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Starts looking; false when no runtime is there to look from.
    fn watch(&self, wake: &AgentWake, detail: AdoptedDetail) -> bool {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return false;
        };
        let stop = Arc::new(AtomicBool::new(false));
        self.lock().insert(wake.id.clone(), stop.clone());
        let sink = self.sink.clone();
        let id = wake.id.clone();
        handle.spawn(async move {
            while !stop.load(Ordering::Acquire) && running(detail.pid, detail.start_ticks) {
                tokio::time::sleep(POLL).await;
            }
            if stop.load(Ordering::Acquire) {
                return;
            }
            let occurrence = WakeOccurrence {
                occurrence_id: format!("{id}-ended"),
                wake_id: id,
                kind: "background_command_ended".into(),
                payload: serde_json::json!({
                    "pid": detail.pid,
                    "command": detail.command,
                    "exit_code": null,
                    "note": "the kernel adopted this command when your turn ended and did not start it, so its exit code and output are not known; read the files it wrote",
                }),
                summary: Some(format!("`{}` ended", detail.command)),
                end: Some(WakeEnd::Succeeded),
            };
            // The command is gone: this occurrence is the only copy of its end.
            while !stop.load(Ordering::Acquire) && sink.awaits_occurrence(&occurrence.wake_id) {
                if sink.occurred(occurrence.clone()).is_ok() {
                    return;
                }
                tokio::time::sleep(COMMIT_RETRY_AFTER).await;
            }
        });
        true
    }
}

impl WakeDriver for AdoptedProcessDriver {
    fn kind(&self) -> &'static str {
        KIND
    }

    fn arm(&self, wake: &AgentWake) -> Result<(), DaemonError> {
        let detail = Self::detail(wake)?;
        if !running(detail.pid, detail.start_ticks) {
            return Err(crate::durable_state::agent_lifecycle::error(
                "the command ended before it was adopted",
            ));
        }
        self.sink.verified(&wake.id)?;
        if !self.watch(wake, detail) {
            return Err(crate::durable_state::agent_lifecycle::error(
                "no runtime is there to look at the command",
            ));
        }
        Ok(())
    }

    fn cancel(&self, wake: &AgentWake) -> Result<Cancel, DaemonError> {
        if let Some(stop) = self.lock().remove(&wake.id) {
            stop.store(true, Ordering::Release);
        }
        Ok(Cancel::Done)
    }

    /// The kernel's own restart does not end an agent's detached command: a command still the same
    /// process is watched again, any other is reported as lost (it ended while nobody looked).
    fn recover(&self, wake: &AgentWake) -> Recovery {
        match Self::detail(wake) {
            Ok(detail) if running(detail.pid, detail.start_ticks) => {
                if self.watch(wake, detail) {
                    Recovery::Reattached
                } else {
                    Recovery::Lost("no runtime was there to look at the command".into())
                }
            }
            _ => Recovery::Lost(
                "the command ended while the kernel was not watching, or its identity is gone"
                    .into(),
            ),
        }
    }

    fn liveness(&self, wake: &AgentWake) -> Liveness {
        // A command that is gone is reported by its watcher on the next look.
        if self.lock().contains_key(&wake.id) {
            Liveness::Live
        } else {
            Liveness::Lost("this kernel no longer looks at the command".into())
        }
    }
}

impl KernelRuntimeState {
    /// Makes an agent's still-running background commands a wake source.
    pub(super) fn register_adopted_process_wake_driver(&mut self) {
        let driver: Arc<dyn WakeDriver> = Arc::new(AdoptedProcessDriver {
            sink: self.wake_sink(KIND),
            watching: Default::default(),
        });
        self.owned
            .agent_wakes
            .drivers
            .register_weak(Arc::downgrade(&driver));
        self.adopted_process_wake_driver = Some(driver);
    }
}
