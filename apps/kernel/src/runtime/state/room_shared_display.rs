//! MP-08/MP-10/MP-11: home owns leases; worker owns the exact Room producer.
#[cfg(test)]
mod admission_tests;
mod audience;
mod capture;
pub(super) mod timing;
mod worker;
use super::*;
use crate::runtime::command::KernelCaller;
use crate::slice::{
    RoomDisplayBinding, RoomDisplayOffer, SliceDisplayEndpoint, SliceDisplayEndpointAccess,
    SliceDisplayEndpointKind,
};
use crate::transport::room_shared_display::{
    RoomDisplayCommand, RoomDisplayResult, RoomDisplayTarget, SealedRoomDisplayFrame,
};
pub(crate) use capture::CapturedRoomDisplayFrame;
#[derive(Clone, Default)]
pub(super) struct RoomDisplayLeases {
    audience: Arc<audience::Audience>,
    home: Arc<std::sync::Mutex<BTreeMap<String, HomeLease>>>,
    worker: Arc<std::sync::Mutex<BTreeMap<String, Arc<Mutex<WorkerLease>>>>>,
}
#[derive(Clone)]
struct HomeLease {
    session_id: String,
    slice_id: String,
    state: Arc<Mutex<RoomDisplayLease>>,
}
impl HomeLease {
    fn new(lease: RoomDisplayLease) -> Self {
        Self {
            session_id: lease.session_id.clone(),
            slice_id: lease.slice_id.clone(),
            state: Arc::new(Mutex::new(lease)),
        }
    }
}
#[derive(Clone)]
struct RoomDisplayLease {
    caller: KernelCaller,
    session_id: String,
    attachment_id: String,
    slice_id: String,
    viewer_public_key: String,
    binding: RoomDisplayBinding,
    worker_public_key: String,
    used: Instant,
}
struct WorkerLease {
    home: String,
    home_key: String,
    session_id: String,
    slice_id: String,
    viewer_public_key: String,
    binding: RoomDisplayBinding,
    viewport: crate::session::CanonicalViewport,
    used: Instant,
}
fn error() -> DaemonError {
    DaemonError::LocalTransport {
        operation: "Room shared display",
        message: "Room display lease or binding is unavailable".into(),
    }
}
impl KernelRuntimeState {
    pub(crate) fn is_room_display(&self, id: &str) -> bool {
        self.owned
            .room_display_leases
            .home
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(id)
    }
    pub(crate) async fn open_room_shared_display(
        &self,
        caller: &KernelCaller,
        request: crate::local::GetSliceDisplayEndpointRequest,
    ) -> Result<SliceDisplayEndpoint, DaemonError> {
        let _audience = self.room_display_audience_guard().await;
        let session = request.session_id.as_deref().ok_or_else(error)?;
        let attachment = request.attachment_id.as_deref().ok_or_else(error)?;
        let viewer = request.viewer_public_key.as_deref().ok_or_else(error)?;
        let offer = request.shared_display.ok_or_else(error)?;
        if offer.codecs.is_empty()
            || offer.codecs.len() > 8
            || offer.codecs.iter().any(|codec| codec.len() > 64)
            || !(500000..=20000000).contains(&offer.bitrate)
            || ![1, 2].contains(&offer.device_scale_factor)
            || offer
                .resume_hint
                .as_ref()
                .is_some_and(|hint| hint.len() > 128)
        {
            return Err(error());
        }

        if let Some(hint) = &offer.resume_hint {
            let lease = self.home_display_lease(hint)?;
            let mut lease = lease.lock().await;
            let (slice, mut guard) = self
                .admit_room_display(
                    caller,
                    session,
                    attachment,
                    viewer,
                    &request.slice_ref,
                    true,
                    None,
                )
                .await?;
            if lease.session_id != session
                || lease.attachment_id != attachment
                || lease.slice_id != slice.id
                || lease.caller.user_id != caller.user_id
                || lease.caller.client_id != caller.client_id
            {
                return Err(error());
            }
            // Admission pins the newly authenticated viewer key before the worker resumes.
            // Existing keys may resume; key changes require a fresh lease and producer.
            if lease.viewer_public_key != viewer {
                return Err(error());
            }
            guard.release_queue();
            self.resume_room_display(&mut lease, hint, &slice, false)
                .await?;
            return self.room_display_endpoint(&slice, &lease);
        }
        let (slice, mut guard) = self
            .admit_room_display(
                caller,
                session,
                attachment,
                viewer,
                &request.slice_ref,
                false,
                None,
            )
            .await?;
        guard.release_queue();
        let id = format!("room-display-{:032x}", rand::random::<u128>());
        let room = self
            .room_environment_snapshot(session)
            .map_err(|_| error())?;
        let viewport = room.viewport;
        // Resolve the worker's existing native surface through its ordinary Room
        // observation protocol. It remains the same desktop Room Actions drive.
        use crate::transport::room_browser_controller::RoomBrowserControllerCommand as Command;
        for command in [
            Command::Acquire,
            Command::Reconcile {
                viewport: viewport.clone(),
                browser_bar_visible: room.browser_bar_visible,
            },
        ] {
            match self
                .room_bound_display_peer(
                    &slice,
                    RelayPeerRequest::RoomBrowserController {
                        session_id: session.into(),
                        slice_id: slice.id.clone(),
                        command,
                    },
                )
                .await?
            {
                RelayPeerResponse::RoomBrowserController {
                    session_id,
                    slice_id,
                    ..
                } if session_id == session && slice_id == slice.id => {}
                _ => return Err(error()),
            }
        }
        let observed = self
            .room_bound_display_peer(
                &slice,
                RelayPeerRequest::ObserveRoomComputer {
                    session_id: session.into(),
                    slice_id: slice.id.clone(),
                    call:
                        crate::transport::relay_peer::RemoteRoomComputerObservationCall::Snapshot {
                            observer: id.clone(),
                        },
                },
            )
            .await?;
        let target = match observed {
            RelayPeerResponse::RoomComputerObserved {
                session_id,
                slice_id,
                result,
            } if session_id == session && slice_id == slice.id => {
                let surface_id = result.0.payload["surface_id"]
                    .as_str()
                    .ok_or_else(error)?
                    .to_owned();
                let generation = result.0.payload["generation"]
                    .as_str()
                    .ok_or_else(error)?
                    .to_owned();
                if surface_id != format!("slice:{}", slice.id) || generation.is_empty() {
                    return Err(error());
                }
                RoomDisplayTarget::Desktop {
                    surface_id,
                    generation,
                }
            }
            _ => return Err(error()),
        };
        self.send_room_display_audience(&slice, session, self.room_display_requires_mask(), true)
            .await?;
        let response = self
            .room_display_peer(
                &slice,
                RelayPeerRequest::OpenRoomSharedDisplay {
                    session_id: session.into(),
                    slice_id: slice.id.clone(),
                    lease_id: id.clone(),
                    viewer_public_key: viewer.into(),
                    offer,
                    viewport,
                    target,
                },
            )
            .await?;
        let RoomDisplayResult::Opened {
            binding,
            peer_public_key,
        } = response
        else {
            return Err(error());
        };
        if binding.subscription_id != id {
            return Err(error());
        }
        let lease = RoomDisplayLease {
            caller: caller.clone(),
            session_id: session.into(),
            attachment_id: attachment.into(),
            slice_id: slice.id.clone(),
            viewer_public_key: viewer.into(),
            binding,
            worker_public_key: peer_public_key,
            used: Instant::now(),
        };
        let endpoint = self.room_display_endpoint(&slice, &lease)?;
        let mut leases = self
            .owned
            .room_display_leases
            .home
            .lock()
            .map_err(|_| error())?;
        leases.retain(|_, lease| {
            lease
                .state
                .try_lock()
                .map_or(true, |lease| lease.used.elapsed() < Duration::from_secs(60))
        });
        if leases.len() >= 32 {
            return Err(error());
        }
        leases.insert(id, HomeLease::new(lease));
        drop(leases);
        self.owned.runtime_projection_changes.record_change();
        Ok(endpoint)
    }
    fn home_display_lease(&self, id: &str) -> Result<Arc<Mutex<RoomDisplayLease>>, DaemonError> {
        self.owned
            .room_display_leases
            .home
            .lock()
            .map_err(|_| error())?
            .get(id)
            .map(|lease| lease.state.clone())
            .ok_or_else(error)
    }
    async fn room_display_peer(
        &self,
        slice: &crate::slice::SliceRecord,
        request: RelayPeerRequest,
    ) -> Result<RoomDisplayResult, DaemonError> {
        let expected_session = match &request {
            RelayPeerRequest::SetRoomSharedDisplayMask { session_id, .. }
            | RelayPeerRequest::OpenRoomSharedDisplay { session_id, .. }
            | RelayPeerRequest::RoomSharedDisplay { session_id, .. } => session_id.clone(),
            _ => return Err(error()),
        };
        let result = self.room_bound_display_peer(slice, request).await?;
        match result {
            RelayPeerResponse::RoomSharedDisplay {
                session_id,
                slice_id,
                result,
            } if session_id == expected_session
                && slice_id == slice.id
                && self
                    .room_environment_slice(&session_id)?
                    .is_some_and(|bound| bound.slice_id == slice.id) =>
            {
                Ok(result)
            }
            _ => Err(error()),
        }
    }
    async fn room_bound_display_peer(
        &self,
        slice: &crate::slice::SliceRecord,
        request: RelayPeerRequest,
    ) -> Result<RelayPeerResponse, DaemonError> {
        let config = self.owned.config_projection.snapshot();
        let config = config.slice_relay_override(slice).unwrap_or(config);
        let target = ClientTarget {
            daemon_id: slice.worker_kernel_id.clone(),
            daemon_alias: slice
                .worker_kernel_id
                .is_none()
                .then(|| slice.worker_kernel_ref.clone()),
        };
        self.send_room_slice_peer_request(&config, target, request, Duration::from_secs(15))
            .await
    }
    fn room_display_endpoint(
        &self,
        slice: &crate::slice::SliceRecord,
        lease: &RoomDisplayLease,
    ) -> Result<SliceDisplayEndpoint, DaemonError> {
        let config = self.owned.config_projection.snapshot();
        Ok(SliceDisplayEndpoint {
            slice_id: slice.id.clone(),
            kind: SliceDisplayEndpointKind::Shared,
            url: config.relay_url.ok_or_else(error)?,
            access: SliceDisplayEndpointAccess::Tunnel,
            expires_at_ms: Some(crate::session::unix_epoch_ms() + 60000),
            capabilities: vec!["view".into(), "encrypted".into(), "CXD1".into()],
            stream_protocol: Some("CXD1".into()),
            stream_id: Some(lease.binding.subscription_id.clone()),
            peer_public_key: Some(lease.worker_public_key.clone()),
            shared_display: Some(lease.binding.clone()),
        })
    }
    async fn resume_room_display(
        &self,
        lease: &mut RoomDisplayLease,
        id: &str,
        slice: &crate::slice::SliceRecord,
        attach: bool,
    ) -> Result<RoomDisplayBinding, DaemonError> {
        let result = self
            .room_display_peer(
                &slice,
                RelayPeerRequest::RoomSharedDisplay {
                    session_id: lease.session_id.clone(),
                    slice_id: lease.slice_id.clone(),
                    lease_id: id.into(),
                    command: if attach {
                        RoomDisplayCommand::ResumeAttach {}
                    } else {
                        RoomDisplayCommand::Resume {}
                    },
                },
            )
            .await?;
        let RoomDisplayResult::Updated { binding } = result else {
            return Err(error());
        };
        if binding.subscription_id == id || binding.source != lease.binding.source {
            return Err(error());
        }
        lease.binding = binding.clone();
        lease.used = Instant::now();
        let mut leases = self
            .owned
            .room_display_leases
            .home
            .lock()
            .map_err(|_| error())?;
        let current = leases.remove(id).ok_or_else(error)?;
        leases.insert(binding.subscription_id.clone(), current);
        Ok(binding)
    }
    pub(crate) async fn room_display_control(
        &self,
        caller: &KernelCaller,
        id: &str,
        generation: u64,
        command: RoomDisplayCommand,
    ) -> Result<serde_json::Value, DaemonError> {
        let timing = if matches!(
            &command,
            RoomDisplayCommand::Resume {} | RoomDisplayCommand::ResumeAttach {}
        ) {
            timing::ResumeTiming::start()
        } else {
            None
        };
        if let Some(timing) = &timing {
            timing.mark("entered");
        }
        let lease = self.home_display_lease(id)?;
        let mut lease = lease.lock().await;
        if let Some(timing) = &timing {
            timing.mark("lease_acquired");
        }
        if lease.binding.generation != generation || lease.used.elapsed() >= Duration::from_secs(60)
        {
            return Err(error());
        }
        let (slice, mut guard) = self
            .admit_room_display(
                caller,
                &lease.session_id,
                &lease.attachment_id,
                &lease.viewer_public_key,
                &lease.slice_id,
                matches!(
                    command,
                    RoomDisplayCommand::Resume {} | RoomDisplayCommand::ResumeAttach {}
                ),
                timing.as_ref(),
            )
            .await?;
        guard.release_queue();
        if let Some(timing) = &timing {
            timing.mark("admitted");
        }
        if matches!(
            command,
            RoomDisplayCommand::Resume {} | RoomDisplayCommand::ResumeAttach {}
        ) {
            let attach = matches!(command, RoomDisplayCommand::ResumeAttach {});
            let resumed = self
                .resume_room_display(&mut lease, id, &slice, attach)
                .await;
            if let Some(timing) = &timing {
                timing.mark("peer_returned");
            }
            return serde_json::to_value(resumed?).map_err(|_| error());
        }
        let close = matches!(command, RoomDisplayCommand::Close {});
        let result = self
            .room_display_peer(
                &slice,
                RelayPeerRequest::RoomSharedDisplay {
                    session_id: lease.session_id.clone(),
                    slice_id: lease.slice_id.clone(),
                    lease_id: id.into(),
                    command,
                },
            )
            .await?;
        if !matches!(result, RoomDisplayResult::Settled {}) {
            return Err(error());
        }
        lease.used = Instant::now();
        if close {
            self.owned
                .room_display_leases
                .home
                .lock()
                .map_err(|_| error())?
                .remove(id);
        }
        Ok(serde_json::json!({"generation":generation,"attached":true}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn mp11_room_display_worker_rejects_each_foreign_binding_before_controller_start() {
        let root = crate::test_support::TestWorktree::new("room-shared-display-admission");
        let home = crate::config::DaemonConfig::for_tests();
        let mut config = crate::config::DaemonConfig::for_tests();
        config.user_config_path = root.path().join("config.toml");
        config.user_config.state.path = Some(root.path().join("state.db").display().to_string());
        config.host_machine_id = "slice:slice-1".into();
        config.room_environment_worker_binding =
            Some(crate::config::RoomEnvironmentWorkerBinding {
                home_kernel_id: "home-kernel".into(),
                home_public_key: home.relay_public_key.clone(),
                session_id: "room-1".into(),
                slice_id: "slice-1".into(),
                provisioned_slice_id: None,
            });
        let app = crate::app::DaemonApp::bootstrap(config.clone()).unwrap();
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
            Arc::new(Mutex::new(app)),
            4,
        );
        let state = router.runtime_state();
        for (kernel, key, room, slice) in [
            (
                "foreign",
                home.relay_public_key.as_str(),
                "room-1",
                "slice-1",
            ),
            (
                "home-kernel",
                config.relay_public_key.as_str(),
                "room-1",
                "slice-1",
            ),
            (
                "home-kernel",
                home.relay_public_key.as_str(),
                "foreign",
                "slice-1",
            ),
            (
                "home-kernel",
                home.relay_public_key.as_str(),
                "room-1",
                "foreign",
            ),
        ] {
            let result = state
                .execute_bound_room_shared_display_open(
                    kernel,
                    key,
                    room,
                    slice,
                    "room-display-00000000000000000000000000000001",
                    home.relay_public_key.clone(),
                    RoomDisplayOffer {
                        codecs: vec!["avc1.420033".into()],
                        bitrate: 8000000,
                        device_scale_factor: 2,
                        resume_hint: None,
                    },
                    crate::session::CanonicalViewport::new(1280, 800, 1, 1280, 800).unwrap(),
                    RoomDisplayTarget::Desktop {
                        surface_id: "slice:slice-1".into(),
                        generation: "native-one".into(),
                    },
                )
                .await;
            assert!(
                result.is_err(),
                "MP-11: foreign peer/binding must not reach a desktop producer"
            );
        }
        assert!(state
            .owned
            .room_display_leases
            .worker
            .lock()
            .unwrap()
            .is_empty());
        assert!(state
            .execute_bound_room_shared_display(
                "home-kernel",
                &home.relay_public_key,
                "room-1",
                "slice-1",
                "absent",
                RoomDisplayCommand::Attach {}
            )
            .await
            .is_err());
        state.shutdown_cleanup().await.unwrap();
    }
    #[test]
    fn mp11_room_display_peer_has_no_input_or_client_selected_observer_seam() {
        for value in [
            serde_json::json!({"op":"input","x":1,"y":2}),
            serde_json::json!({"op":"attach","observed_by":"forged"}),
            serde_json::json!({"op":"resume","viewer_public_key":"forged"}),
            serde_json::json!({"op":"resume_attach","observed_by":"forged"}),
        ] {
            assert!(serde_json::from_value::<RoomDisplayCommand>(value).is_err());
        }
    }
}
