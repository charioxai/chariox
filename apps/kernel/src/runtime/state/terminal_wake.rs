//! MP-08 / MP-10 / MP-11 A03: the Room terminal as a wake source. An agent
//! that asked to be told when a terminal finishes, prints a text or waits for
//! input is woken through the same wake registry as its timers; this driver
//! only arms the terminal's watch and reports the one event that fired.
use super::terminal_wake_worker::{self as worker, Settled, Step, WorkerWatches};
use super::wake_driver::{Cancel, Liveness, Recovery, WakeDriver, WakeOccurrence, WakeSink};
use super::*;
use crate::durable_state::agent_lifecycle::{
    self as ledger, AgentTaskExecution, AgentWake, WakeEnd,
};
use crate::local::{RoomTerminalOperation, RoomTerminalResult};
use crate::room_terminal::{
    CompletionEvents, RoomTerminalEvent, RoomTerminalEventKind, RoomTerminalService,
    RoomTerminalStatus, RoomTerminalWatch, TerminalActor, WatchOwner, MAX_WAIT,
};

pub(super) const KIND: &str = "terminal";
/// The end of the screen an agent is woken with.
const SCREEN_BYTES: usize = 16 * 1024;
const COMMIT_RETRY_AFTER: std::time::Duration = std::time::Duration::from_millis(500);

/// What a terminal wake watches, kept in the wake's `detail`.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct TerminalWakeDetail {
    pub terminal_id: String,
    pub until: Vec<RoomTerminalWatch>,
    pub after_offset: Option<u64>,
    /// How much output the worker's terminal had when the wake was armed:
    /// supervising waits never replay below it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub armed_at_offset: Option<u64>,
    /// The Room agent whose leased machine runs the terminal; `None` is this
    /// kernel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_agent: Option<String>,
}

pub(super) struct TerminalWakeDriver {
    state: KernelRuntimeState,
    terminals: RoomTerminalService,
    sink: WakeSink,
    workers: Arc<WorkerWatches>,
}

impl TerminalWakeDriver {
    #[cfg(test)]
    pub(super) fn sink(&self) -> &WakeSink {
        &self.sink
    }

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
        if detail.host_agent.is_some() {
            self.sink.verified(&wake.id)?;
            self.watch_on_worker(wake.clone(), detail);
            return Ok(());
        }
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
        if detail.host_agent.is_some() {
            self.workers.stop(&wake.id);
            return Ok(Cancel::Done);
        }
        self.terminals
            .cancel_watch(&wake.room_id, &detail.terminal_id, &wake.id);
        Ok(Cancel::Done)
    }

    /// Terminals end with a kernel restart, so a watch armed before it has
    /// nothing left to watch. A worker's terminal survives the home, but the
    /// home's supervision of it does not.
    fn recover(&self, wake: &AgentWake) -> Recovery {
        match Self::detail(wake) {
            Ok(detail) if detail.host_agent.is_some() => Recovery::Lost(
                "it stopped watching the terminal on the worker machine; the terminal may still be running there, read it with terminal_read".into(),
            ),
            _ => Recovery::Lost("the terminal ended when the kernel restarted".into()),
        }
    }

    fn liveness(&self, wake: &AgentWake) -> Liveness {
        match Self::detail(wake) {
            Ok(detail) if detail.host_agent.is_some() => {
                if self.workers.is_supervised(&wake.id) {
                    Liveness::Live
                } else {
                    Liveness::Lost(
                        "this kernel no longer watches the terminal on the worker machine".into(),
                    )
                }
            }
            Ok(detail) if self.terminal_is_running(wake, &detail.terminal_id) => Liveness::Live,
            _ => Liveness::Lost("the terminal was closed or ended".into()),
        }
    }
}

impl CompletionEvents for TerminalWakeDriver {
    fn deliver(&self, event: RoomTerminalEvent) {
        report(&self.sink, event);
    }
}

/// The one occurrence of a fired watch; it ends the wake.
fn report(sink: &WakeSink, event: RoomTerminalEvent) {
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
    if let Err(error) = sink.occurred(occurrence.clone()) {
        crate::logging::error_with_fields(
            "room_terminal.event",
            "a terminal event could not wake its agent yet; retrying",
            serde_json::json!({ "terminal_id": event.terminal_id, "error": error.to_string() }),
        );
        retry_until_committed(sink.clone(), occurrence);
    }
}

/// The fired watch is gone from the terminal, so this occurrence is the only
/// copy of the completion: it is offered again, under the same occurrence id,
/// until the ledger commits it or the wake no longer awaits it.
fn retry_until_committed(sink: WakeSink, occurrence: WakeOccurrence) {
    // A committed occurrence schedules agent delivery on this runtime too.
    tokio::spawn(async move {
        while sink.awaits_occurrence(&occurrence.wake_id) {
            tokio::time::sleep(COMMIT_RETRY_AFTER).await;
            if sink.occurred(occurrence.clone()).is_ok() {
                return;
            }
        }
    });
}

