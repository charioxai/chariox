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

        let result = match tool_name {
            SLICE_ACCESSIBILITY_TOOL => {
                let args = parse_controller_tool_arguments::<serde_json::Map<String, serde_json::Value>>(arguments, "slice_accessibility")?;
                if !args.is_empty() { return Err(DaemonError::LocalTransport {operation:"slice_accessibility",message:"snapshot accepts no arguments".into()}); }
                self.observe_room_computer_for_agent(session_id, slice_id, agent_id, crate::transport::relay_peer::RemoteRoomComputerObservationCall::Snapshot { observer: crate::session::agent_environment_actor_id(agent_id) }).await
            }
            SLICE_TARGET_ACTION_TOOL => {
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Args { tree_revision:u64, target_id:String, action:String }
                let args = parse_controller_tool_arguments::<Args>(arguments, "slice_target_action")?;
                self.controller_computer_input_tool_result(session_id, slice_id, agent_id, crate::transport::room_browser_controller::RoomComputerInputAction::TargetAction { tree_revision:args.tree_revision,target_id:args.target_id,action:args.action }).await
            }
            PASTE_SECRET_TO_SLICE_TOOL => {
                let args = parse_controller_tool_arguments::<PasteSecretToSliceArgs>(
                    arguments,
                    "runtime_tool_paste_secret_to_slice",
                )?;
                self.controller_paste_secret_to_slice_tool_result(
                    session_id, slice_id, agent_id, args,
                )
                .await
            }
            SLICE_SCREEN_STATUS_TOOL => {
                parse_controller_tool_arguments::<serde_json::Map<String, serde_json::Value>>(
                    arguments,
                    "runtime_tool_slice_screen_status",
                )?;
                self.controller_computer_screen_status_tool_result(session_id, slice_id, agent_id)
                    .await
            }
            SLICE_SCREENSHOT_TOOL => {
                let args = parse_controller_tool_arguments::<SliceScreenshotArgs>(
                    arguments,
                    "runtime_tool_slice_screenshot",
                )?;
                self.controller_computer_screenshot_tool_result(
                    session_id, slice_id, agent_id, args,
                )
                .await
            }
            SLICE_OCR_TOOL => {
                let args = parse_controller_tool_arguments::<SliceOcrArgs>(
                    arguments,
                    "runtime_tool_slice_ocr",
                )?;
                self.controller_computer_ocr_tool_result(session_id, slice_id, agent_id, args)
                    .await
            }
            SLICE_FIND_TEXT_TOOL => {
                let args = parse_controller_tool_arguments::<SliceFindTextArgs>(
                    arguments,
                    "runtime_tool_slice_find_text",
                )?;
                self.controller_computer_find_text_tool_result(session_id, slice_id, agent_id, args)
                    .await
            }
            SLICE_MOUSE_TOOL => {
                let args = parse_controller_tool_arguments::<SliceMouseArgs>(
                    arguments,
                    "runtime_tool_slice_mouse",
                )?;
                self.controller_computer_mouse_tool_result(session_id, slice_id, agent_id, args)
                    .await
            }
            SLICE_KEYBOARD_TOOL => {
                let args = parse_controller_tool_arguments::<SliceKeyboardArgs>(
                    arguments,
                    "runtime_tool_slice_keyboard",
                )?;
                self.controller_computer_keyboard_tool_result(session_id, slice_id, agent_id, args)
                    .await
            }
            SLICE_CLIPBOARD_WRITE_TOOL => {
                let args = parse_controller_tool_arguments::<SliceClipboardWriteArgs>(
                    arguments,
                    "runtime_tool_slice_clipboard_write",
                )?;
                self.controller_computer_clipboard_write_tool_result(
                    session_id, slice_id, agent_id, args,
                )
                .await
            }
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
            SLICE_BROWSER_TAB_TOOL => {
                let args = parse_controller_tool_arguments::<SliceBrowserTabArgs>(
                    arguments,
                    "runtime_tool_slice_browser_tab",
                )?;
                self.controller_browser_tab_tool_result(session_id, slice_id, agent_id, args)
                    .await
            }
            SLICE_BROWSER_HISTORY_TOOL => {
                let args = parse_controller_tool_arguments::<SliceBrowserHistoryArgs>(
                    arguments,
                    "runtime_tool_slice_browser_history",
                )?;
                self.controller_browser_history_tool_result(session_id, slice_id, agent_id, args)
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
                let args = parse_controller_tool_arguments::<SliceBrowserDownloadsArgs>(
                    arguments,
                    "runtime_tool_slice_browser_downloads",
                )?;
                self.controller_browser_downloads_tool_result(session_id, slice_id, agent_id, args)
                    .await
            }
            SLICE_BROWSER_ARTIFACT_TOOL => {
                let args = parse_controller_tool_arguments::<SliceBrowserArtifactArgs>(
                    arguments,
                    "runtime_tool_slice_browser_artifact",
                )?;
                self.ensure_room_observation_ready(session_id, agent_id, true)
                    .await?;
                let environment = super::controller_browser::ensure_controller_browser_environment(
                    self,
                    session_id,
                    "runtime_tool_slice_browser_artifact",
                )
                .await?;
                let tab_id =
                    environment
                        .focused_tab_id
                        .ok_or_else(|| DaemonError::LocalTransport {
                            operation: "runtime_tool_slice_browser_artifact",
                            message: "Room browser has no focused tab".into(),
                        })?;
                self.execute_room_browser_artifact(session_id, &tab_id, args)
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
        .map_err(|error| {
            self.owned
                .room_secret_observations
                .scrub_error(session_id, error)
        })?;
        self.owned
            .room_secret_observations
            .scrub(session_id, result)
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
