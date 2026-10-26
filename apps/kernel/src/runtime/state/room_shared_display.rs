//! MP-08/MP-10/MP-11: home owns leases; worker owns the exact Room producer.
#[cfg(test)]
mod admission_tests;
pub(super) mod timing;
mod worker;
use super::room_display::RoomDisplayAdmission;
use super::*;
use crate::runtime::command::KernelCaller;
use crate::slice::{
    RoomDisplayBinding, RoomDisplayOffer, SliceDisplayEndpoint, SliceDisplayEndpointAccess,
    SliceDisplayEndpointKind,
};
use crate::transport::room_shared_display::{
    RoomDisplayAudience, RoomDisplayCommand, RoomDisplayResult, RoomDisplayTarget,
    SealedRoomDisplayFrame,
};
#[derive(Clone, Default)]
pub(super) struct RoomDisplayLeases {
    home: Arc<std::sync::Mutex<BTreeMap<String, Arc<Mutex<RoomDisplayLease>>>>>,
    worker: Arc<std::sync::Mutex<BTreeMap<String, Arc<Mutex<WorkerLease>>>>>,
}
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
/// MP-11: only the Room owner's display open drives the shared browser
/// (acquire + viewport reconcile); an invited viewer watches read-only.
fn room_display_prelude(
    audience: RoomDisplayAudience,
    viewport: &crate::session::CanonicalViewport,
    browser_bar_visible: bool,
) -> Vec<crate::transport::room_browser_controller::RoomBrowserControllerCommand> {
    use crate::transport::room_browser_controller::RoomBrowserControllerCommand as Command;
    match audience {
        RoomDisplayAudience::Owner => vec![
            Command::Acquire,
            Command::Reconcile {
                viewport: viewport.clone(),
                browser_bar_visible,
            },
        ],
        RoomDisplayAudience::Viewer => Vec::new(),
    }
}
impl KernelRuntimeState {
    fn room_display_audience(
        &self,
        caller: &KernelCaller,
        session_id: &str,
    ) -> Result<RoomDisplayAudience, DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        let user = caller.user_id.as_deref().or_else(|| {
            matches!(
                caller.caller_kind,
                crate::runtime::command::KernelCallerKind::LocalClient
            )
            .then_some(crate::session::DEFAULT_LOCAL_USER_ID)
        });
        Ok(if user == Some(session.owner_user_id()) {
            RoomDisplayAudience::Owner
        } else {
            RoomDisplayAudience::Viewer
        })
    }
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
                    RoomDisplayAdmission {
                        session_id: session,
                        attachment_id: attachment,
                        viewer_public_key: viewer,
                        slice_ref: &request.slice_ref,
                    },
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
                RoomDisplayAdmission {
                    session_id: session,
                    attachment_id: attachment,
                    viewer_public_key: viewer,
                    slice_ref: &request.slice_ref,
                },
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
        let audience = self.room_display_audience(caller, session)?;
        for command in room_display_prelude(audience, &viewport, room.browser_bar_visible) {
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
        let response = self
            .room_display_peer(
                &slice,
                RelayPeerRequest::OpenRoomSharedDisplay {
                    session_id: session.into(),
                    slice_id: slice.id.clone(),
                    lease_id: id.clone(),
                    viewer_public_key: viewer.into(),
                    audience,
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
                .try_lock()
                .map_or(true, |lease| lease.used.elapsed() < Duration::from_secs(60))
        });
        if leases.len() >= 32 {
            return Err(error());
        }
        leases.insert(id, Arc::new(Mutex::new(lease)));
        Ok(endpoint)
    }
    fn home_display_lease(&self, id: &str) -> Result<Arc<Mutex<RoomDisplayLease>>, DaemonError> {
        self.owned
            .room_display_leases
            .home
            .lock()
            .map_err(|_| error())?
            .get(id)
            .cloned()
            .ok_or_else(error)
    }
    async fn room_display_peer(
        &self,
        slice: &crate::slice::SliceRecord,
        request: RelayPeerRequest,
    ) -> Result<RoomDisplayResult, DaemonError> {
        let expected_session = match &request {
            RelayPeerRequest::OpenRoomSharedDisplay { session_id, .. }
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
                slice,
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
                RoomDisplayAdmission {
                    session_id: &lease.session_id,
                    attachment_id: &lease.attachment_id,
                    viewer_public_key: &lease.viewer_public_key,
                    slice_ref: &lease.slice_id,
                },
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
    pub(crate) async fn room_display_frame(
        &self,
        caller: &KernelCaller,
        id: &str,
        generation: u64,
        after_sequence: u64,
        reset: bool,
        congested: bool,
    ) -> Result<Option<SealedRoomDisplayFrame>, DaemonError> {
        let lease = self.home_display_lease(id)?;
        let mut lease = lease.lock().await;
        if lease.binding.generation != generation || lease.used.elapsed() >= Duration::from_secs(60)
        {
            return Err(error());
        }
        let (slice, _guard) = self
            .admit_room_display(
                caller,
                RoomDisplayAdmission {
                    session_id: &lease.session_id,
                    attachment_id: &lease.attachment_id,
                    viewer_public_key: &lease.viewer_public_key,
                    slice_ref: &lease.slice_id,
                },
                true,
                None,
            )
            .await?;
        let result = self
            .room_display_peer(
                &slice,
                RelayPeerRequest::RoomSharedDisplay {
                    session_id: lease.session_id.clone(),
                    slice_id: lease.slice_id.clone(),
                    lease_id: id.into(),
                    command: RoomDisplayCommand::Capture {
                        after_sequence,
                        reset,
                        congested,
                    },
                },
            )
            .await?;
        let RoomDisplayResult::Frame { frame } = result else {
            return Err(error());
        };
        lease.used = Instant::now();
        Ok(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mp11_room_display_viewer_open_never_drives_the_shared_browser() {
        let viewport = crate::session::CanonicalViewport::new(1280, 720, 1, 1280, 720).unwrap();
        assert_eq!(
            room_display_prelude(RoomDisplayAudience::Owner, &viewport, false).len(),
            2
        );
        assert!(
            room_display_prelude(RoomDisplayAudience::Viewer, &viewport, false).is_empty(),
            "MP-11 a read-only viewer must not acquire or resize the owner's browser"
        );
    }
    #[tokio::test]
    async fn mp11_room_display_home_distinguishes_room_owner_from_invited_human() {
        let root = crate::test_support::TestWorktree::new("room-display-audiences");
        let mut config = crate::config::DaemonConfig::for_tests();
        config.user_config.state.path = Some(root.path().join("state.db").display().to_string());
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::ProcessMemory;
        let mut app = crate::app::DaemonApp::bootstrap(config).unwrap();
        let (session, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(root.session_request().with_owner_user_id("owner"))
            .unwrap();
        let session = session.id().to_string();
        let state = crate::runtime::router::CommandRouter::with_interactive_capacity(
            Arc::new(Mutex::new(app)),
            4,
        )
        .runtime_state();
        state
            .create_session_invite(
                &session,
                "invite".into(),
                "owner".into(),
                None,
                None,
                crate::session::CollaborationLevel::Full,
            )
            .unwrap();
        state
            .join_session_invite(&session, "invite", "invited".into(), 1)
            .unwrap();
        for (user, expected) in [
            ("owner", RoomDisplayAudience::Owner),
            ("invited", RoomDisplayAudience::Viewer),
        ] {
            let caller = KernelCaller {
                caller_kind: crate::runtime::command::KernelCallerKind::RemoteClient,
                user_id: Some(user.into()),
                client_id: Some(format!("{user}-client")),
                ..KernelCaller::default()
            };
            assert_eq!(
                state.room_display_audience(&caller, &session).unwrap(),
                expected,
                "MP-11 attachment ownership must not imply Room ownership"
            );
        }
        state.shutdown_cleanup().await.unwrap();
    }
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
                    RoomDisplayAudience::Viewer,
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
