//! MP-08/MP-10/MP-11: home attachments own the shared display audience.
//! A short renewable control grant fails closed when the home link disappears.
use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[derive(Default)]
pub(super) struct Audience {
    gate: Arc<Mutex<()>>,
    worker_gate: Mutex<()>,
    revision: AtomicU64,
    refreshing: AtomicBool,
    sent: std::sync::Mutex<BTreeMap<(String, String), (bool, Instant)>>,
    received: std::sync::Mutex<BTreeMap<String, u64>>,
    epochs: std::sync::Mutex<BTreeMap<String, u64>>,
}

impl KernelRuntimeState {
    pub(crate) async fn room_display_audience_guard(&self) -> tokio::sync::OwnedMutexGuard<()> {
        self.owned
            .room_display_leases
            .audience
            .gate
            .clone()
            .lock_owned()
            .await
    }

    pub(crate) fn room_display_owner(&self, user: &str) -> bool {
        crate::account_profile::provider_account_authority_owner_user_id(
            &self.owned.config_projection.snapshot(),
            user,
        ) == crate::session::DEFAULT_LOCAL_USER_ID
    }

    pub(crate) fn room_display_requires_mask(&self) -> bool {
        // Snapshot all attachments, including other sessions on this kernel.
        // Local and Cloud tabs of the configured owner identify the same person.
        let config = self.owned.config_projection.snapshot();
        let attachments = self.owned.attachment_store.read();
        attachments
            .list_attached_session_ids()
            .iter()
            .any(|session| {
                attachments
                    .list_session_attachment_ids(session)
                    .iter()
                    .any(|id| {
                        attachments.get_attachment(id).map_or(true, |attachment| {
                            crate::account_profile::provider_account_authority_owner_user_id(
                                &config,
                                attachment.owner_user_id(),
                            ) != crate::session::DEFAULT_LOCAL_USER_ID
                        })
                    })
            })
    }

    pub(crate) async fn send_room_display_audience(
        &self,
        slice: &crate::slice::SliceRecord,
        room: &str,
        mask: bool,
        force: bool,
    ) -> Result<(), DaemonError> {
        let audience = &self.owned.room_display_leases.audience;
        let key = (room.to_owned(), slice.id.clone());
        if !force
            && audience
                .sent
                .lock()
                .map_err(|_| error())?
                .get(&key)
                .is_some_and(|(previous, at)| {
                    *previous == mask && at.elapsed() < Duration::from_millis(500)
                })
        {
            return Ok(());
        }
        if audience
            .sent
            .lock()
            .map_err(|_| error())?
            .get(&key)
            .is_none_or(|(previous, _)| *previous != mask)
        {
            let mut epochs = audience.epochs.lock().map_err(|_| error())?;
            *epochs.entry(room.into()).or_default() += 1;
        }
        // The timestamp floor also orders controls across home kernel restarts.
        // A late timed-out unmask can never overwrite a newer mask.
        audience.revision.fetch_max(
            crate::session::unix_epoch_ms().saturating_mul(1024),
            Ordering::SeqCst,
        );
        let revision = audience.revision.fetch_add(1, Ordering::SeqCst) + 1;
        let result = self
            .room_display_peer(
                slice,
                RelayPeerRequest::SetRoomSharedDisplayMask {
                    session_id: room.into(),
                    slice_id: slice.id.clone(),
                    mask,
                    revision,
                },
            )
            .await?;
        if !matches!(result, RoomDisplayResult::Settled {}) {
            return Err(error());
        }
        audience
            .sent
            .lock()
            .map_err(|_| error())?
            .insert(key, (mask, Instant::now()));
        Ok(())
    }

    // Call with the audience gate held. Joins mask before becoming visible;
    // departures may reveal only after the last guest attachment is gone.
    pub(crate) async fn sync_room_display_audience(
        &self,
        mask: bool,
        force: bool,
    ) -> Result<(), DaemonError> {
        let rooms = self.room_display_scopes();
        for (room, slice_id) in rooms {
            let slice = self.resolve_slice(&slice_id)?;
            self.send_room_display_audience(&slice, &room, mask, force)
                .await?;
        }
        Ok(())
    }

