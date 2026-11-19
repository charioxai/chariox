//! AO-IN-02: workers that finish together reach their parent in one wake. A `child_finished`
//! that leaves a sibling still working waits for the others for a moment, so the events that
//! arrive meanwhile ride in the same batch. The window never outlasts `SIBLING_WINDOW`, however
//! long a sibling keeps working.
use super::*;
use crate::durable_state::agent_lifecycle::{ExecutionState, InboxEvent};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SIBLING_WINDOW: Duration = Duration::from_millis(2500);

/// When each parent's window opened, by room and agent.
#[derive(Clone, Default)]
pub(super) struct SiblingWindows(Arc<Mutex<HashMap<(String, String), Instant>>>);

/// What a delivery does with the lead event it found.
pub(super) enum Window {
    /// Deliver now.
    Closed,
    /// Deliver after this long; `first` is set for the delivery that opened the window and
    /// must schedule the one that ends it.
    Open { left: Duration, first: bool },
}

impl SiblingWindows {
    fn windows(&self) -> std::sync::MutexGuard<'_, HashMap<(String, String), Instant>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn check(&self, room: &str, agent: &str, siblings_working: bool) -> Window {
        let key = (room.to_owned(), agent.to_owned());
        let mut windows = self.windows();
        if !siblings_working {
            windows.remove(&key);
            return Window::Closed;
        }
        let first = !windows.contains_key(&key);
        let opened = *windows.entry(key.clone()).or_insert_with(Instant::now);
        match SIBLING_WINDOW.checked_sub(opened.elapsed()) {
            Some(left) if !left.is_zero() => Window::Open { left, first },
            _ => {
                windows.remove(&key);
                Window::Closed
            }
        }
    }
}

impl KernelRuntimeState {
    /// Whether delivering `lead` to `agent` waits for the siblings of the child it reports.
    pub(super) fn sibling_window(&self, room: &str, agent: &str, lead: &InboxEvent) -> Window {
        let working = lead.kind == "child_finished"
            && lead.attempted_at_ms.is_none()
            && self.sub_agents_of(agent).iter().any(|child| {
                self.owned
                    .durable_state_store
                    .agent_tasks(Some(room), Some(child.id()))
                    .ok()
                    .is_some_and(|tasks| {
                        tasks
                            .last()
                            .is_some_and(|task| task.state == ExecutionState::Working)
                    })
            });
        self.sibling_windows.check(room, agent, working)
    }

    /// Delivers to `agent` again once its window ends.
    pub(super) fn deliver_when_window_ends(&self, room: &str, agent: &str, left: Duration) {
        let state = self.clone();
        let (room, agent) = (room.to_owned(), agent.to_owned());
        tokio::spawn(async move {
            tokio::time::sleep(left).await;
            if let Err(error) = Box::pin(state.deliver_agent_inbox(&room, &agent)).await {
                tracing::warn!(%error, "AO-IN-02: delivery after the sibling window retained for the periodic sweep");
            }
        });
    }
}
