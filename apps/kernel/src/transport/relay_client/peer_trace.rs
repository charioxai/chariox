//! Safe operation labels for outbound peer diagnostics; request payloads stay private.

use super::peer_client::elapsed_ms_u64;
use super::*;
use crate::transport::relay_peer::RelayPeerRequest;
use crate::transport::room_browser_controller::RoomBrowserControllerCommand;

#[derive(Debug)]
pub(super) struct RelayPeerRequestTrace {
    request_id: String,
    request_kind: &'static str,
    request_operation: Option<&'static str>,
    target_daemon_id: Option<String>,
    target_daemon_alias: Option<String>,
    transport: &'static str,
    started_at: Instant,
    timeout_ms: u64,
}

impl RelayPeerRequestTrace {
    pub(super) fn new(
        request_id: &str,
        request: &RelayPeerRequest,
        target: &ClientTarget,
        transport: &'static str,
        timeout_ms: u64,
    ) -> Self {
        Self {
            request_id: request_id.to_string(),
            request_kind: request_kind(request),
            request_operation: match request {
                RelayPeerRequest::RoomBrowserController { command, .. } => {
                    Some(browser_operation(command))
                }
                _ => None,
            },
            target_daemon_id: target.daemon_id.clone(),
            target_daemon_alias: target.daemon_alias.clone(),
            transport,
            started_at: Instant::now(),
            timeout_ms,
        }
    }

    pub(super) fn fields(
        &self,
        status: &'static str,
        error: Option<&str>,
        from_daemon_id: Option<&str>,
    ) -> serde_json::Value {
        serde_json::json!({
            "request_id": self.request_id,
            "request_kind": self.request_kind,
            "request_operation": self.request_operation,
            "waiting_for": "authenticated relay peer response",
            "target_daemon_id": self.target_daemon_id,
            "target_daemon_alias": self.target_daemon_alias,
            "from_daemon_id": from_daemon_id,
            "transport": self.transport,
            "status": status,
            "relay_rtt_ms": elapsed_ms_u64(self.started_at),
            "timeout_ms": self.timeout_ms,
            "error": error,
        })
    }

    pub(super) fn log_completed(
        &self,
        status: &'static str,
        error: Option<&str>,
        from_daemon_id: Option<&str>,
    ) {
        let fields = self.fields(status, error, from_daemon_id);
        if error.is_some() {
            crate::logging::warn_with_fields(
                "daemon.relay_client",
                "relay peer request completed",
                fields,
            );
        } else {
            crate::logging::info_with_fields(
                "daemon.relay_client",
                "relay peer request completed",
                fields,
            );
        }
    }
}

