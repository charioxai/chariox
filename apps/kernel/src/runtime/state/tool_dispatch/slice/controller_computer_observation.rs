use base64::Engine as _;

use crate::error::DaemonError;
use crate::runtime::state::KernelRuntimeState;
use crate::transport::runtime_tools::{
    RuntimeToolResult, SliceFindTextArgs, SliceOcrArgs, SliceScreenshotArgs,
};

impl KernelRuntimeState {
    pub(super) async fn controller_computer_screen_status_tool_result(
        &self,
        session_id: &str,
        slice_id: &str,
        agent_id: &str,
    ) -> Result<RuntimeToolResult, DaemonError> {
        self.observe_room_computer_for_agent(
            session_id,
            slice_id,
            agent_id,
            crate::transport::relay_peer::RemoteRoomComputerObservationCall::ScreenStatus,
        )
        .await
    }

    pub(super) async fn controller_computer_ocr_tool_result(
        &self,
        session_id: &str,
        slice_id: &str,
        agent_id: &str,
        args: SliceOcrArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        if args.image_path.is_some() {
            return Err(DaemonError::LocalTransport {
                operation: "runtime_tool_slice_ocr",
                message: "Room Computer OCR accepts an opaque artifact_id, not image_path"
                    .to_string(),
            });
        }
        self.observe_room_computer_for_agent(
            session_id,
            slice_id,
            agent_id,
            crate::transport::relay_peer::RemoteRoomComputerObservationCall::Ocr {
                artifact_id: args.artifact_id,
            },
        )
        .await
    }

    pub(super) async fn controller_computer_find_text_tool_result(
        &self,
        session_id: &str,
        slice_id: &str,
        agent_id: &str,
        args: SliceFindTextArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        if args.image_path.is_some() {
            return Err(DaemonError::LocalTransport {
                operation: "runtime_tool_slice_find_text",
                message: "Room Computer text lookup accepts an opaque artifact_id, not image_path"
                    .to_string(),
            });
        }
        let query = super::validated_slice_find_text_query(&args.query)?;
        self.observe_room_computer_for_agent(
            session_id,
            slice_id,
            agent_id,
            crate::transport::relay_peer::RemoteRoomComputerObservationCall::FindText {
                query,
                artifact_id: args.artifact_id,
            },
        )
        .await
    }

    pub(super) async fn controller_computer_screenshot_tool_result(
        &self,
        session_id: &str,
        slice_id: &str,
        agent_id: &str,
        args: SliceScreenshotArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let observed = self
            .capture_room_environment_screenshot_for_agent(
                session_id,
                agent_id,
                args.return_image_base64,
            )
            .await?;
        let artifact = observed.artifact;
        let mut payload = serde_json::json!({
            "source": "computer_controller",
            "session_id": session_id,
            "slice_id": slice_id,
            "agent_id": agent_id,
            "artifact_id": artifact.artifact_id,
            "sha256": artifact.sha256,
            "size_bytes": artifact.size_bytes,
            "mime_type": artifact.media_type,
            "display_name": artifact.display_name,
        });
        if let Some(image_bytes) = observed.image_bytes {
            payload["image_base64"] = serde_json::Value::String(
                base64::engine::general_purpose::STANDARD.encode(image_bytes),
            );
        }
        Ok(RuntimeToolResult { ok: true, payload })
    }
}
