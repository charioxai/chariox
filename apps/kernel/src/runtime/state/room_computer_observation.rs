use std::time::Duration;

use chariox_relay::protocol::ClientTarget;

use super::*;
use crate::transport::relay_peer::RemoteRoomComputerObservationCall;
use crate::transport::runtime_tools::RuntimeToolResult;

impl KernelRuntimeState {
    pub(in crate::runtime::state) async fn observe_room_computer_for_agent(
        &self,
        session_id: &str,
        slice_id: &str,
        agent_id: &str,
        call: RemoteRoomComputerObservationCall,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        let pixels = !matches!(&call, RemoteRoomComputerObservationCall::ScreenStatus);
        self.ensure_room_observation_ready(session_id, agent_id, pixels)
            .await?;
        self.owned
            .room_secret_observations
            .require(session_id, pixels)?;
        let slice = self.running_room_screenshot_slice(session_id)?;
        if slice.id != slice_id {
            return Err(computer_observation_error(
                "the requested slice is not the Room Environment slice",
            ));
        }
        let canonical_viewport = self
            .room_environment_snapshot(session_id)
            .map_err(|error| {
                computer_observation_error(&format!(
                    "Room Computer state is unavailable: {}",
                    error.code()
                ))
            })?
            .viewport;
        let _guard = self
            .owned
            .slice_store
            .queue_environment_use(&slice.id, Some(session_id), "environment.computer.observe")
            .await?;
        let slice = self.running_room_screenshot_slice(session_id)?;
        let config = self.owned.config_projection.snapshot();
        let config = config.slice_relay_override(&slice).unwrap_or(config);
        let target = ClientTarget {
            daemon_id: slice.worker_kernel_id.clone(),
            daemon_alias: slice
                .worker_kernel_id
                .is_none()
                .then(|| slice.worker_kernel_ref.clone()),
        };
        let screen_status = matches!(&call, RemoteRoomComputerObservationCall::ScreenStatus);
        let response = self
            .send_room_slice_peer_request(
                &config,
                target,
                RelayPeerRequest::ObserveRoomComputer {
                    session_id: session_id.to_string(),
                    slice_id: slice.id.clone(),
                    call,
                },
                Duration::from_secs(15),
            )
            .await?;
        let RelayPeerResponse::RoomComputerObserved {
            session_id: returned_session_id,
            slice_id: returned_slice_id,
            result,
        } = response
        else {
            return Err(computer_observation_error(
                "worker returned an unexpected Computer observation response",
            ));
        };
        if returned_session_id != session_id || returned_slice_id != slice.id {
            return Err(computer_observation_error(
                "worker returned mismatched Computer observation metadata",
            ));
        }
        let result = self
            .owned
            .room_secret_observations
            .scrub(session_id, result.0)?;
        authoritative_computer_observation_result(
            result,
            screen_status,
            session_id,
            &slice.id,
            agent_id,
            &canonical_viewport,
        )
    }