fn request_kind(request: &RelayPeerRequest) -> &'static str {
    match request {
        RelayPeerRequest::AgentMove { .. } => "agent_move",
        RelayPeerRequest::ListWorkflowNotificationSources { .. } => {
            "list_workflow_notification_sources"
        }
        RelayPeerRequest::SubscribeWorkflowNotifications { .. } => {
            "subscribe_workflow_notifications"
        }
        RelayPeerRequest::UnsubscribeWorkflowNotifications { .. } => {
            "unsubscribe_workflow_notifications"
        }
        RelayPeerRequest::DeliverWorkflowNotification { .. } => "deliver_workflow_notification",
        RelayPeerRequest::PeerAgentMessage { .. } => "peer_agent_message",
        RelayPeerRequest::PeerAgentCancel { .. } => "peer_agent_cancel",
        RelayPeerRequest::SubscribePeerAgent { .. } => "subscribe_peer_agent",
        RelayPeerRequest::UnsubscribePeerAgent { .. } => "unsubscribe_peer_agent",
        RelayPeerRequest::SpawnRoomPeer { .. } => "spawn_room_peer",
        RelayPeerRequest::DeliverPeerAgentOutcome { .. } => "deliver_peer_agent_outcome",
        RelayPeerRequest::RoomBrowserController { .. } => "room_browser_controller",
        RelayPeerRequest::OpenRoomSharedDisplay { .. } => "open_room_shared_display_with_audience",
        RelayPeerRequest::RoomSharedDisplay { .. } => "room_shared_display",
        RelayPeerRequest::OpenRoomDisplay { .. } => "open_room_display",
        RelayPeerRequest::CaptureRoomScreenshot { .. } => "capture_room_screenshot",
        RelayPeerRequest::ReadRoomScreenshotChunk { .. } => "read_room_screenshot_chunk",
        RelayPeerRequest::ObserveRoomComputer { .. } => "observe_room_computer",
        RelayPeerRequest::Ping { .. } => "ping",
        RelayPeerRequest::InstallManagedSliceRelayToken { .. } => {
            "install_managed_slice_relay_token"
        }
        RelayPeerRequest::ConfirmManagedSliceRelayToken { .. } => {
            "confirm_managed_slice_relay_token"
        }
        RelayPeerRequest::RefreshManagedSliceRelayToken { .. } => {
            "refresh_managed_slice_relay_token"
        }
        RelayPeerRequest::CreateExecutionLease { .. } => "create_execution_lease",
        RelayPeerRequest::DestroyExecutionLease { .. } => "destroy_execution_lease",
        RelayPeerRequest::SpawnLeasedAgent { .. } => "spawn_leased_agent",
        RelayPeerRequest::DestroyLeasedAgent { .. } => "destroy_leased_agent",
        RelayPeerRequest::UpdateLeasedAgentConfig { .. } => "update_leased_agent_config",
        RelayPeerRequest::UpdateLeasedAgentProfile { .. } => "update_leased_agent_profile",
        RelayPeerRequest::UpdateLeasedAgentMetaMode { .. } => "update_leased_agent_meta_mode",
        RelayPeerRequest::SetLeasedAgentDeadline { .. } => "set_leased_agent_deadline",
        RelayPeerRequest::UpdateLeasedAgentRemoteExtensionManifest { .. } => {
            "update_leased_agent_remote_extension_manifest"
        }
        RelayPeerRequest::LaunchLeasedNativeProviderRun { .. } => {
            "launch_leased_native_provider_run"
        }
        RelayPeerRequest::SendLeasedNativeProviderInput { .. } => {
            "send_leased_native_provider_input"
        }
        RelayPeerRequest::ResizeLeasedProviderTerminal { .. } => "resize_leased_provider_terminal",
        RelayPeerRequest::RoomTerminal { .. } => "room_terminal",
        RelayPeerRequest::SubmitLeasedPrompt { .. } => "submit_leased_prompt",
        RelayPeerRequest::UpdateLeasedSudo { .. } => "update_leased_sudo",
        RelayPeerRequest::GetLeasedPromptReceipt { .. } => "get_leased_prompt_receipt",
        RelayPeerRequest::ReconcileLeasedPromptSteerReceipt { .. } => {
            "reconcile_leased_prompt_steer_receipt"
        }
        RelayPeerRequest::SteerLeasedPrompt { .. } => "steer_leased_prompt",
        RelayPeerRequest::DrainLeasedRuntimeProjection { .. } => "drain_leased_runtime_projection",
        RelayPeerRequest::CompleteLeasedPrompt { .. } => "complete_leased_prompt",
        RelayPeerRequest::ObserveLeasedGitAfter { .. } => "observe_leased_git_after",
        RelayPeerRequest::CancelLeasedPrompt { .. } => "cancel_leased_prompt",
        RelayPeerRequest::LeasedWatchedProcess { .. } => "leased_watched_process",
        RelayPeerRequest::ForwardWorkflowRuntimeTool { .. } => "forward_workflow_runtime_tool",
        RelayPeerRequest::ForwardWorkspaceLiveSyncRuntimeTool { .. } => {
            "forward_workspace_live_sync_runtime_tool"
        }
        RelayPeerRequest::FinalizeWorkspaceLiveSyncRuntimeTool { .. } => {
            "finalize_workspace_live_sync_runtime_tool"
        }
        RelayPeerRequest::ForwardCapabilityRuntimeTool { .. } => "forward_capability_runtime_tool",
        RelayPeerRequest::ForwardMetaRuntimeTool { .. } => "forward_meta_runtime_tool",
        RelayPeerRequest::SetLeasedComputerGrant { .. } => "set_leased_computer_grant",
        RelayPeerRequest::CheckLeasedComputerGrant { .. } => "check_leased_computer_grant",
        RelayPeerRequest::ForwardRoomBrowserRuntimeTool { .. } => {
            "forward_room_browser_runtime_tool"
        }
        RelayPeerRequest::ForwardTerminalRuntimeTool { .. } => "forward_terminal_runtime_tool",
        RelayPeerRequest::InvokeHomeExtensionTool { .. } => "invoke_home_extension_tool",
        RelayPeerRequest::InvokeHomeMcpProxy { .. } => "invoke_home_mcp_proxy",
        RelayPeerRequest::CancelHomeExtensionInvocation { .. } => {
            "cancel_home_extension_invocation"
        }
        RelayPeerRequest::InvokeHomeCredentialTool { .. } => "invoke_home_credential_tool",
        RelayPeerRequest::ResolveHomeCredentialSecret { .. } => "resolve_home_credential_secret",
        RelayPeerRequest::ApplyWorkspaceLiveSyncChange { .. } => "apply_workspace_live_sync_change",
        RelayPeerRequest::ForwardNativeTurnInteraction { .. } => "forward_native_turn_interaction",
        RelayPeerRequest::UpdateNativeInteraction { .. } => "update_native_interaction",
        RelayPeerRequest::RunProjectEnvironmentSetup { .. } => "run_project_environment_setup",
        RelayPeerRequest::GetProjectEnvironmentOperation { .. } => {
            "get_project_environment_operation"
        }
        RelayPeerRequest::CancelProjectEnvironmentOperation { .. } => {
            "cancel_project_environment_operation"
        }
        RelayPeerRequest::ReadProjectEnvironment { .. } => "read_project_environment",
        RelayPeerRequest::PullProjectEnvironmentSecret { .. } => "pull_project_environment_secret",
        RelayPeerRequest::PullProjectEnvironmentFile { .. } => "pull_project_environment_file",
        RelayPeerRequest::PullProjectEnvironmentRegistration { .. } => {
            "pull_project_environment_registration"
        }
        RelayPeerRequest::EnsureRemoteSkillPackages { .. } => "ensure_remote_skill_packages",
        RelayPeerRequest::RequestProviderAccountCopy { .. } => "request_provider_account_copy",
        RelayPeerRequest::InstallProviderAccountCopy { .. } => "install_provider_account_copy",
        RelayPeerRequest::EnsureRemoteProviderAccount { .. } => "ensure_remote_provider_account",
        RelayPeerRequest::CheckRemoteMcpAvailability { .. } => "check_remote_mcp_availability",
        RelayPeerRequest::ArmManagedContextImport { .. } => "arm_managed_context_import",
        RelayPeerRequest::BeginManagedContextImport { .. } => "begin_managed_context_import",
        RelayPeerRequest::UploadManagedContextChunk { .. } => "upload_managed_context_chunk",
        RelayPeerRequest::FinalizeManagedContextImport { .. } => "finalize_managed_context_import",
        RelayPeerRequest::GetManagedContextImportStatus { .. } => {
            "get_managed_context_import_status"
        }
    }
}