    pub(super) fn room_display_scopes(&self) -> BTreeMap<String, String> {
        self.owned
            .room_display_leases
            .home
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .filter(|lease| {
                lease
                    .state
                    .try_lock()
                    .map_or(true, |state| state.used.elapsed() < Duration::from_secs(60))
            })
            .map(|lease| (lease.session_id.clone(), lease.slice_id.clone()))
            .collect()
    }

    pub(crate) fn room_display_audience_due_at_ms(&self, now_ms: u64) -> Option<u64> {
        // Owner grants last three seconds. Live display leases keep the pump
        // renewing at its active cadence even while every provider is idle.
        (!self.room_display_scopes().is_empty()).then(|| now_ms.saturating_add(500))
    }

    pub(crate) fn room_display_audience_epoch(&self, room: &str) -> u64 {
        *self
            .owned
            .room_display_leases
            .audience
            .epochs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(room)
            .unwrap_or(&0)
    }

    pub(crate) fn schedule_room_display_audience(&self) {
        let leases = &self.owned.room_display_leases;
        if leases
            .home
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
            || leases.audience.refreshing.swap(true, Ordering::AcqRel)
        {
            return;
        }
        let runtime = self.clone();
        tokio::spawn(async move {
            let _gate = runtime.room_display_audience_guard().await;
            if let Err(error) = runtime
                .sync_room_display_audience(runtime.room_display_requires_mask(), false)
                .await
            {
                tracing::warn!(%error, "MP-08/MP-10/MP-11: Room display audience renewal failed; worker expires masked");
            }
            runtime
                .owned
                .room_display_leases
                .audience
                .refreshing
                .store(false, Ordering::Release);
        });
    }

    pub(crate) async fn execute_bound_room_shared_display_mask(
        &self,
        home: &str,
        key: &str,
        room: &str,
        slice: &str,
        mask: bool,
        revision: u64,
    ) -> Result<RoomDisplayResult, DaemonError> {
        self.authorize_bound_room_computer_read(home, key, room, slice)?;
        let audience = &self.owned.room_display_leases.audience;
        let _gate = audience.worker_gate.lock().await;
        if revision == 0 || !mask && !self.owned.relay_state.read().await.connected() {
            return Err(error());
        }
        {
            let mut received = audience.received.lock().map_err(|_| error())?;
            if received
                .get(room)
                .is_some_and(|previous| revision <= *previous)
            {
                return Err(error());
            }
            received.insert(room.into(), revision);
        }
        self.set_worker_display_mask(room, mask).await?;
        Ok(RoomDisplayResult::Settled {})
    }

    async fn set_worker_display_mask(&self, room: &str, mask: bool) -> Result<(), DaemonError> {
        let policy = match self.owned.room_secret_observations.capture_policy(room) {
            Ok(policy) => serde_json::from_str(&policy).map_err(|_| error())?,
            Err(_) if mask => serde_json::json!({"unknown":true,"values":[],"targets":[]}),
            Err(error) => return Err(error),
        };
        let processes = self.owned.browser_controller_processes.clone();
        let room = room.to_owned();
        tokio::task::spawn_blocking(move || {
            processes.room_computer_request(
                &room,
                "room.display",
                serde_json::json!({"op":"display_mask","mask":mask,"policy":policy}),
            )
        })
        .await
        .map_err(|_| error())?
        .map_err(|message| DaemonError::LocalTransport {
            operation: "Room display audience",
            message,
        })?;
        Ok(())
    }

    pub(crate) async fn fail_closed_room_display_audience(&self) {
        let audience = &self.owned.room_display_leases.audience;
        let _gate = audience.worker_gate.lock().await;
        let rooms = audience
            .received
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for room in rooms {
            if let Err(error) = self.set_worker_display_mask(&room, true).await {
                tracing::warn!(%error, "MP-08/MP-10/MP-11: disconnected Room display expires masked");
            }
        }
        audience
            .sent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::state::browser_controller_action_execution_runtime_state::computer_input_reconcile_test_support::TestRoom;
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};

