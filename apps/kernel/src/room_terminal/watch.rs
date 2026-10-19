//! Completion watches: the kernel side of the wake sources an agent asks for.
//! Each watch fires once, then is removed.

use std::time::Duration;

use super::plain_text::PlainText;
use super::types::{RoomTerminalEventKind, RoomTerminalWatch};

/// A foreground program this quiet is taken to wait for input.
pub(super) const QUIET_BEFORE_WAITING: Duration = Duration::from_secs(3);

struct ArmedWatch {
    armed_by: String,
    watch: RoomTerminalWatch,
    armed_at_plain_bytes: u64,
    /// A program has been in the foreground since arming (for `Finished`).
    seen_busy: bool,
}

pub(super) type Fired = Vec<(String, RoomTerminalEventKind)>;

#[derive(Default)]
pub(super) struct WatchSet {
    armed: Vec<ArmedWatch>,
}

impl WatchSet {
    pub(super) fn arm(
        &mut self,
        armed_by: String,
        watch: RoomTerminalWatch,
        plain: &PlainText,
        program_running: bool,
    ) {
        self.armed.push(ArmedWatch {
            armed_by,
            watch,
            armed_at_plain_bytes: plain.total_bytes(),
            seen_busy: program_running,
        });
    }

    pub(super) fn on_output(&mut self, plain: &PlainText) -> Fired {
        self.fire(|armed| match &armed.watch {
            RoomTerminalWatch::OutputContains(text)
                if plain.since(armed.armed_at_plain_bytes).contains(text.as_str()) =>
            {
                Some(RoomTerminalEventKind::OutputMatched { text: text.clone() })
            }
            _ => None,
        })
    }

    pub(super) fn on_foreground(&mut self, program_running: bool) -> Fired {
        for armed in &mut self.armed {
            armed.seen_busy |= program_running;
        }
        if program_running {
            return Fired::new();
        }
        self.fire(|armed| match armed.watch {
            RoomTerminalWatch::Finished if armed.seen_busy => {
                Some(RoomTerminalEventKind::Finished {
                    exit_code: None,
                    signal: None,
                })
            }
            _ => None,
        })
    }

    pub(super) fn on_quiet_program(&mut self) -> Fired {
        self.fire(|armed| match armed.watch {
            RoomTerminalWatch::WaitingForInput => Some(RoomTerminalEventKind::WaitingForInput),
            _ => None,
        })
    }

    pub(super) fn on_shell_exit(&mut self, exit_code: Option<u32>, signal: Option<String>) -> Fired {
        self.fire(|armed| match armed.watch {
            RoomTerminalWatch::Finished => Some(RoomTerminalEventKind::Finished {
                exit_code,
                signal: signal.clone(),
            }),
            _ => None,
        })
    }

    pub(super) fn is_waiting_for_input(&self) -> bool {
        self.armed
            .iter()
            .any(|armed| armed.watch == RoomTerminalWatch::WaitingForInput)
    }

    fn fire(
        &mut self,
        mut decide: impl FnMut(&ArmedWatch) -> Option<RoomTerminalEventKind>,
    ) -> Fired {
        let mut fired = Fired::new();
        self.armed.retain(|armed| match decide(armed) {
            Some(kind) => {
                fired.push((armed.armed_by.clone(), kind));
                false
            }
            None => true,
        });
        fired
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_watch_ignores_text_from_before_arming_and_fires_once() {
        let mut plain = PlainText::new();
        plain.append(b"Password: ");
        let mut watches = WatchSet::default();
        watches.arm(
            "agent-a".into(),
            RoomTerminalWatch::OutputContains("Password:".into()),
            &plain,
            false,
        );
        assert!(watches.on_output(&plain).is_empty());
        plain.append(b"\nPassword: ");
        let fired = watches.on_output(&plain);
        assert_eq!(fired.len(), 1);
        assert!(watches.on_output(&plain).is_empty());
    }

    #[test]
    fn finished_waits_for_a_program_to_return_to_the_prompt() {
        let plain = PlainText::new();
        let mut watches = WatchSet::default();
        watches.arm("agent-a".into(), RoomTerminalWatch::Finished, &plain, false);
        assert!(watches.on_foreground(false).is_empty());
        assert!(watches.on_foreground(true).is_empty());
        assert_eq!(watches.on_foreground(false).len(), 1);
    }
}
