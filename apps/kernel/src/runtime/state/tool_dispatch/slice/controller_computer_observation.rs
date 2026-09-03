use base64::Engine as _;

use crate::error::DaemonError;
use crate::runtime::state::KernelRuntimeState;
use crate::transport::runtime_tools::{RuntimeToolResult, SliceScreenshotArgs};

impl KernelRuntimeState {
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
