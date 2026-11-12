//! What a command's wait tells the outside. The `Waiting` row is the one
//! description: the line the command's own error stream gets, the event the
//! room's followers get all say what
//! the row says.
use super::projection::SessionProcessState;
use super::{admission::WaitReason, MIB};
use std::collections::VecDeque;
use std::sync::Mutex;

/// Events kept until the runtime takes them; the oldest go first.
const KEPT: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WaitChange {
    Began,
    Resumed {
        waited_ms: u64,
    },
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

/// The waits that began or ended since the last call, oldest first.
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

/// Sizes in the lines a command, an agent and a person read: gigabytes with
/// one decimal from one gigabyte up (and for none), megabytes below. 3.37 and
/// 2.4 do not both round to a misleading whole number, and 0 reads "0 GB".
pub(crate) fn size(bytes: u64) -> String {
    if bytes != 0 && bytes < 1024 * MIB {
        return format!("{} MB", (bytes + MIB / 2) / MIB);
    }
    let tenths = (bytes * 10 + 512 * MIB) / (1024 * MIB);
    match tenths % 10 {
        0 => format!("{} GB", tenths / 10),
        decimal => format!("{}.{decimal} GB", tenths / 10),
    }
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
        size(row.need_bytes),
        size(row.free_bytes),
    ))
}

/// Why a command filling the disk was ended, on its own error stream.
pub(super) fn filling_line(filling: &super::admission::Filling) -> String {
    let deleted = if filling.deleted_open > 0 {
        format!(
            " It also held {} in deleted files it kept open; they are freed with it.",
            size(filling.deleted_open)
        )
    } else {
        String::new()
    };
    format!(
        "chariox: stopped this command: the disk has {} free (the floor is {}) and this command was filling it, writing {}/s ({} written so far).{deleted} Free space or move its output elsewhere, then run it again.\n",
        size(filling.free),
        size(filling.floor),
        size(filling.rate),
        size(filling.written),
    )
}

/// Why a paused command that stayed blocked was ended, on its own error
/// stream: the room that was left against what it needed, for the resource
/// that decided. `ceiling` is the pool's limit when that bound the memory
/// room, `None` when the machine's free memory did.
pub(super) fn overdue_line(
    waited_ms: u64,
    blocked: &super::admission::Blocked,
    resident: u64,
    ceiling: Option<u64>,
) -> String {
    let waited = waited_ms / 1000;
    match *blocked {
        super::admission::Blocked::Memory { left, need } => {
            let bound = match ceiling {
                Some(ceiling) => format!(
                    "under the {} shared ceiling once what is in use and reserved is counted",
                    size(ceiling)
                ),
                None => "of the machine's free memory".to_string(),
            };
            format!(
                "chariox: stopped this waiting command: after waiting {waited} s it still needs {} to continue and only {} are left {bound} ({} resident). Run it again; it starts when there is room.\n",
                size(need),
                size(left),
                size(resident),
            )
        }
        super::admission::Blocked::Disk { spare, need } => format!(
            "chariox: stopped this waiting command: after waiting {waited} s it still needs {} of disk above the floor to continue and only {} are free on the volume it writes to. Free space or move its output elsewhere, then run it again.\n",
            size(need),
            size(spare),
        ),
    }
}

pub(super) fn resumed_line(waited_ms: u64) -> String {
    format!(
        "chariox: this command continues after waiting {}.\n",
        duration(waited_ms)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_in_gigabytes_with_a_decimal_megabytes_below_and_zero_stays_zero() {
        assert_eq!(size(0), "0 GB");
        assert_eq!(size(8 * MIB), "8 MB");
        assert_eq!(size(2 * 1024 * MIB), "2 GB");
        assert_eq!(size(3 * 1024 * MIB + 380 * MIB), "3.4 GB");
        assert_eq!(size(2 * 1024 * MIB + 410 * MIB), "2.4 GB");
    }
}
