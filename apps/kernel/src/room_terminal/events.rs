//! Completion events: a program finished, output contains a text, a program
//! waits for input. The kernel's event core arms a watch per wake and
//! receives its event through `CompletionEvents`; nothing else in this
//! module knows about wakes.

use std::sync::{RwLock, Weak};
use std::time::Duration;

use super::plain_text::PlainText;
use super::terminal::ProgramEnd;
use super::types::{RoomTerminalEvent, RoomTerminalEventKind, RoomTerminalWatch};

/// A foreground program this quiet is taken to wait for input.
pub(super) const QUIET_BEFORE_WAITING: Duration = Duration::from_secs(3);

/// Where fired watches go. Called without any terminal lock held.
pub trait CompletionEvents: Send + Sync {
    fn deliver(&self, event: RoomTerminalEvent);
}

/// The event core's sink, bound when the runtime exists. A terminal event
/// before that has nobody to wake, so it is logged as lost. A router built
/// again on the same app (tests do) takes over the binding: the newest
/// runtime owns terminal wakes.
#[derive(Default)]
pub(super) struct BoundEvents(RwLock<Option<Weak<dyn CompletionEvents>>>);

impl BoundEvents {
    pub(super) fn bind(&self, events: Weak<dyn CompletionEvents>) {
        *self.0.write().unwrap_or_else(|e| e.into_inner()) = Some(events);
    }

    pub(super) fn deliver(&self, event: RoomTerminalEvent) {
        let bound = self
            .0
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(Weak::upgrade);
        match bound {
            Some(events) => events.deliver(event),
            None => crate::logging::error_with_fields(
                "room_terminal.event",
                "terminal event dropped: the event core is not bound to Room terminals",
                serde_json::json!({ "terminal_id": event.terminal_id, "wake_id": event.wake_id }),
            ),
        }
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
    output: OutputMatcher,
    /// A program has been in the foreground since arming (for `Finished`).
    seen_busy: bool,
    binding: FinishBinding,
}

/// Which command a `Finished` watch is about.
#[derive(Clone, Copy, Default)]
pub(super) struct FinishBinding {
    /// The number of the owner's latest line: it is the command with that
    /// number, or a later one, so an earlier command that ends later is not
    /// taken for it.
    pub line: Option<u64>,
    /// The output offset the owner had read when it armed the watch.
    pub after_offset: Option<u64>,
}

impl ArmedWatch {
    fn counts_command(&self, command: Option<u64>) -> bool {
        match (self.binding.line, command) {
            (Some(line), Some(command)) => command >= line,
            _ => true,
        }
    }
}

/// Incremental text matching, shared by live watches and retained-log replay.
pub(super) struct OutputMatcher {
    processed_plain_bytes: u64,
    text_tail: String,
}

impl OutputMatcher {
    pub(super) fn new(plain: &PlainText) -> Self {
        Self {
            processed_plain_bytes: plain.total_bytes(),
            text_tail: String::new(),
        }
    }

    /// Text that may be the start of a match, kept from output already seen
    /// when the matcher starts at the end of `plain`.
    pub(super) fn after(plain: &PlainText, tail: String) -> Self {
        Self {
            processed_plain_bytes: plain.total_bytes(),
            text_tail: tail,
        }
    }

    pub(super) fn into_tail(self) -> String {
        self.text_tail
    }

    pub(super) fn matches(&mut self, plain: &PlainText, text: &str) -> bool {
        self.text_tail
            .push_str(plain.since(self.processed_plain_bytes));
        self.processed_plain_bytes = plain.total_bytes();
        if self.text_tail.contains(text) {
            return true;
        }
        let mut cut = self
            .text_tail
            .len()
            .saturating_sub(text.len().saturating_sub(1));
        while !self.text_tail.is_char_boundary(cut) {
            cut += 1;
        }
        self.text_tail.drain(..cut);
        false
    }
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
        output: OutputMatcher,
        program_running: bool,
        binding: FinishBinding,
    ) {
        self.armed.push(ArmedWatch {
            owner,
            watch,
            output,
            seen_busy: program_running,
            binding,
        });
    }