impl TerminalWakeDriver {
    /// Watches a terminal on the agent's leased machine: the worker waits,
    /// the home reports what fired (or that the worker was lost).
    fn watch_on_worker(&self, wake: AgentWake, detail: TerminalWakeDetail) {
        let state = self.state.clone();
        let sink = self.sink.clone();
        self.workers.spawn(wake.id.clone(), async move {
            let overlap = worker::Overlap::for_texts(
                detail.after_offset.or(detail.armed_at_offset).unwrap_or(0),
                detail.until.iter().filter_map(|watch| match watch {
                    RoomTerminalWatch::OutputContains { text } => Some(text.as_str()),
                    _ => None,
                }),
            );
            let settled = worker::supervise(detail.after_offset, overlap, |offset| {
                state.wait_on_worker_terminal(&wake, &detail, offset)
            })
            .await;
            match settled {
                Settled::Fired(waited) => report(
                    &sink,
                    RoomTerminalEvent {
                        terminal_id: detail.terminal_id.clone(),
                        session_id: wake.room_id.clone(),
                        wake_id: wake.id.clone(),
                        armed_by: wake.agent_id.clone(),
                        kind: waited.fired.expect("a settled wait fired"),
                        screen_text: waited.screen_text,
                    },
                ),
                Settled::Lost(reason) => {
                    let now = crate::session::unix_epoch_ms();
                    if let Err(error) = state.report_lost_wake(&wake, now, &reason) {
                        crate::logging::error_with_fields(
                            "room_terminal.event",
                            "a lost terminal watch could not be reported",
                            serde_json::json!({ "wake_id": wake.id, "error": error.to_string() }),
                        );
                    }
                }
            }
        });
    }
}

impl KernelRuntimeState {
    /// Refuses at once, like a local arm, a terminal the worker does not run;
    /// otherwise returns how much output it has so far.
    async fn ensure_running_on_worker(
        &self,
        task: &AgentTaskExecution,
        host: &str,
        terminal_id: &str,
    ) -> Result<u64, DaemonError> {
        let listed = self
            .route_room_terminal(
                &task.room_id,
                TerminalActor::Agent(task.agent_id.clone()),
                Some(host),
                false,
                RoomTerminalOperation::List,
            )
            .await
            .map_err(|error| ledger::error(error.to_string()))?;
        let RoomTerminalResult::Listed { terminals } = listed else {
            return Err(ledger::error(
                "the worker answered a terminal list with something else",
            ));
        };
        terminals
            .iter()
            .find(|t| t.terminal_id == terminal_id && t.status == RoomTerminalStatus::Running)
            .map(|t| t.output_offset)
            .ok_or_else(|| {
                ledger::error(
                    "no running terminal with that id on the agent's worker machine; list the terminals to find its id and host_agent",
                )
            })
    }

    /// One capped wait on the worker that runs the terminal.
    async fn wait_on_worker_terminal(
        &self,
        wake: &AgentWake,
        detail: &TerminalWakeDetail,
        after_offset: Option<u64>,
    ) -> Step {
        let Some(host) = detail.host_agent.as_deref() else {
            return Step::Gone("the terminal is not on a worker machine".into());
        };
        let leased = self
            .owned
            .agent_store
            .get_agent(host)
            .ok()
            .is_some_and(|agent| agent.remote_execution().is_some());
        if !leased {
            return Step::Gone("the agent's worker machine lease ended".into());
        }
        let operation = RoomTerminalOperation::Wait {
            terminal_id: detail.terminal_id.clone(),
            watches: detail.until.clone(),
            after_offset,
            timeout_ms: MAX_WAIT.as_millis() as u64,
        };
        match self
            .route_room_terminal(
                &wake.room_id,
                TerminalActor::Agent(wake.agent_id.clone()),
                Some(host),
                false,
                operation,
            )
            .await
        {
            Ok(RoomTerminalResult::Waited { waited }) => Step::Waited(Box::new(waited)),
            Ok(_) => Step::Gone("the worker answered a terminal wait with something else".into()),
            Err(error) => Step::Unreachable(error.to_string()),
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
    pub(super) fn register_terminal_wake_driver(&mut self) {
        let driver = Arc::new(TerminalWakeDriver {
            state: self.clone(),
            workers: Arc::default(),
            terminals: self.owned.room_terminals.clone(),
            sink: self.wake_sink(KIND),
        });
        let events: Arc<dyn CompletionEvents> = driver.clone();
        self.owned
            .room_terminals
            .bind_events(Arc::downgrade(&events));
        let wake_driver: Arc<dyn WakeDriver> = driver.clone();
        self.owned
            .agent_wakes
            .drivers
            .register_weak(Arc::downgrade(&wake_driver));
        self.terminal_wake_driver = Some(driver);
    }

    /// Arms a terminal wake for the running turn of `task`.
    pub(super) async fn arm_terminal_wake(
        &self,
        task: &AgentTaskExecution,
        mut wake: AgentWake,
        mut detail: TerminalWakeDetail,
    ) -> Result<serde_json::Value, DaemonError> {
        self.ensure_room_agent(&task.room_id, &task.agent_id)?;
        if let Some(host) = &detail.host_agent {
            detail.armed_at_offset = Some(
                self.ensure_running_on_worker(task, host, &detail.terminal_id)
                    .await?,
            );
        }
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
