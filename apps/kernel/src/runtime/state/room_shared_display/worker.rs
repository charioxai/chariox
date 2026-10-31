//! MP-08/MP-10/MP-11: exact bound-worker capture and viewer-key sealing.
use super::*;
fn display_subscribe_params(
    lease_id: &str,
    offer: RoomDisplayOffer,
    viewport: &crate::session::CanonicalViewport,
    target: RoomDisplayTarget,
    audience: RoomDisplayAudience,
) -> serde_json::Value {
    let mut params = serde_json::json!({"op":"display_subscribe","lease_id":lease_id,"observed_by":lease_id,"codecs":offer.codecs,"bitrate":offer.bitrate,"device_scale_factor":offer.device_scale_factor,"viewport":viewport,"target":target});
    if audience == RoomDisplayAudience::Owner {
        params["audience"] = "owner".into();
    }
    params
}
impl KernelRuntimeState {
    async fn room_producer_request(
        &self,
        session_id: &str,
        mut params: serde_json::Value,
    ) -> Result<serde_json::Value, DaemonError> {
        params["policy"] = serde_json::from_str(
            &self
                .owned
                .room_secret_observations
                .capture_policy(session_id)?,
        )
        .map_err(|_| error())?;
        let processes = self.owned.browser_controller_processes.clone();
        let room = session_id.to_string();
        tokio::task::spawn_blocking(move || {
            processes.room_computer_request(&room, "room.display", params)
        })
        .await
        .map_err(|_| error())?
        .map_err(|message| DaemonError::LocalTransport {
            operation: "Room shared display producer",
            message,
        })
    }
    // Keep authenticated peer identity explicit alongside the peer protocol fields.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn execute_bound_room_shared_display_open(
        &self,
        home: &str,
        key: &str,
        session_id: &str,
        slice_id: &str,
        lease_id: &str,
        viewer_public_key: String,
        audience: RoomDisplayAudience,
        offer: RoomDisplayOffer,
        viewport: crate::session::CanonicalViewport,
        target: RoomDisplayTarget,
    ) -> Result<RoomDisplayResult, DaemonError> {
        let config = self.authorize_bound_room_computer_read(home, key, session_id, slice_id)?;
        crate::transport::relay_crypto::decode_public_key(&viewer_public_key)
            .map_err(|_| error())?;
        if lease_id
            .strip_prefix("room-display-")
            .is_none_or(|id| id.len() != 32 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()))
            || offer.resume_hint.is_some()
            || offer.codecs.is_empty()
            || offer.codecs.len() > 8
        {
            return Err(error());
        }
        {
            let mut leases = self
                .owned
                .room_display_leases
                .worker
                .lock()
                .map_err(|_| error())?;
            leases.retain(|_, lease| {
                lease
                    .try_lock()
                    .map_or(true, |lease| lease.used.elapsed() < Duration::from_secs(60))
            });
            if leases.contains_key(lease_id) || leases.len() >= 32 {
                return Err(error());
            }
        }
        if !matches!(&target,RoomDisplayTarget::Desktop{surface_id,..} if surface_id==&format!("slice:{slice_id}"))
        {
            return Err(error());
        }
        let _barrier = self
            .owned
            .room_secret_observations
            .barrier(session_id)?
            .read_owned()
            .await;
        self.owned
            .room_secret_observations
            .require(session_id, true)?;
        let _capture = self
            .owned
            .computer_input_executions
            .capture_guard()
            .map_err(|_| error())?;
        let result = self
            .room_producer_request(
                session_id,
                display_subscribe_params(lease_id, offer, &viewport, target, audience),
            )
            .await?;
        let binding: RoomDisplayBinding = serde_json::from_value(result).map_err(|_| error())?;
        if binding.subscription_id != lease_id
            || binding.source.kind != "desktop"
            || binding.source.surface_id != format!("slice:{slice_id}")
        {
            return Err(error());
        }
        let lease = WorkerLease {
            home: home.into(),
            home_key: key.into(),
            session_id: session_id.into(),
            slice_id: slice_id.into(),
            viewer_public_key,
            binding: binding.clone(),
            viewport,
            used: Instant::now(),
        };
        let mut leases = self
            .owned
            .room_display_leases
            .worker
            .lock()
            .map_err(|_| error())?;
        leases.retain(|_, lease| {
            lease
                .try_lock()
                .map_or(true, |lease| lease.used.elapsed() < Duration::from_secs(60))
        });
        leases.insert(lease_id.into(), Arc::new(Mutex::new(lease)));
        Ok(RoomDisplayResult::Opened {
            binding,
            peer_public_key: config.relay_public_key,
        })
    }
    pub(crate) async fn execute_bound_room_shared_display(
        &self,
        home: &str,
        key: &str,
        session_id: &str,
        slice_id: &str,
        lease_id: &str,
        command: RoomDisplayCommand,
    ) -> Result<RoomDisplayResult, DaemonError> {
        let config = self.authorize_bound_room_computer_read(home, key, session_id, slice_id)?;
        let lease = self
            .owned
            .room_display_leases
            .worker
            .lock()
            .map_err(|_| error())?
            .get(lease_id)
            .cloned()
            .ok_or_else(error)?;
        let mut lease = lease.lock().await;
        if lease.home != home
            || lease.home_key != key
            || lease.session_id != session_id
            || lease.slice_id != slice_id
            || lease.used.elapsed() >= Duration::from_secs(60)
        {
            return Err(error());
        }
        let _barrier = self
            .owned
            .room_secret_observations
            .barrier(session_id)?
            .read_owned()
            .await;
        self.owned
            .room_secret_observations
            .require(session_id, true)?;
        let _capture = self
            .owned
            .computer_input_executions
            .capture_guard()
            .map_err(|_| error())?;
        let op = producer_op(&command);
        let mut params = serde_json::json!({"op":op,"subscription_id":lease_id,"observed_by":lease_id,"generation":lease.binding.generation,"viewport":lease.viewport});
        if let RoomDisplayCommand::Capture {
            after_sequence,
            reset,
            congested,
        } = &command
        {
            params["after_sequence"] = (*after_sequence).into();
            params["push"] = serde_json::json!({"reset":reset,"congested":congested});
        }
        let mut result = self.room_producer_request(session_id, params).await?;
        lease.used = Instant::now();
        match command {
            RoomDisplayCommand::Resume {} | RoomDisplayCommand::ResumeAttach {} => {
                let binding: RoomDisplayBinding =
                    serde_json::from_value(result).map_err(|_| error())?;
                if binding.subscription_id == lease_id || binding.source != lease.binding.source {
                    return Err(error());
                }
                lease.binding = binding.clone();
                let mut leases = self
                    .owned
                    .room_display_leases
                    .worker
                    .lock()
                    .map_err(|_| error())?;
                let current = leases.remove(lease_id).ok_or_else(error)?;
                leases.insert(binding.subscription_id.clone(), current);
                Ok(RoomDisplayResult::Updated { binding })
            }
            RoomDisplayCommand::Capture { .. } => {
                if result["display_frame"].is_null() {
                    return Ok(RoomDisplayResult::Frame { frame: None });
                }
                if result["display_frame"]["subscription_id"] != lease_id
                    || result["display_frame"]["generation"] != lease.binding.generation
                {
                    return Err(error());
                }
                let mut response = serde_json::json!({"KernelBrowser":{"result":result.take()}});
                let (_, sequence, event) =
                    crate::transport::kernel_browser_display::take_display_event(&mut response)
                        .ok_or_else(error)?;
                let bytes = crate::transport::kernel_browser_display::encode_display_event(event)
                    .map_err(|_| error())?;
                let encrypted_event = crate::transport::relay_crypto::encrypt_payload_for_peer(
                    &config.relay_private_key,
                    &lease.viewer_public_key,
                    &bytes,
                )
                .map_err(|_| error())?;
                Ok(RoomDisplayResult::Frame {
                    frame: Some(SealedRoomDisplayFrame {
                        sequence,
                        encrypted_event,
                    }),
                })
            }
            RoomDisplayCommand::Close {} => {
                self.owned
                    .room_display_leases
                    .worker
                    .lock()
                    .map_err(|_| error())?
                    .remove(lease_id);
                Ok(RoomDisplayResult::Settled {})
            }
            _ => Ok(RoomDisplayResult::Settled {}),
        }
    }
}

