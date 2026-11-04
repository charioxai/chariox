//! What a command's wait tells the outside. The `Waiting` row is the one
//! description: the line the command's own error stream gets, the event the
//! room's followers get and the person's notice after a long wait all say what
//! the row says.
use super::projection::{SessionProcessState, Waiting};
use super::{admission::WaitReason, MIB};
use std::collections::VecDeque;
use std::sync::Mutex;

/// A command that has waited this long also reaches the person.
pub(super) const LONG_WAIT_MS: u64 = 120_000;
/// Events kept until the runtime takes them; the oldest go first.
const KEPT: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WaitChange {
    Began,
    Resumed {
        waited_ms: u64,
    },
    /// Still waiting after `LONG_WAIT_MS`.
    Long,
    /// The pool ended the command, for this reason.
    Stopped {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WaitEvent {
    /// The keys of the workload the command runs in: they find its owner.
    pub keys: Vec<String>,
    pub process_id: String,
    pub change: WaitChange,
    /// The row at that moment (`Running` once it continues).
    pub state: SessionProcessState,
}

static EVENTS: Mutex<VecDeque<WaitEvent>> = Mutex::new(VecDeque::new());

#[cfg(test)]
pub(crate) fn push_for_tests(event: WaitEvent) {
    push(event);
}

pub(super) fn push(event: WaitEvent) {
    let mut events = EVENTS.lock().unwrap_or_else(|e| e.into_inner());
    if events.len() >= KEPT {
        events.pop_front();
    }
    events.push_back(event);
}

/// The waits that began, ended or grew long since the last call, oldest first.
pub(crate) fn take_wait_events() -> Vec<WaitEvent> {
    EVENTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .drain(..)
        .collect()
}

fn resource(reason: WaitReason) -> &'static str {
    match reason {
        WaitReason::Memory => "memory",
        WaitReason::Disk => "disk space",
    }
}

fn ordinal(n: u32) -> String {
    let suffix = match (n % 100, n % 10) {
        (11..=13, _) => "th",
        (_, 1) => "st",
        (_, 2) => "nd",
        (_, 3) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

fn gigabytes(bytes: u64) -> String {
    format!("{} GB", (bytes / (1024 * MIB)).max(1))
}

fn duration(ms: u64) -> String {
    match ms / 1000 {
        seconds @ 0..=59 => format!("{seconds} s"),
        seconds => format!("{} min {} s", seconds / 60, seconds % 60),
    }
}

/// What the waiting command is told, on its own error stream.
pub(super) fn began_line(state: &SessionProcessState) -> Option<String> {
    let (what, row, stays) = match state {
        SessionProcessState::Held(row) => ("is waiting to start", row, ""),
        // Work that already started cannot move to another machine.
        SessionProcessState::Paused(row) => (
            "is paused while it grows",
            row,
            " It already started, so it stays on this machine.",
        ),
        SessionProcessState::Running => return None,
    };
    Some(format!(
        "chariox: this command {what} for {} ({} in line, about {} needed, {} free). It continues by itself when there is room; Start now in Machines runs it at once, Cancel ends it.{stays}\n",
        resource(row.reason),
        ordinal(row.position.max(1)),
        gigabytes(row.need_bytes),
        gigabytes(row.free_bytes),
    ))
}

pub(super) fn resumed_line(waited_ms: u64) -> String {
    format!(
        "chariox: this command continues after waiting {}.\n",
        duration(waited_ms)
    )
}

/// What the person is told when a wait grows long.
pub(crate) fn long_wait_notice(row: &Waiting, now_ms: u64) -> String {
    format!(
        "{} has waited {} for {}. It continues by itself; Start now in Machines runs it at once, Cancel ends it.",
        row.command_line,
        duration(now_ms.saturating_sub(row.since_ms)),
        resource(row.reason),
    )
}
