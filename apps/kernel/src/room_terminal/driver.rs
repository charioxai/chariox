//! The one-driver rule, decided from the kernel's side of the PTY.
//!
//! At the prompt any Room agent can type. A foreground program (foreground
//! process group other than the shell) belongs to the actor who submitted the
//! command that started it until it exits. The user can always type.

use std::time::{Duration, Instant};

use super::types::{RoomTerminalDriver, TerminalActor};

/// A command may take this long to become the foreground process group.
pub(super) const LAUNCH_SETTLE: Duration = Duration::from_millis(250);

#[derive(Default)]
pub(super) struct DriverTracker {
    last_submit: Option<(TerminalActor, Instant)>,
    program_owner: Option<TerminalActor>,
}

pub(super) enum Admission {
    Admit,
    Refuse(RoomTerminalDriver),
    /// A different actor just submitted a command that may not have started
    /// yet: look again shortly.
    Settle,
}

impl DriverTracker {
    /// A program the terminal was opened with belongs to its opener.
    pub(super) fn owned_by(actor: TerminalActor) -> Self {
        Self {
            last_submit: None,
            program_owner: Some(actor),
        }
    }

    /// A command was just submitted at the prompt and may not be the
    /// foreground program yet.
    pub(super) fn launching(&self, now: Instant) -> bool {
        self.last_submit
            .as_ref()
            .is_some_and(|(_, at)| now.duration_since(*at) < LAUNCH_SETTLE)
    }

    pub(super) fn observe(&mut self, shell_is_foreground: bool) -> RoomTerminalDriver {
        if shell_is_foreground {
            self.program_owner = None;
            return RoomTerminalDriver::Free;
        }
        let owner = self.program_owner.get_or_insert_with(|| {
            self.last_submit
                .as_ref()
                .map(|(actor, _)| actor.clone())
                .unwrap_or(TerminalActor::User)
        });
        RoomTerminalDriver::Busy {
            driven_by: owner.clone(),
        }
    }

    pub(super) fn admit(
        &mut self,
        actor: &TerminalActor,
        shell_is_foreground: bool,
        now: Instant,
    ) -> Admission {
        if *actor == TerminalActor::User {
            return Admission::Admit;
        }
        if shell_is_foreground {
            if let Some((submitter, at)) = &self.last_submit {
                if submitter != actor && now.duration_since(*at) < LAUNCH_SETTLE {
                    return Admission::Settle;
                }
            }
        }
        match self.observe(shell_is_foreground) {
            RoomTerminalDriver::Free => Admission::Admit,
            RoomTerminalDriver::Busy { driven_by } if driven_by == *actor => Admission::Admit,
            busy => Admission::Refuse(busy),
        }
    }

    /// Remember who submitted a command at the prompt.
    pub(super) fn record_write(
        &mut self,
        actor: &TerminalActor,
        shell_is_foreground: bool,
        input: &[u8],
        now: Instant,
    ) {
        if shell_is_foreground && input.iter().any(|byte| matches!(byte, b'\r' | b'\n')) {
            self.last_submit = Some((actor.clone(), now));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str) -> TerminalActor {
        TerminalActor::Agent(id.to_string())
    }

    #[test]
    fn foreground_program_belongs_to_the_agent_that_started_it() {
        let mut tracker = DriverTracker::default();
        let start = Instant::now();
        assert!(matches!(
            tracker.admit(&agent("a"), true, start),
            Admission::Admit
        ));
        tracker.record_write(&agent("a"), true, b"python3\n", start);
        let later = start + LAUNCH_SETTLE * 2;
        assert!(matches!(
            tracker.admit(&agent("b"), false, later),
            Admission::Refuse(RoomTerminalDriver::Busy { driven_by }) if driven_by == agent("a")
        ));
        assert!(matches!(
            tracker.admit(&agent("a"), false, later),
            Admission::Admit
        ));
        assert!(matches!(
            tracker.admit(&TerminalActor::User, false, later),
            Admission::Admit
        ));
        assert!(matches!(
            tracker.admit(&agent("b"), true, later),
            Admission::Admit
        ));
    }

    #[test]
    fn another_actor_waits_for_a_command_that_is_still_launching() {
        let mut tracker = DriverTracker::default();
        let start = Instant::now();
        tracker.record_write(&agent("a"), true, b"python3\r", start);
        assert!(matches!(
            tracker.admit(&agent("b"), true, start),
            Admission::Settle
        ));
    }

    #[test]
    fn a_program_the_user_started_is_driven_by_the_user() {
        let mut tracker = DriverTracker::default();
        let start = Instant::now();
        tracker.record_write(&TerminalActor::User, true, b"top\r", start);
        assert!(matches!(
            tracker.admit(&agent("a"), false, start + LAUNCH_SETTLE * 2),
            Admission::Refuse(RoomTerminalDriver::Busy {
                driven_by: TerminalActor::User
            })
        ));
    }
}
