//! Browser health belongs to the kernel, including Rooms with no attached UI.
use super::KernelRuntimeState;
use crate::session::{EnvironmentComponent, EnvironmentComponentHealthState, EnvironmentLifecycle};
use crate::transport::room_browser_controller::RoomBrowserControllerResult as Response;
use std::{
    collections::BTreeSet,
    sync::{atomic::Ordering, Arc, Mutex},
    time::Duration,
};

struct InflightProbe {
    rooms: Arc<Mutex<BTreeSet<String>>>,
    session_id: String,
}

impl Drop for InflightProbe {
    fn drop(&mut self) {
        self.rooms
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.session_id);
    }
}

impl KernelRuntimeState {
    pub(super) fn schedule_room_browser_health(&self) {
        let now = crate::session::unix_epoch_ms();
        let next = self
            .owned
            .next_room_browser_health_at_ms
            .load(Ordering::Relaxed);
        if now < next
            || self
                .owned
                .next_room_browser_health_at_ms
                .compare_exchange(next, now + 5_000, Ordering::Relaxed, Ordering::Relaxed)
                .is_err()
        {
            return;
        }
        let rooms: Vec<_> = self
            .owned
            .session_store
            .list_sessions()
            .into_iter()
            .filter_map(|session| {
                let snapshot = self.room_environment_snapshot(session.id()).ok()?;
                (matches!(
                    snapshot.lifecycle,
                    EnvironmentLifecycle::Ready | EnvironmentLifecycle::Degraded
                ) && self.browser_controller_enabled_for_room(session.id()))
                .then_some((session.id().to_owned(), snapshot.runtime_generation))
            })
            .collect();
        let runtime = self.clone();
        tokio::spawn(async move {
            let mut checks = tokio::task::JoinSet::new();
            for (session_id, generation) in rooms {
                let runtime = runtime.clone();
                checks.spawn(async move {
                    runtime
                        .refresh_room_browser_health(&session_id, generation)
                        .await;
                });
            }
            while checks.join_next().await.is_some() {}
        });
    }

    pub(crate) async fn refresh_room_browser_health(&self, session_id: &str, generation: u64) {
        let rooms = self.owned.room_browser_health_inflight.clone();
        if !rooms
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(session_id.to_owned())
        {
            return;
        }
        let guard = InflightProbe {
            rooms,
            session_id: session_id.to_owned(),
        };
        let Ok(snapshot) = self.room_environment_snapshot(session_id) else {
            return;
        };
        if snapshot.runtime_generation != generation
            || !matches!(
                snapshot.lifecycle,
                EnvironmentLifecycle::Ready | EnvironmentLifecycle::Degraded
            )
        {
            return;
        }
        // Reconcile proves debugger and page availability, but a health probe
        // does not project tab identities or change the viewport owner.
        // A timed-out caller cannot cancel a sent relay request or spawn_blocking
        // work. Keep ownership in that task until the underlying request ends;
        // later passes skip this Room while other Rooms still get their probes.
        let runtime = self.clone();
        let room = session_id.to_owned();
        let query = tokio::spawn(async move {
            let result = runtime
                .room_browser_controller_health_probe(&room, snapshot.viewport, snapshot.browser_bar_visible)
                .await;
            (guard, result)
        });
        let Ok(Ok((_guard, result))) = tokio::time::timeout(Duration::from_secs(4), query).await
        else {
            return;
        };
        let diagnostic = match result {
            Ok(Response::Reconciled {
                reconciliation: Some(_),
            }) => None,
            Err(error) if error.to_string().contains("browser_debugger_unavailable") => {
                Some("browser_debugger_unavailable")
            }
            Err(error) if error.to_string().contains("browser_cdp_disconnected") => {
                Some("browser_cdp_disconnected")
            }
            // Reconcile shares a serial controller queue with foreground
            // commands and can wait on page dialogs. A timeout or route error
            // is inconclusive; only positive debugger loss changes health.
            Ok(_) | Err(_) => return,
        };
        self.observe_room_browser_health(session_id, generation, diagnostic);
    }

    pub(crate) fn observe_room_browser_health(
        &self,
        session_id: &str,
        generation: u64,
        diagnostic: Option<&str>,
    ) {
        // Commit under the session owner: late probes must not resurrect a
        // stopped Room or overwrite a new Start/Retry generation.
        let mut sessions = self.owned.session_store.write();
        let Ok(snapshot) = sessions.room_environment_snapshot(session_id) else {
            return;
        };
        if snapshot.runtime_generation != generation
            || !matches!(
                snapshot.lifecycle,
                EnvironmentLifecycle::Ready | EnvironmentLifecycle::Degraded
            )
        {
            return;
        }
        let state = if diagnostic.is_some() {
            EnvironmentComponentHealthState::Unavailable
        } else {
            EnvironmentComponentHealthState::Ready
        };
        let Some(browser) = snapshot
            .health
            .iter()
            .find(|h| h.component == EnvironmentComponent::Browser)
        else {
            return;
        };
        if browser.state == state && browser.diagnostic_code.as_deref() == diagnostic {
            return;
        }
        let Ok(updated) = sessions.update_room_environment_component_health(
            session_id,
            EnvironmentComponent::Browser,
            state,
            diagnostic,
        ) else {
            return;
        };
        if diagnostic.is_some() && snapshot.lifecycle == EnvironmentLifecycle::Ready {
            let _ =
                sessions.transition_room_environment(session_id, EnvironmentLifecycle::Degraded);
        } else if diagnostic.is_none()
            && snapshot.lifecycle == EnvironmentLifecycle::Degraded
            && updated
                .health
                .iter()
                .all(|h| h.state == EnvironmentComponentHealthState::Ready)
        {
            let _ = sessions.transition_room_environment(session_id, EnvironmentLifecycle::Ready);
        }
    }
}
