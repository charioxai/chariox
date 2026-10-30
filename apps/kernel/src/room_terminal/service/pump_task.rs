//! Independent parse/render scheduling; the service only looks up owners.

use super::*;

impl RoomTerminalService {
    pub(super) fn start_pump(&self, owner: &Arc<TerminalOwner>) {
        let runtime = self
            .inner
            .pump_runtime
            .get_or_init(tokio::runtime::Handle::current);
        let service = Arc::downgrade(&self.inner);
        let owner = Arc::downgrade(owner);
        runtime.spawn(async move {
            let mut last_full = Instant::now();
            loop {
                let (Some(inner), Some(owner)) = (service.upgrade(), owner.upgrade()) else {
                    break;
                };
                let full = last_full.elapsed() >= PUMP_INTERVAL;
                if full {
                    last_full = Instant::now();
                }
                let pump_owner = owner.clone();
                // Parsing and scrollback rendering can use a whole CPU turn.
                // They must not occupy a Tokio transport/timer worker.
                let interval = tokio::task::spawn_blocking(move || {
                    RoomTerminalService { inner }.pump_terminal(&pump_owner, full)
                })
                .await
                .expect("terminal pump panicked");
                let Some(interval) = interval else { break };
                tokio::select! {
                    () = tokio::time::sleep(interval) => {}
                    () = owner.pump_wake.notified() => {}
                }
            }
        });
    }

    pub(super) fn pump_soon(&self, owner: &TerminalOwner) {
        owner.lock().active_until = Instant::now() + ACTIVE_PUMP_WINDOW;
        owner.pump_wake.notify_one();
    }

    #[cfg(test)]
    pub(in crate::room_terminal) fn pump_once(&self) {
        for owner in self.owners() {
            self.pump_terminal(&owner, true);
        }
    }

    pub(super) fn pump_terminal(&self, owner: &TerminalOwner, full: bool) -> Option<Duration> {
        let mut notifications = Vec::new();
        let mut wakes = Vec::new();
        let output_changed;
        let interval;
        {
            let mut state = owner.lock();
            if state.removed || state.shutdown {
                return None;
            }
            if state.handed_over {
                return Some(PUMP_INTERVAL);
            }
            let TerminalState {
                pty,
                terminal,
                waiters,
                active_until,
                ..
            } = &mut *state;
            let now = Instant::now();
            let pumped = if full {
                terminal.pump(pty, now)
            } else {
                terminal.pump_output(pty, now)
            };
            output_changed = pumped.output_changed;
            if !pumped.fired.is_empty() {
                let screen_text = terminal.screen.visible_text();
                for (owner, kind) in pumped.fired {
                    let fired = event(terminal, owner.clone(), kind, screen_text.clone());
                    match waiters.remove(&owner.wake_id) {
                        Some(waiter) => wakes.push((waiter, fired)),
                        None => notifications.push(fired),
                    }
                }
            }
            self.publish_outside_jobs(terminal);
            self.persist_if_due(terminal, now);
            if output_changed {
                *active_until = Instant::now() + ACTIVE_PUMP_WINDOW;
            }
            interval = if Instant::now() < *active_until {
                ACTIVE_PUMP_INTERVAL
            } else {
                PUMP_INTERVAL
            };
        }
        if full {
            self.report_work();
        }
        if output_changed {
            owner.output_changed.notify_waiters();
        }
        for (waiter, fired) in wakes {
            let _ = waiter.send(fired);
        }
        for fired in notifications {
            self.inner.events.deliver(fired);
        }
        Some(interval)
    }