fn browser_operation(command: &RoomBrowserControllerCommand) -> &'static str {
    match command {
        RoomBrowserControllerCommand::NoteObservation { .. } => "note_observation",
        RoomBrowserControllerCommand::Acquire => "acquire",
        RoomBrowserControllerCommand::Reconcile { .. } => "reconcile",
        RoomBrowserControllerCommand::Artifact { .. } => "artifact",
        RoomBrowserControllerCommand::Snapshot { .. } => "snapshot",
        RoomBrowserControllerCommand::Tab { .. } => "tab",
        RoomBrowserControllerCommand::History { .. } => "history",
        RoomBrowserControllerCommand::Navigate { .. } => "navigate",
        RoomBrowserControllerCommand::Wait { .. } => "wait",
        RoomBrowserControllerCommand::Dialog { .. } => "dialog",
        RoomBrowserControllerCommand::RecoverLifecycle { .. } => "recover_lifecycle",
        RoomBrowserControllerCommand::ConfigureDownloads { .. } => "configure_downloads",
        RoomBrowserControllerCommand::RecoverDownloadConfiguration { .. } => {
            "recover_download_configuration"
        }
        RoomBrowserControllerCommand::CancelDownload { .. } => "cancel_download",
        RoomBrowserControllerCommand::Upload { .. } => "upload",
        RoomBrowserControllerCommand::RecoverUpload { .. } => "recover_upload",
        RoomBrowserControllerCommand::Permission { .. } => "permission",
        RoomBrowserControllerCommand::RecoverPermission { .. } => "recover_permission",
        RoomBrowserControllerCommand::AppView { .. } => "app_view",
        RoomBrowserControllerCommand::PollEvents { .. } => "poll_events",
        RoomBrowserControllerCommand::Action { .. } => "action",
        RoomBrowserControllerCommand::RecoverAction { .. } => "recover_action",
        RoomBrowserControllerCommand::CancelAction { .. } => "cancel_action",
        RoomBrowserControllerCommand::ComputerInput { .. } => "computer_input",
        RoomBrowserControllerCommand::ComputerSecretTarget => "computer_secret_target",
        RoomBrowserControllerCommand::ClearSecretObservation { .. } => "clear_secret_observation",
        RoomBrowserControllerCommand::ComputerClipboardRead { .. } => "computer_clipboard_read",
        RoomBrowserControllerCommand::ImportCookies { .. } => "import_cookies",
        RoomBrowserControllerCommand::CancelCookieImport { .. } => "cancel_cookie_import",
        RoomBrowserControllerCommand::RecoverCookieImport { .. } => "recover_cookie_import",
        RoomBrowserControllerCommand::Release => "release",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_peer_request_trace_fields_name_browser_operation_without_payloads() {
        let trace = RelayPeerRequestTrace::new(
            "request-browser",
            &RelayPeerRequest::RoomBrowserController {
                session_id: "private-session".into(),
                slice_id: "private-slice".into(),
                command: RoomBrowserControllerCommand::Snapshot {
                    target_id: "private-tab".into(),
                    document_id: "private-document".into(),
                },
            },
            &ClientTarget {
                daemon_id: Some("slice:worker".into()),
                daemon_alias: None,
            },
            "persistent",
            15_000,
        );
        let fields = trace.fields("timeout", Some("timed out"), None);
        assert_eq!(fields["request_kind"], "room_browser_controller");
        assert_eq!(fields["request_operation"], "snapshot");
        assert_eq!(fields["target_daemon_id"], "slice:worker");
        assert_eq!(fields["waiting_for"], "authenticated relay peer response");
        assert_eq!(fields["timeout_ms"], 15_000);
        assert!(fields["relay_rtt_ms"].as_u64().is_some());
        assert!(!fields.to_string().contains("private-"));
    }
}