    #[tokio::test]
    async fn mp11_room_audience_counts_people_across_sessions_and_owner_tabs() {
        let room = TestRoom::new("room-audience-people");
        let state = &room.runtime;
        let mut config = state.owned.config_projection.snapshot();
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            user_id: "owner-cloud".into(),
            ..Default::default()
        });
        state.owned.config_projection.update(config);
        assert!(!state.room_display_requires_mask());
        for (client, user) in [
            ("local-tab", "local"),
            ("web-tab", "owner-cloud"),
            ("other-owner-tab", "owner-cloud"),
        ] {
            state
                .owned
                .attach(AttachRequest::for_user(
                    &room.session_id,
                    client,
                    ClientCapabilityLevel::FullTerminal,
                    user,
                ))
                .unwrap();
            assert!(
                !state.room_display_requires_mask(),
                "owner's own tabs stay unmasked"
            );
        }
        let other_root = crate::test_support::TestWorktree::new("room-audience-other");
        let other_path = other_root.path().to_str().unwrap();
        let other = crate::app::KernelSessionService::new(&mut *state.app.lock().await)
            .create_session(crate::session::CreateSessionRequest::new(
                other_path, other_path,
            ))
            .unwrap()
            .0;
        let guest = state
            .owned
            .attach(AttachRequest::for_user(
                other.id(),
                "guest-other-room",
                ClientCapabilityLevel::FullTerminal,
                "other-person",
            ))
            .unwrap();
        assert!(
            state.room_display_requires_mask(),
            "another person's other session counts"
        );
        let same_room = state
            .owned
            .attach(AttachRequest::for_user(
                &room.session_id,
                "guest-shared-room",
                ClientCapabilityLevel::FullTerminal,
                "guest",
            ))
            .unwrap();
        state.owned.detach(guest.id()).unwrap();
        assert!(
            state.room_display_requires_mask(),
            "the last guest must leave before reveal"
        );
        state.owned.detach(same_room.id()).unwrap();
        assert!(!state.room_display_requires_mask());
    }

    #[tokio::test]
    async fn mp11_room_audience_worker_rejects_foreign_replayed_and_disconnected_unmask() {
        let room = TestRoom::new("room-audience-admission");
        let state = &room.runtime;
        let mut config = state.owned.config_projection.snapshot();
        config.room_environment_worker_binding =
            Some(crate::config::RoomEnvironmentWorkerBinding {
                home_kernel_id: "home".into(),
                home_public_key: config.relay_public_key.clone(),
                session_id: room.session_id.clone(),
                slice_id: "slice".into(),
                provisioned_slice_id: None,
            });
        state.owned.config_projection.update(config.clone());
        for (home, key, session, slice) in [
            (
                "foreign",
                config.relay_public_key.as_str(),
                room.session_id.as_str(),
                "slice",
            ),
            ("home", "foreign-key", room.session_id.as_str(), "slice"),
            (
                "home",
                config.relay_public_key.as_str(),
                "foreign-room",
                "slice",
            ),
            (
                "home",
                config.relay_public_key.as_str(),
                room.session_id.as_str(),
                "foreign-slice",
            ),
        ] {
            assert!(state
                .execute_bound_room_shared_display_mask(home, key, session, slice, false, 99)
                .await
                .is_err());
        }
        assert!(state
            .owned
            .room_display_leases
            .audience
            .received
            .lock()
            .unwrap()
            .is_empty());
        assert!(
            state
                .execute_bound_room_shared_display_mask(
                    "home",
                    &config.relay_public_key,
                    &room.session_id,
                    "slice",
                    false,
                    99
                )
                .await
                .is_err(),
            "a disconnected worker never reveals"
        );
        assert!(state
            .owned
            .room_display_leases
            .audience
            .received
            .lock()
            .unwrap()
            .is_empty());
        state
            .owned
            .room_display_leases
            .audience
            .received
            .lock()
            .unwrap()
            .insert(room.session_id.clone(), 100);
        for revision in [0, 99, 100] {
            assert!(state
                .execute_bound_room_shared_display_mask(
                    "home",
                    &config.relay_public_key,
                    &room.session_id,
                    "slice",
                    true,
                    revision
                )
                .await
                .is_err());
        }
        assert_eq!(
            state
                .owned
                .room_display_leases
                .audience
                .received
                .lock()
                .unwrap()[&room.session_id],
            100
        );
    }
}
