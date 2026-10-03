//! One bounded controller observation per Room, shared by every client.
use super::KernelRuntimeState;
use crate::session::{EnvironmentComponent, EnvironmentComponentHealthState, EnvironmentLifecycle};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Default)]
pub(super) struct RoomEnvironmentHealthProbes {
    rooms: Mutex<BTreeMap<String, Arc<tokio::sync::Mutex<Option<Instant>>>>>,
}

impl KernelRuntimeState {
    pub(crate) fn schedule_room_environment_health_refresh(&self, session_id: &str) {
        let Ok(environment) = self.room_environment_snapshot(session_id) else {
            return;
        };
        if !self.browser_controller_enabled_for_room(session_id)
            || matches!(
                environment.lifecycle,
                EnvironmentLifecycle::Stopped
                    | EnvironmentLifecycle::Stopping
                    | EnvironmentLifecycle::Failed
            )
        {
            return;
        }
        let probe = self
            .owned
            .room_environment_health_probes
            .rooms
            .lock()
            .expect("Room health probe lock poisoned")
            .entry(session_id.into())
            .or_default()
            .clone();
        let Ok(mut lease) = probe.try_lock_owned() else {
            return;
        };
        if lease.is_some_and(|last| last.elapsed() < Duration::from_secs(1)) {
            return;
        }
        let state = self.clone();
        let room = session_id.to_string();
        tokio::spawn(async move {
            if state
                .ensure_browser_controller_process_started(&room)
                .await
                .is_ok()
                && state
                    .reconcile_browser_controller_environment(&room)
                    .await
                    .is_ok()
            {
                // Starting and Degraded include restored Rooms. A fresh controller/worker
                // observation uses the same normal start completion path as initial launch.
                if let Ok(environment) = state.room_environment_snapshot(&room) {
                    if matches!(
                        environment.lifecycle,
                        EnvironmentLifecycle::Starting | EnvironmentLifecycle::Degraded
                    ) {
                        let _ = state
                            .finish_room_environment_controller_start(&room, "environment.health")
                            .await;
                    }
                }
            } else if state
                .room_environment_snapshot(&room)
                .is_ok_and(|environment| {
                    matches!(
                        environment.lifecycle,
                        EnvironmentLifecycle::Starting
                            | EnvironmentLifecycle::Ready
                            | EnvironmentLifecycle::Degraded
                    )
                })
            {
                let _ = state.update_room_environment_component_health(
                    &room,
                    EnvironmentComponent::BrowserController,
                    EnvironmentComponentHealthState::Unavailable,
                    Some("controller_probe_unavailable"),
                );
                let _ = state.transition_room_environment(&room, EnvironmentLifecycle::Degraded);
            }
            *lease = Some(Instant::now());
        });
    }
}