    /// Stop under this terminal's lock. A pump that already cloned its handle
    /// sees `removed` before it can parse or queue a save after deletion.
    pub(super) fn stop_owner(
        &self,
        owner: &TerminalOwner,
        removed: bool,
    ) -> Result<(), DaemonError> {
        let mut result = Ok(());
        let mut state = owner.lock();
        state.removed |= removed;
        let TerminalState {
            pty,
            terminal,
            waiters,
            ..
        } = &mut *state;
        if let Some(run_id) = &terminal.run_id {
            if let Some(sweep) = &terminal.session_sweep {
                sweep.stop();
            }
            if let Err(error) = pty.remove_process(run_id) {
                crate::logging::error_with_fields(
                    "room_terminal.close",
                    "terminal process could not be removed",
                    serde_json::json!({"terminal_id": terminal.record.terminal_id, "error": error.to_string()}),
                );
                result = Err(error);
            }
            terminal.end(RoomTerminalOutcome::ProgramExited {
                exit_code: None,
                signal: None,
            });
        }
        if removed {
            waiters.clear();
        }
        drop(state);
        owner.pump_wake.notify_one();
        owner.output_changed.notify_waiters();
        result
    }

    /// Keep ended records readable, but stop every per-terminal pump before
    /// saving the final screen.
    pub fn shutdown(&self) {
        let owners = {
            let _creation = self
                .inner
                .lifecycle
                .lock()
                .expect("terminal lifecycle poisoned");
            let mut state = self.lock();
            state.shutdown = true;
            state.terminals.values().cloned().collect::<Vec<_>>()
        };
        for owner in owners {
            self.pump_terminal(&owner, true);
            let pending = {
                let mut state = owner.lock();
                if state.removed {
                    continue;
                }
                state.shutdown = true;
                let TerminalState {
                    pty,
                    terminal,
                    waiters,
                    ..
                } = &mut *state;
                if let Some(run_id) = &terminal.run_id {
                    if let Some(sweep) = &terminal.session_sweep {
                        sweep.stop();
                    }
                    if let Err(error) = pty.remove_process(run_id) {
                        crate::logging::error_with_fields(
                            "room_terminal.shutdown",
                            "terminal process could not be removed",
                            serde_json::json!({"terminal_id": terminal.record.terminal_id, "error": error.to_string()}),
                        );
                    }
                    terminal.end(RoomTerminalOutcome::KernelRestarted);
                }
                waiters.clear();
                terminal.unsaved_since = None;
                // Enqueue under the owner's lock so close cannot enqueue a
                // deletion before this final save and then be resurrected.
                self.inner.store.enqueue_save(&terminal.persisted_record())
            };
            owner.pump_wake.notify_one();
            owner.output_changed.notify_waiters();
            if let Err(error) = pending.and_then(|pending| pending.wait()) {
                crate::logging::error_with_fields(
                    "room_terminal.persist",
                    "terminal scrollback could not be saved",
                    serde_json::json!({"error": error.to_string()}),
                );
            }
        }
        self.report_work();
    }
    fn persist_if_due(&self, terminal: &mut Terminal, now: Instant) {
        if let Some(result) = terminal
            .pending_save
            .as_ref()
            .and_then(|pending| pending.try_result())
        {
            terminal.pending_save = None;
            if let Err(error) = result {
                terminal.unsaved_since.get_or_insert(now);
                crate::logging::error_with_fields(
                    "room_terminal.persist",
                    "terminal scrollback save failed; it will be retried",
                    serde_json::json!({"error": error.to_string()}),
                );
            }
        }
        if terminal.pending_save.is_some() {
            return;
        }
        let due = terminal.unsaved_since.is_some_and(|since| {
            !terminal.is_running() || now.duration_since(since) >= PERSIST_INTERVAL
        });
        if due {
            match self.inner.store.enqueue_save(&terminal.persisted_record()) {
                Ok(pending) => {
                    terminal.pending_save = Some(pending);
                    terminal.unsaved_since = None;
                }
                Err(error) => crate::logging::error_with_fields(
                    "room_terminal.persist",
                    "terminal scrollback save could not be queued; it will be retried",
                    serde_json::json!({"error": error.to_string()}),
                ),
            }
        }
    }
}