    pub(crate) async fn execute_bound_room_computer_observation(
        &self,
        authenticated_kernel_id: &str,
        authenticated_public_key: &str,
        session_id: &str,
        slice_id: &str,
        call: RemoteRoomComputerObservationCall,
    ) -> Result<RuntimeToolResult, DaemonError> {
        self.authorize_bound_room_computer_read(
            authenticated_kernel_id,
            authenticated_public_key,
            session_id,
            slice_id,
        )?;
        let _observation_guard = self
            .owned
            .room_secret_observations
            .barrier(session_id)?
            .read_owned()
            .await;
        self.owned.room_secret_observations.require(
            session_id,
            !matches!(&call, RemoteRoomComputerObservationCall::ScreenStatus),
        )?;
        let capture_guard = self
            .owned
            .computer_input_executions
            .capture_guard()
            .map_err(computer_observation_error)?;
        if let RemoteRoomComputerObservationCall::Snapshot { observer } = &call {
            if observer.is_empty() || observer.len() > 256 {
                return Err(computer_observation_error("invalid observer"));
            }
            let policy = self
                .owned
                .room_secret_observations
                .capture_policy(session_id)?;
            let policy: serde_json::Value = serde_json::from_str(&policy)
                .map_err(|_| computer_observation_error("protection unavailable"))?;
            let processes = self.owned.browser_controller_processes.clone();
            let room = session_id.to_string();
            let params = serde_json::json!({"observer":observer,"policy":policy});
            let result = tokio::task::spawn_blocking(move || {
                processes.room_computer_request(&room, "computer.snapshot", params)
            })
            .await
            .map_err(|_| computer_observation_error("snapshot task failed"))?
            .map_err(|_| computer_observation_error("accessibility snapshot unavailable"))?;
            return self.owned.room_secret_observations.scrub(
                session_id,
                RuntimeToolResult {
                    ok: true,
                    payload: result,
                },
            );
        }
        // Preserve opaque-artifact ownership checks without reading old pixels.
        let artifact_id = match &call {
            RemoteRoomComputerObservationCall::Ocr { artifact_id }
            | RemoteRoomComputerObservationCall::FindText { artifact_id, .. } => {
                artifact_id.as_deref()
            }
            RemoteRoomComputerObservationCall::ScreenStatus
            | RemoteRoomComputerObservationCall::Snapshot { .. } => None,
        };
        if let Some(artifact_id) = artifact_id {
            super::room_screenshot::load_room_screenshot_artifact(
                &self.owned.config_projection.snapshot(),
                session_id,
                slice_id,
                artifact_id,
            )?;
        }
        // Old/pre-redaction artifacts are never OCR sources. Re-capture fresh.
        let result = super::tool_dispatch::execute_room_computer_observation(
            call,
            self.owned
                .room_secret_observations
                .capture_policy(session_id)?,
            capture_guard,
            |ids| {
                self.owned
                    .room_secret_observations
                    .prune_native_targets(session_id, ids)
            },
        )
        .await?;
        self.owned
            .room_secret_observations
            .scrub(session_id, result)
    }
}

fn authoritative_computer_observation_result(
    mut result: RuntimeToolResult,
    screen_status: bool,
    session_id: &str,
    slice_id: &str,
    agent_id: &str,
    canonical_viewport: &crate::session::CanonicalViewport,
) -> Result<RuntimeToolResult, DaemonError> {
    let screen_size = format!(
        "{}x{}",
        canonical_viewport.desktop_pixel_width, canonical_viewport.desktop_pixel_height
    );
    let canonical_viewport_value = screen_status
        .then(|| serde_json::to_value(canonical_viewport))
        .transpose()
        .map_err(|error| {
            computer_observation_error(&format!(
                "failed to serialize the canonical Computer viewport: {error}"
            ))
        })?;
    let payload = result.payload.as_object_mut().ok_or_else(|| {
        computer_observation_error("worker returned a non-object Computer observation payload")
    })?;
    for field in [
        "source",
        "session_id",
        "slice_id",
        "agent_id",
        "display",
        "viewer",
        "stdout",
        "stderr",
    ] {
        payload.remove(field);
    }
    if let Some(canonical_viewport_value) = canonical_viewport_value {
        payload.insert(
            "viewer_access".to_string(),
            serde_json::Value::String("client_attachment_required".to_string()),
        );
        payload.insert("screen".to_string(), serde_json::Value::String(screen_size));
        payload.insert("canonical_viewport".to_string(), canonical_viewport_value);
    }
    payload.insert(
        "source".to_string(),
        serde_json::Value::String("computer_controller".to_string()),
    );
    payload.insert(
        "session_id".to_string(),
        serde_json::Value::String(session_id.to_string()),
    );
    payload.insert(
        "slice_id".to_string(),
        serde_json::Value::String(slice_id.to_string()),
    );
    payload.insert(
        "agent_id".to_string(),
        serde_json::Value::String(agent_id.to_string()),
    );
    Ok(result)
}

fn computer_observation_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "environment.computer.observe",
        message: message.to_string(),
    }
}