fn producer_op(command: &RoomDisplayCommand) -> &'static str {
    match command {
        RoomDisplayCommand::Attach {} => "display_attach",
        RoomDisplayCommand::Park {} => "display_park",
        RoomDisplayCommand::Capture { .. } => "screenshot",
        RoomDisplayCommand::Suspend {} => "display_suspend",
        RoomDisplayCommand::Resume {} => "display_resume",
        RoomDisplayCommand::ResumeAttach {} => "display_resume_attach",
        RoomDisplayCommand::Close {} => "unsubscribe",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::room_shared_display::RoomDisplayAudience;
    /// A paused Room display must reach the worker's desktop source as a pause, not as a plain attach.
    #[test]
    fn mp08_a_parked_room_display_asks_the_producer_to_pause() {
        assert_eq!(producer_op(&RoomDisplayCommand::Park {}), "display_park");
        assert_eq!(
            serde_json::to_value(RoomDisplayCommand::Park {}).unwrap(),
            serde_json::json!({"op":"park"})
        );
        assert!(serde_json::from_value::<RoomDisplayCommand>(
            serde_json::json!({"op":"park","observed_by":"forged"})
        )
        .is_err());
    }
    #[test]
    fn mp11_room_display_preserves_owner_and_invited_viewer_audiences() {
        for audience in [RoomDisplayAudience::Owner, RoomDisplayAudience::Viewer] {
            let params = display_subscribe_params(
                "room-display-00000000000000000000000000000001",
                RoomDisplayOffer {
                    codecs: vec!["avc1.420033".into()],
                    bitrate: 8000000,
                    device_scale_factor: 1,
                    resume_hint: None,
                },
                &crate::session::CanonicalViewport::new(1280, 800, 1, 1280, 800).unwrap(),
                RoomDisplayTarget::Desktop {
                    surface_id: "slice:slice-1".into(),
                    generation: "native-one".into(),
                },
                audience,
            );
            match audience {
                RoomDisplayAudience::Owner => assert_eq!(params["audience"], "owner"),
                RoomDisplayAudience::Viewer => assert!(
                    params.get("audience").is_none(),
                    "MP-11 invited viewers use the masked default"
                ),
            }
        }
    }
}
