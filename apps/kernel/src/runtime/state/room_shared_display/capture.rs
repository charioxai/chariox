//! MP-08/MP-10/MP-11: peer waits do not own the home lease mutex. Queued
//! ciphertext retains its exact admission and audience until dispatch.
use super::*;

pub(crate) struct CapturedRoomDisplayFrame {
    pub(crate) frame: SealedRoomDisplayFrame,
    lease: Arc<Mutex<RoomDisplayLease>>,
    binding: RoomDisplayBinding,
    audience_epoch: u64,
    protection_revision: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn room_shared_display_dispatch_rechecks_binding_revision_audience_and_lease() {
        let (room, slice, _sink) = super::super::admission_tests::fixture().await;
        let runtime = &room.runtime;
        let attachment = crate::app::KernelSessionService::new(&mut *runtime.app.lock().await)
            .attach(crate::attachment::AttachRequest::new(
                &room.session_id,
                "capture-viewer",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap()
            .id()
            .to_owned();
        let config = runtime.owned.config_projection.snapshot();
        let binding: RoomDisplayBinding = serde_json::from_value(serde_json::json!({
            "generation":1,"subscription_id":"capture-lease","codec":"avc1.420033",
            "bitrate":8000000,"device_scale_factor":1,
            "source":{"kind":"desktop","surface_id":"surface","generation":"1","width":1280,"height":800}
        })).unwrap();
        let snapshot = RoomDisplayLease {
            caller: KernelCaller::default(),
            session_id: room.session_id.clone(),
            attachment_id: attachment,
            slice_id: slice,
            viewer_public_key: config.relay_public_key.clone(),
            binding: binding.clone(),
            worker_public_key: config.relay_public_key.clone(),
            used: Instant::now(),
        };
        let lease = Arc::new(Mutex::new(snapshot.clone()));
        runtime
            .owned
            .room_display_leases
            .home
            .lock()
            .unwrap()
            .insert(
                "capture-lease".into(),
                HomeLease {
                    session_id: room.session_id.clone(),
                    slice_id: snapshot.slice_id.clone(),
                    state: lease.clone(),
                },
            );
        let revision = runtime
            .owned
            .room_secret_observations
            .revision(&room.session_id)
            .unwrap();
        let mut captured = CapturedRoomDisplayFrame {
            frame: SealedRoomDisplayFrame {
                sequence: 1,
                encrypted_event: crate::transport::relay_crypto::encrypt_payload_for_peer(
                    &config.relay_private_key,
                    &config.relay_public_key,
                    b"supplementary-test",
                )
                .unwrap(),
            },
            lease: lease.clone(),
            binding,
            audience_epoch: runtime.room_display_audience_epoch(&room.session_id),
            protection_revision: revision,
        };
        let caller = KernelCaller::default();
        drop(
            runtime
                .validate_room_display_frame(&caller, "capture-lease", &captured)
                .await
                .unwrap(),
        );
        lease.lock().await.binding.generation += 1;
        assert!(runtime
            .validate_room_display_frame(&caller, "capture-lease", &captured)
            .await
            .is_err());
        lease.lock().await.binding = captured.binding.clone();
        captured.protection_revision = revision.wrapping_add(1);
        assert!(runtime
            .validate_room_display_frame(&caller, "capture-lease", &captured)
            .await
            .is_err());
        captured.protection_revision = revision;
        captured.audience_epoch = captured.audience_epoch.wrapping_add(1);
        assert!(runtime
            .validate_room_display_frame(&caller, "capture-lease", &captured)
            .await
            .is_err());
        captured.audience_epoch = runtime.room_display_audience_epoch(&room.session_id);
        runtime
            .owned
            .room_display_leases
            .home
            .lock()
            .unwrap()
            .get_mut("capture-lease")
            .unwrap()
            .state = Arc::new(Mutex::new(snapshot));
        assert!(runtime
            .validate_room_display_frame(&caller, "capture-lease", &captured)
            .await
            .is_err());
    }
}

fn stale() -> DaemonError {
    DaemonError::UserDomainRefused {
        reason: crate::error::UserDomainRefusalReason::StaleReference,
    }
}

impl KernelRuntimeState {
    pub(crate) async fn validate_room_display_frame(
        &self,
        caller: &KernelCaller,
        id: &str,
        captured: &CapturedRoomDisplayFrame,
    ) -> Result<tokio::sync::OwnedRwLockReadGuard<()>, DaemonError> {
        let current = self.home_display_lease(id)?;
        if !Arc::ptr_eq(&current, &captured.lease) {
            return Err(stale());
        }
        let lease = current.lock().await.clone();
        let _admission = self
            .admit_room_display(
                caller,
                &lease.session_id,
                &lease.attachment_id,
                &lease.viewer_public_key,
                &lease.slice_id,
                true,
                None,
            )
            .await?;
        // Do not wait for control operations while owning the observation
        // barrier. A busy lease retires this credit; recovery takes a key.
        let latest = self.home_display_lease(id)?;
        if !Arc::ptr_eq(&latest, &current) {
            return Err(stale());
        }
        let barrier = self
            .owned
            .room_secret_observations
            .barrier(&lease.session_id)?
            .read_owned()
            .await;
        let latest = latest.try_lock().map_err(|_| stale())?;
        if latest.binding != captured.binding
            || latest.used.elapsed() >= Duration::from_secs(60)
            || self.room_display_audience_epoch(&lease.session_id) != captured.audience_epoch
            || self
                .owned
                .room_secret_observations
                .revision(&lease.session_id)?
                != captured.protection_revision
        {
            return Err(stale());
        }
        Ok(barrier)
    }

    pub(crate) async fn room_display_frame(
        &self,
        caller: &KernelCaller,
        id: &str,
        generation: u64,
        after_sequence: u64,
        reset: bool,
        congested: bool,
    ) -> Result<Option<CapturedRoomDisplayFrame>, DaemonError> {
        let lease = self.home_display_lease(id)?;
        let snapshot = {
            let lease = lease.lock().await;
            if lease.binding.generation != generation
                || lease.used.elapsed() >= Duration::from_secs(60)
            {
                return Err(error());
            }
            lease.clone()
        };
        let (slice, _guard) = self
            .admit_room_display(
                caller,
                &snapshot.session_id,
                &snapshot.attachment_id,
                &snapshot.viewer_public_key,
                &snapshot.slice_id,
                true,
                None,
            )
            .await?;
        let audience_epoch = self.room_display_audience_epoch(&snapshot.session_id);
        let protection_revision = self
            .owned
            .room_secret_observations
            .revision(&snapshot.session_id)?;
        let capture_started = Instant::now();
        let result = self
            .room_display_peer(
                &slice,
                RelayPeerRequest::RoomSharedDisplay {
                    session_id: snapshot.session_id,
                    slice_id: snapshot.slice_id,
                    lease_id: id.into(),
                    command: RoomDisplayCommand::Capture {
                        after_sequence,
                        reset,
                        congested,
                    },
                },
            )
            .await?;
        crate::transport::kernel_browser_display::timing("room_capture_peer", capture_started);
        let RoomDisplayResult::Frame { frame } = result else {
            return Err(error());
        };
        let Some(frame) = frame else {
            return Ok(None);
        };
        let captured = CapturedRoomDisplayFrame {
            frame,
            lease: lease.clone(),
            binding: snapshot.binding,
            audience_epoch,
            protection_revision,
        };
        // A produced but discarded frame is a gap, requiring a new key.
        let _barrier = self
            .validate_room_display_frame(caller, id, &captured)
            .await?;
        lease.try_lock().map_err(|_| stale())?.used = Instant::now();
        Ok(Some(captured))
    }
}
