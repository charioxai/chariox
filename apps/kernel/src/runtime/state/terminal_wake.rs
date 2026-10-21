//! MP-08 / MP-10 / MP-11 A03: the Room terminal as a wake source. An agent
//! that asked to be told when a terminal finishes, prints a text or waits for
//! input is woken through the same wake registry as its timers; this driver
//! only arms the terminal's watch and reports the one event that fired.
use super::wake_driver::{Cancel, Liveness, Recovery, WakeDriver, WakeOccurrence, WakeSink};
use super::*;
use crate::durable_state::agent_lifecycle::{
    self as ledger, AgentTaskExecution, AgentWake, WakeEnd,
};
use crate::room_terminal::{
    CompletionEvents, RoomTerminalEvent, RoomTerminalEventKind, RoomTerminalService,
    RoomTerminalStatus, RoomTerminalWatch, WatchOwner,
};

pub(super) const KIND: &str = "terminal";
/// The end of the screen an agent is woken with.
const SCREEN_BYTES: usize = 16 * 1024;

/// What a terminal wake watches, kept in the wake's `detail`.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct TerminalWakeDetail {
    pub terminal_id: String,
    pub until: Vec<RoomTerminalWatch>,
    pub after_offset: Option<u64>,
}

pub(super) struct TerminalWakeDriver {
    terminals: RoomTerminalService,
    sink: WakeSink,
}

impl TerminalWakeDriver {
    fn detail(wake: &AgentWake) -> Result<TerminalWakeDetail, DaemonError> {
        serde_json::from_value(wake.detail.clone())
            .map_err(|_| ledger::error("terminal wake detail is unreadable"))
    }

    fn terminal_is_running(&self, wake: &AgentWake, terminal_id: &str) -> bool {
        self.terminals
            .list(&wake.room_id)
            .iter()
            .any(|t| t.terminal_id == terminal_id && t.status == RoomTerminalStatus::Running)
    }
}

impl WakeDriver for TerminalWakeDriver {
    fn kind(&self) -> &'static str {
        KIND
    }

    fn arm(&self, wake: &AgentWake) -> Result<(), DaemonError> {
        let detail = Self::detail(wake)?;
        if !self.terminal_is_running(wake, &detail.terminal_id) {
            return Err(ledger::error(
                "no running terminal with that id in this Room on this kernel; use terminal_wait for a terminal on another machine",
            ));
        }
        self.sink.verified(&wake.id)?;
        self.terminals.watch(
            &wake.room_id,
            &detail.terminal_id,
            WatchOwner {
                wake_id: wake.id.clone(),
                agent_id: wake.agent_id.clone(),
            },
            detail.until,
            detail.after_offset,
        )
    }

    fn cancel(&self, wake: &AgentWake) -> Result<Cancel, DaemonError> {
        let detail = Self::detail(wake)?;
        self.terminals
            .cancel_watch(&wake.room_id, &detail.terminal_id, &wake.id);
        Ok(Cancel::Done)
    }

    /// Terminals end with a kernel restart, so a watch armed before it has
    /// nothing left to watch.
    fn recover(&self, _wake: &AgentWake) -> Recovery {
        Recovery::Lost("the terminal ended when the kernel restarted".into())
    }

    fn liveness(&self, wake: &AgentWake) -> Liveness {
        match Self::detail(wake) {
            Ok(detail) if self.terminal_is_running(wake, &detail.terminal_id) => Liveness::Live,
            _ => Liveness::Lost("the terminal was closed or ended".into()),
        }
    }
}

impl CompletionEvents for TerminalWakeDriver {
    fn deliver(&self, event: RoomTerminalEvent) {
        let (name, summary, mut payload) = describe(&event);
        payload["terminal_id"] = event.terminal_id.clone().into();
        payload["screen_text"] = tail_of(&event.screen_text, SCREEN_BYTES).into();
        let occurrence = WakeOccurrence {
            occurrence_id: format!("{}-{name}", event.wake_id),
            wake_id: event.wake_id,
            kind: format!("terminal_{name}"),
            payload,
            summary: Some(summary),
            end: Some(WakeEnd::Succeeded),
        };
        if let Err(error) = self.sink.occurred(occurrence) {
            crate::logging::error_with_fields(
                "room_terminal.event",
                "a terminal event could not wake its agent",
                serde_json::json!({ "terminal_id": event.terminal_id, "error": error.to_string() }),
            );
        }
    }
}

fn describe(event: &RoomTerminalEvent) -> (&'static str, String, serde_json::Value) {
    let terminal = &event.terminal_id;
    match &event.kind {
        RoomTerminalEventKind::Finished { exit_code, signal } => {
            let outcome = match (exit_code, signal) {
                (Some(code), _) => format!("exit code {code}"),
                (None, Some(signal)) => format!("signal {signal}"),
                (None, None) => "exit code not reported".to_string(),
            };
            (
                "finished",
                format!("Terminal {terminal}: the program finished ({outcome})"),
                serde_json::json!({"event": "finished", "exit_code": exit_code, "signal": signal}),
            )
        }
        RoomTerminalEventKind::OutputMatched { text } => (
            "output_matched",
            format!("Terminal {terminal}: output contains the awaited text"),
            serde_json::json!({"event": "output_matched", "text": text}),
        ),
        RoomTerminalEventKind::WaitingForInput => (
            "waiting_for_input",
            format!("Terminal {terminal}: the program waits for input"),
            serde_json::json!({"event": "waiting_for_input"}),
        ),
    }
}

fn tail_of(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        return text;
    }
    let mut start = text.len() - limit;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

impl KernelRuntimeState {
    /// Makes terminals a wake source: the driver arms watches and its sink
    /// receives the terminal's events.
    pub(super) fn register_terminal_wake_driver(&self) {
        let driver = Arc::new(TerminalWakeDriver {
            terminals: self.owned.room_terminals.clone(),
            sink: self.wake_sink(KIND),
        });
        self.owned.room_terminals.bind_events(driver.clone());
        self.register_wake_driver(driver);
    }

    /// Arms a terminal wake for the running turn of `task`.
    pub(super) fn arm_terminal_wake(
        &self,
        task: &AgentTaskExecution,
        mut wake: AgentWake,
        detail: TerminalWakeDetail,
    ) -> Result<serde_json::Value, DaemonError> {
        self.ensure_room_agent(&task.room_id, &task.agent_id)?;
        wake.state = "starting".into();
        wake.detail = serde_json::to_value(&detail)
            .map_err(|_| ledger::error("terminal wake detail could not be encoded"))?;
        self.arm_wake_source(task, &wake)?;
        self.refresh_wake_projection(&task.room_id);
        Ok(serde_json::json!({
            "wake_id": wake.id,
            "registration_id": wake.registration_id,
            "terminal_id": detail.terminal_id,
            "next_action": "Yield on the registration_id; the kernel wakes this agent once with the exit code or the matched text and the screen.",
        }))
    }
}
