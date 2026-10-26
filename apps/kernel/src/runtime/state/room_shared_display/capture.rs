//! MP-08/MP-10/MP-11: peer waits do not own the home lease mutex. Queued
//! ciphertext retains its exact admission and audience until dispatch.
use super::*;

pub(crate) struct CapturedRoomDisplayFrame {
    pub(crate) frame: SealedRoomDisplayFrame,
    lease: Arc<Mutex<RoomDisplayLease>>,
    binding: RoomDisplayBinding,
    audience_epoch: u64,
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
    ) -> Result<(), DaemonError> {
        let current = self.home_display_lease(id)?;
        if !Arc::ptr_eq(&current, &captured.lease) {
            return Err(stale());
        }
        let lease = current.lock().await;
        if lease.binding != captured.binding
            || lease.used.elapsed() >= Duration::from_secs(60)
            || self.room_display_audience_epoch(&lease.session_id) != captured.audience_epoch
        {
            return Err(stale());
        }
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
        Ok(())
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
        };
        // A produced but discarded frame is a gap, requiring a new key.
        self.validate_room_display_frame(caller, id, &captured)
            .await?;
        lease.lock().await.used = Instant::now();
        Ok(Some(captured))
    }
}