    pub(super) fn on_output(&mut self, plain: &PlainText) -> Fired {
        self.fire(|armed| {
            let RoomTerminalWatch::OutputContains { text } = &armed.watch else {
                return None;
            };
            armed
                .output
                .matches(plain, text)
                .then(|| RoomTerminalEventKind::OutputMatched { text: text.clone() })
        })
    }

    /// `ended_command` is the number of the program that just returned to the
    /// prompt.
    pub(super) fn on_foreground(
        &mut self,
        program_running: bool,
        ended_command: Option<u64>,
    ) -> Fired {
        for armed in &mut self.armed {
            armed.seen_busy |= program_running;
        }
        if program_running {
            return Fired::new();
        }
        self.fire(|armed| match armed.watch {
            RoomTerminalWatch::Finished
                if armed.seen_busy && armed.counts_command(ended_command) =>
            {
                Some(RoomTerminalEventKind::Finished {
                    exit_code: None,
                    signal: None,
                })
            }
            _ => None,
        })
    }

    /// The shell reported a command line finished.
    pub(super) fn on_command_finished(
        &mut self,
        exit_code: Option<u32>,
        command: Option<u64>,
    ) -> Fired {
        self.fire(|armed| match armed.watch {
            RoomTerminalWatch::Finished if armed.counts_command(command) => {
                Some(RoomTerminalEventKind::Finished {
                    exit_code,
                    signal: None,
                })
            }
            _ => None,
        })
    }

    /// Lines the shell never ran as commands left some watches waiting for a
    /// command that will not start (`commands_started` is all there is). They
    /// now follow the last command that did, which has already ended.
    pub(super) fn on_lines_settled(
        &mut self,
        commands_started: u64,
        last_end: Option<&ProgramEnd>,
    ) -> Fired {
        for armed in &mut self.armed {
            if let Some(line) = &mut armed.binding.line {
                *line = (*line).min(commands_started);
            }
        }
        let Some(end) = last_end else {
            return Fired::new();
        };
        self.fire(|armed| match armed.watch {
            RoomTerminalWatch::Finished
                if armed.counts_command(end.command)
                    && armed
                        .binding
                        .after_offset
                        .is_some_and(|offset| end.output_offset > offset) =>
            {
                Some(RoomTerminalEventKind::Finished {
                    exit_code: end.exit_code,
                    signal: end.signal.clone(),
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

    pub(super) fn on_outside_end(&mut self) -> Fired {
        self.fire(|_| Some(RoomTerminalEventKind::OutsideEnd))
    }

    pub(super) fn on_memory_limit(&mut self, limit_bytes: u64) -> Fired {
        self.fire(|_| Some(RoomTerminalEventKind::MemoryLimit { limit_bytes }))
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
        mut decide: impl FnMut(&mut ArmedWatch) -> Option<RoomTerminalEventKind>,
    ) -> Fired {
        let mut fired = Fired::new();
        for armed in &mut self.armed {
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
            OutputMatcher::new(&plain),
            false,
            FinishBinding::default(),
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
        watches.arm(
            owner("w1"),
            RoomTerminalWatch::Finished,
            OutputMatcher::new(&plain),
            false,
            FinishBinding::default(),
        );
        assert!(watches.on_foreground(false, None).is_empty());
        assert!(watches.on_foreground(true, None).is_empty());
        assert_eq!(watches.on_foreground(false, None).len(), 1);
    }

    #[test]
    fn a_wake_fires_once_even_with_several_conditions() {
        let mut plain = PlainText::new();
        let mut watches = WatchSet::default();
        watches.arm(
            owner("w1"),
            RoomTerminalWatch::WaitingForInput,
            OutputMatcher::new(&plain),
            true,
            FinishBinding::default(),
        );
        watches.arm(
            owner("w1"),
            RoomTerminalWatch::OutputContains {
                text: "ready".into(),
            },
            OutputMatcher::new(&plain),
            true,
            FinishBinding::default(),
        );
        plain.append(b"ready");
        assert_eq!(watches.on_output(&plain).len(), 1);
        assert!(watches.on_quiet_program().is_empty());
        assert!(!watches.is_waiting_for_input());
    }
}
