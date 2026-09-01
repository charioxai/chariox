use crate::error::DaemonError;
use crate::runtime::state::KernelRuntimeState;

impl KernelRuntimeState {
    pub(in crate::runtime::state::tool_dispatch) async fn dispatch_room_browser_controller_runtime_tool_call(
        &self,
        session_id: &str,
        slice_id: &str,
        agent_id: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        use crate::transport::runtime_tools::*;

        match tool_name {
            SLICE_OPEN_URL_TOOL => {
                let args = parse_controller_tool_arguments::<SliceOpenUrlArgs>(
                    arguments,
                    "runtime_tool_slice_open_url",
                )?;
                self.controller_browser_open_url_compatibility_tool_result(
                    session_id, slice_id, agent_id, &args.url,
                )
                .await
            }
            SLICE_BROWSER_STATUS_TOOL => {
                self.controller_browser_status_tool_result(session_id, slice_id, agent_id)
                    .await
            }
            SLICE_BROWSER_FIND_TOOL => {
                let args = parse_controller_tool_arguments::<SliceBrowserFindArgs>(
                    arguments,
                    "runtime_tool_slice_browser_find",
                )?;
                self.controller_browser_find_tool_result(
                    session_id,
                    slice_id,
                    agent_id,
                    &args.query,
                    args.kind.as_deref().unwrap_or("any"),
                )
                .await
            }
            SLICE_BROWSER_FILL_TOOL => {
                let args = parse_controller_tool_arguments::<SliceBrowserFillArgs>(
                    arguments,
                    "runtime_tool_slice_browser_fill",
                )?;
                self.controller_browser_fill_tool_result(session_id, slice_id, agent_id, args)
                    .await
            }
            SLICE_BROWSER_CLICK_TOOL => {
                let args = parse_controller_tool_arguments::<SliceBrowserClickArgs>(
                    arguments,
                    "runtime_tool_slice_browser_click",
                )?;
                self.controller_browser_click_tool_result(session_id, slice_id, agent_id, args)
                    .await
            }
            SLICE_BROWSER_SUBMIT_TOOL => {
                let args = parse_controller_tool_arguments::<SliceBrowserSubmitArgs>(
                    arguments,
                    "runtime_tool_slice_browser_submit",
                )?;
                self.controller_browser_submit_tool_result(session_id, slice_id, agent_id, args)
                    .await
            }
            SLICE_BROWSER_DIALOG_TOOL => {
                let args = parse_controller_tool_arguments::<SliceBrowserDialogArgs>(
                    arguments,
                    "runtime_tool_slice_browser_dialog",
                )?;
                self.controller_browser_dialog_tool_result(session_id, slice_id, agent_id, args)
                    .await
            }
            SLICE_BROWSER_EVENTS_TOOL => {
                let args = parse_controller_tool_arguments::<SliceBrowserEventsArgs>(
                    arguments,
                    "runtime_tool_slice_browser_events",
                )?;
                self.controller_browser_events_tool_result(session_id, slice_id, agent_id, args)
                    .await
            }
            SLICE_BROWSER_DOWNLOADS_TOOL => {
                parse_controller_tool_arguments::<SliceBrowserDownloadsArgs>(
                    arguments,
                    "runtime_tool_slice_browser_downloads",
                )?;
                self.controller_browser_downloads_tool_result(session_id, slice_id, agent_id)
                    .await
            }
            SLICE_BROWSER_UPLOAD_TOOL => {
                let args = parse_controller_tool_arguments::<SliceBrowserUploadArgs>(
                    arguments,
                    "runtime_tool_slice_browser_upload",
                )?;
                self.controller_browser_upload_tool_result(session_id, slice_id, agent_id, args)
                    .await
            }
            SLICE_BROWSER_PERMISSION_TOOL => {
                let args = parse_controller_tool_arguments::<SliceBrowserPermissionArgs>(
                    arguments,
                    "runtime_tool_slice_browser_permission",
                )?;
                self.controller_browser_permission_tool_result(session_id, slice_id, agent_id, args)
                    .await
            }
            SLICE_BROWSER_TEXT_TOOL => {
                self.controller_browser_text_tool_result(session_id, slice_id, agent_id)
                    .await
            }
            SLICE_BROWSER_WAIT_FOR_TEXT_TOOL => {
                let args = parse_controller_tool_arguments::<SliceBrowserWaitForTextArgs>(
                    arguments,
                    "runtime_tool_slice_browser_wait_for_text",
                )?;
                self.controller_browser_wait_for_text_tool_result(
                    session_id,
                    slice_id,
                    agent_id,
                    &args.text,
                    args.timeout_ms,
                )
                .await
            }
            SLICE_BROWSER_WAIT_FOR_SELECTOR_TOOL => {
                let args = parse_controller_tool_arguments::<SliceBrowserWaitForSelectorArgs>(
                    arguments,
                    "runtime_tool_slice_browser_wait_for_selector",
                )?;
                self.controller_browser_wait_for_selector_compatibility_tool_result(
                    session_id,
                    slice_id,
                    agent_id,
                    args.selector,
                    args.timeout_ms,
                )
                .await
            }
            SLICE_BROWSER_WAIT_FOR_IDLE_TOOL => {
                let args = parse_controller_tool_arguments::<SliceBrowserWaitForIdleArgs>(
                    arguments,
                    "runtime_tool_slice_browser_wait_for_idle",
                )?;
                self.controller_browser_wait_for_idle_compatibility_tool_result(
                    session_id,
                    slice_id,
                    agent_id,
                    args.timeout_ms,
                )
                .await
            }
            _ => Err(DaemonError::LocalTransport {
                operation: "dispatch_room_browser_controller_runtime_tool_call",
                message: format!("unsupported Room browser runtime tool `{tool_name}`"),
            }),
        }
    }
}

fn parse_controller_tool_arguments<T: serde::de::DeserializeOwned>(
    arguments: serde_json::Value,
    operation: &'static str,
) -> Result<T, DaemonError> {
    serde_json::from_value(arguments).map_err(|error| DaemonError::LocalTransport {
        operation,
        message: format!("invalid tool arguments: {error}"),
    })
}
