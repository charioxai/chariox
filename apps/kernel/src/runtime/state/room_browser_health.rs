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
                .room_browser_controller_health_probe(
                    &room,
                    snapshot.viewport,
                    snapshot.browser_bar_visible,
                )
                .await;
            (guard, result)
        });
        let Ok(Ok((_guard, result))) = tokio::time::timeout(Duration::from_secs(4), query).await
        else {
            return;
        };
        self.observe_room_browser_health_receipt(session_id, generation, result);
    }

    pub(crate) fn observe_room_browser_health_receipt(
        &self,
        session_id: &str,
        generation: u64,
        result: Result<Response, crate::error::DaemonError>,
    ) {
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
            Err(error)
                if error
                    .to_string()
                    .contains("browser controller is not leased by Room ") =>
            {
                Some("browser_controller_lease_lost")
            }
            Err(error)
                if super::room_browser_controller::is_room_slice_unreachable(&error)
                    || matches!(&error, crate::error::DaemonError::RelayTransport { code, .. }
                        if matches!(code.as_str(), "target_not_connected" | "target_disconnected" | "target_not_allowed"))
                    || error
                        .to_string()
                        .contains("browser_controller_scope_denied") =>
            {
                Some("browser_controller_unreachable")
            }
            // Reconcile shares a serial controller queue with foreground
            // commands and can wait on page dialogs. Busy, timeout and other
            // route errors are inconclusive; only positive browser/controller
            // loss changes health.
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
        // A verified periodic receipt also completes capture recovery, even
        // when component health was already Ready. Ignore stale generations
        // above before clearing the current runtime's recovery window.
        if diagnostic.is_none() {
            self.owned
                .room_environment_health_probes
                .startup_recovery
                .reset(session_id);
        }
        let state = if diagnostic.is_some() {
            EnvironmentComponentHealthState::Unavailable
        } else {
            EnvironmentComponentHealthState::Ready
        };
        let component = if diagnostic == Some("browser_controller_unreachable") {
            EnvironmentComponent::BrowserController
        } else {
            EnvironmentComponent::Browser
        };
        if diagnostic.is_some_and(super::app_view_runtime::browser_recovery_downtime) {
            self.app_control()
                .views()
                .suspend_for_cold_start(session_id);
        }
        let _ = sessions
            .update_room_environment_component_health(session_id, component, state, diagnostic);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::{DaemonApp, KernelSessionService},
        config::{CredentialVaultBackend, DaemonConfig},
        runtime::router::CommandRouter,
        session::{CanonicalViewport, CreateSessionRequest},
    };
    use std::time::Instant;
    use tokio::sync::Mutex as AsyncMutex;

    #[tokio::test]
    async fn periodic_healthy_receipt_rearms_capture_recovery_after_old_deadline() {
        let root = std::env::temp_dir().join(format!(
            "chariox-periodic-capture-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir(&root).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        let mut config = DaemonConfig::for_tests().with_session_history_root(root.join("history"));
        config.user_config.state.path = Some(root.join("state.db").display().to_string());
        config.user_config.credential_vault.backend = CredentialVaultBackend::ProcessMemory;
        let mut app = DaemonApp::bootstrap(config).unwrap();
        let (session, _) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "periodic-capture",
                "periodic-capture",
            ))
            .unwrap();
        let session = session.id().to_owned();
        let runtime = CommandRouter::with_interactive_capacity(Arc::new(AsyncMutex::new(app)), 1)
            .runtime_state();
        let viewport = CanonicalViewport::new(1280, 800, 1, 1280, 800).unwrap();
        let before = runtime.start_room_environment(&session, viewport).unwrap();
        for component in [
            EnvironmentComponent::BrowserController,
            EnvironmentComponent::Browser,
            EnvironmentComponent::Desktop,
            EnvironmentComponent::Streamer,
        ] {
            runtime
                .owned
                .session_store
                .update_room_environment_component_health(
                    &session,
                    component,
                    EnvironmentComponentHealthState::Ready,
                    None,
                )
                .unwrap();
        }
        runtime
            .transition_room_environment(&session, EnvironmentLifecycle::Ready)
            .unwrap();
        let recovery = &runtime
            .owned
            .room_environment_health_probes
            .startup_recovery;
        let error = crate::error::DaemonError::RelayTransport {
            operation: "read relay peer response",
            code: "transport_error".into(),
            message: "local transport `browser_controller.route` failed: browser controller `browser.reconcile` failed with viewport_apply_failed: capture unavailable".into(),
            retryable: true,
        };
        let now = Instant::now();
        assert_eq!(
            recovery.after_start_error(&session, &error, now),
            EnvironmentLifecycle::Degraded
        );
        runtime.observe_room_browser_health(
            &session,
            before.runtime_generation,
            Some("browser_cdp_disconnected"),
        );
        assert_eq!(
            runtime
                .room_environment_snapshot(&session)
                .unwrap()
                .lifecycle,
            EnvironmentLifecycle::Degraded
        );
        // A late healthy receipt cannot reset another generation's deadline.
        runtime.observe_room_browser_health(&session, before.runtime_generation + 1, None);
        let later = now + Duration::from_secs(121);
        assert_eq!(
            recovery.after_start_error(&session, &error, later),
            EnvironmentLifecycle::Failed
        );
        runtime.observe_room_browser_health(&session, before.runtime_generation, None);
        assert_eq!(
            runtime
                .room_environment_snapshot(&session)
                .unwrap()
                .lifecycle,
            EnvironmentLifecycle::Ready
        );
        assert_eq!(
            recovery.after_start_error(&session, &error, later),
            EnvironmentLifecycle::Degraded
        );
        // An unchanged Ready health receipt must clear recovery as well.
        runtime.observe_room_browser_health(&session, before.runtime_generation, None);
        assert_eq!(
            recovery.after_start_error(&session, &error, later + Duration::from_secs(121)),
            EnvironmentLifecycle::Degraded
        );
    }
}
