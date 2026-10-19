//! Completion events: a program finished, output contains a text, a program
//! waits for input. The kernel's event core arms a watch per wake and
//! receives its event through `CompletionEvents`; nothing else in this
//! module knows about wakes.

use std::time::Duration;

use super::plain_text::PlainText;
use super::types::{RoomTerminalEvent, RoomTerminalEventKind, RoomTerminalWatch};

/// A foreground program this quiet is taken to wait for input.
pub(super) const QUIET_BEFORE_WAITING: Duration = Duration::from_secs(3);

/// Where fired watches go. Called without any terminal lock held.
pub trait CompletionEvents: Send + Sync {
    fn deliver(&self, event: RoomTerminalEvent);
}

/// The sink until the event core is wired: no notification can be armed
/// through the protocol, so reaching it is a bug and says so.
pub struct EventCoreNotWired;

impl CompletionEvents for EventCoreNotWired {
    fn deliver(&self, event: RoomTerminalEvent) {
        crate::logging::error_with_fields(
            "room_terminal.event",
            "terminal event dropped: no event core is wired to Room terminals",
            serde_json::json!({ "terminal_id": event.terminal_id, "wake_id": event.wake_id }),
        );
    }
}

/// Who a fired watch wakes: the wake registration and its agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchOwner {
    pub wake_id: String,
    pub agent_id: String,
}

struct ArmedWatch {
    owner: WatchOwner,
    watch: RoomTerminalWatch,
    armed_at_plain_bytes: u64,
    /// A program has been in the foreground since arming (for `Finished`).
    seen_busy: bool,
}

pub(super) type Fired = Vec<(WatchOwner, RoomTerminalEventKind)>;

#[derive(Default)]
pub(super) struct WatchSet {
    armed: Vec<ArmedWatch>,
}

impl WatchSet {
    pub(super) fn arm(
        &mut self,
        owner: WatchOwner,
        watch: RoomTerminalWatch,
        plain: &PlainText,
        program_running: bool,
    ) {
        self.armed.push(ArmedWatch {
            owner,
            watch,
            armed_at_plain_bytes: plain.total_bytes(),
            seen_busy: program_running,
        });
    }

    pub(super) fn on_output(&mut self, plain: &PlainText) -> Fired {
        self.fire(|armed| match &armed.watch {
            RoomTerminalWatch::OutputContains { text }
                if plain
                    .since(armed.armed_at_plain_bytes)
                    .contains(text.as_str()) =>
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

    /// The shell reported a command line finished.
    pub(super) fn on_command_finished(&mut self, exit_code: Option<u32>) -> Fired {
        self.fire(|armed| match armed.watch {
            RoomTerminalWatch::Finished => Some(RoomTerminalEventKind::Finished {
                exit_code,
                signal: None,
            }),
            _ => None,
        })
    }

    pub(super) fn on_quiet_program(&mut self) -> Fired {
        self.fire(|armed| match armed.watch {
            RoomTerminalWatch::WaitingForInput => Some(RoomTerminalEventKind::WaitingForInput),
            _ => None,
        })
    }

    pub(super) fn on_shell_exit(
        &mut self,
        exit_code: Option<u32>,
        signal: Option<String>,
    ) -> Fired {
        self.fire(|armed| match armed.watch {
            RoomTerminalWatch::Finished => Some(RoomTerminalEventKind::Finished {
                exit_code,
                signal: signal.clone(),
            }),
            _ => None,
        })
    }

    /// Drop every watch of a wake that is no longer wanted: a wait that
    /// timed out, or a cancelled wake.
    pub(super) fn disarm(&mut self, wake_id: &str) {
        self.armed.retain(|armed| armed.owner.wake_id != wake_id);
    }

    pub(super) fn is_waiting_for_input(&self) -> bool {
        self.armed
            .iter()
            .any(|armed| armed.watch == RoomTerminalWatch::WaitingForInput)
    }

    /// A wake fires once: its other conditions are dropped with it.
    fn fire(
        &mut self,
        mut decide: impl FnMut(&ArmedWatch) -> Option<RoomTerminalEventKind>,
    ) -> Fired {
        let mut fired = Fired::new();
        for armed in &self.armed {
            if fired
                .iter()
                .any(|(owner, _)| owner.wake_id == armed.owner.wake_id)
            {
                continue;
            }
            if let Some(kind) = decide(armed) {
                fired.push((armed.owner.clone(), kind));
            }
        }
        for (owner, _) in &fired {
            self.disarm(&owner.wake_id);
        }
        fired
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(wake_id: &str) -> WatchOwner {
        WatchOwner {
            wake_id: wake_id.into(),
            agent_id: "agent-a".into(),
        }
    }

    #[test]
    fn output_watch_ignores_text_from_before_arming_and_fires_once() {
        let mut plain = PlainText::new();
        plain.append(b"Password: ");
        let mut watches = WatchSet::default();
        watches.arm(
            owner("w1"),
            RoomTerminalWatch::OutputContains {
                text: "Password:".into(),
            },
            &plain,
            false,
        );
        assert!(watches.on_output(&plain).is_empty());
        plain.append(b"\nPassword: ");
        assert_eq!(watches.on_output(&plain).len(), 1);
        assert!(watches.on_output(&plain).is_empty());
    }

    #[test]
    fn finished_waits_for_a_program_to_return_to_the_prompt() {
        let plain = PlainText::new();
        let mut watches = WatchSet::default();
        watches.arm(owner("w1"), RoomTerminalWatch::Finished, &plain, false);
        assert!(watches.on_foreground(false).is_empty());
        assert!(watches.on_foreground(true).is_empty());
        assert_eq!(watches.on_foreground(false).len(), 1);
    }

    #[test]
    fn a_wake_fires_once_even_with_several_conditions() {
        let mut plain = PlainText::new();
        let mut watches = WatchSet::default();
        watches.arm(
            owner("w1"),
            RoomTerminalWatch::WaitingForInput,
            &plain,
            true,
        );
        watches.arm(
            owner("w1"),
            RoomTerminalWatch::OutputContains {
                text: "ready".into(),
            },
            &plain,
            true,
        );
        plain.append(b"ready");
        assert_eq!(watches.on_output(&plain).len(), 1);
        assert!(watches.on_quiet_program().is_empty());
        assert!(!watches.is_waiting_for_input());
    }
}
