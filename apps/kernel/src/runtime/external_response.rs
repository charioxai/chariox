//! MP-08 / MP-10 / MP-11: one response boundary for local grants and sudo MCP.
//! Secret-bearing replies are classified by response type, never request name.
//! The exhaustive match forces review when a response variant is added.
use crate::config::{UserCredentialConfig, UserCredentialInjectionConfig};
use crate::error::DaemonError;
use crate::local::LocalDaemonResponse;
use crate::runtime::command::KernelCommand;

pub(crate) fn finish_response(
    command: &KernelCommand,
    result: Result<LocalDaemonResponse, DaemonError>,
) -> Result<LocalDaemonResponse, DaemonError> {
    if command.external_grant_id().is_none() {
        return result;
    }
    // Parser/provider errors can echo stored values. Admission errors remain
    // specific because they are returned before executor dispatch.
    let response = result.map_err(protect_error)?;
    project(response)
}

fn protect_error(error: DaemonError) -> DaemonError {
    // These exact lifecycle messages are kernel constants, never provider or
    // parser output. Preserve refusal/revocation so callers can react correctly.
    if matches!(&error, DaemonError::LocalTransport { operation: "kernel access", message }
        if matches!(message.as_str(),
            "grant revoked or expired"
            | "sudo request refused"
            | "sudo request revoked"
            | "sudo request cancelled"
            | "sudo request expired; answer the popup in a Chariox terminal"
            | "queued sudo was revoked"
            | "sudo authorization revoked before dispatch"))
    {
        return error;
    }
    denied()
}

fn denied() -> DaemonError {
    crate::runtime::kernel_access::error(
        "external response withheld: operation failed or reply may contain credentials; use the host terminal",
    )
}

fn redact_credential(credential: &mut UserCredentialConfig) {
    match &mut credential.injection {
        UserCredentialInjectionConfig::Header { value, .. } => *value = "[REDACTED]".into(),
        UserCredentialInjectionConfig::Query { .. }
        | UserCredentialInjectionConfig::Basic { .. }
        | UserCredentialInjectionConfig::Hmac { .. }
        | UserCredentialInjectionConfig::Pty
        | UserCredentialInjectionConfig::Browser
        | UserCredentialInjectionConfig::Computer
        | UserCredentialInjectionConfig::Provider => {}
    }
}

fn project(mut response: LocalDaemonResponse) -> Result<LocalDaemonResponse, DaemonError> {
    use LocalDaemonResponse as R;
    match &mut response {
        // Reads AND mutation results share the same stored credential type.
        R::CredentialRegistered { credential, .. }
        | R::CredentialUpserted { credential, .. }
        | R::CredentialRemoved { credential, .. }
        | R::Credential { credential } => redact_credential(credential),
        R::CredentialsListed { credentials } => {
            for credential in credentials {
                redact_credential(credential);
            }
        }
        // Literal MCP env/headers/args/URLs, connector configuration, login
        // output/codes and admission tokens are opaque credential surfaces.
        R::SessionInviteCreated { .. }
        | R::DebugBundleExported { .. }
        | R::McpServerInstalled { .. }
        | R::McpServerUpdated { .. }
        | R::McpServersImported { .. }
        | R::ProviderCapabilitiesImported { .. }
        | R::McpServer { .. }
        | R::McpServersListed { .. }
        | R::ConnectorRegistered { .. }
        | R::ConnectorUpserted { .. }
        | R::ConnectorAdapterRegistered { .. }
        | R::ConnectorAdapterRemoved { .. }
        | R::ConnectorAdapter { .. }
        | R::ConnectorAdaptersListed { .. }
        | R::ConnectorRemoved { .. }
        | R::Connector { .. }
        | R::ConnectorsListed { .. }
        | R::CloudRelayStatus { .. }
        | R::CloudRelayLoginStarted { .. }
        | R::CloudRelayLoginPolled { .. }
        | R::CloudRelayClientPaired { .. }
        | R::CloudRelayMachinePaired { .. }
        | R::CloudRelayConnected { .. }
        | R::CloudRelayClientTokenIssued { .. }
        | R::KernelClientConnectionResolved { .. }
        | R::CloudSessionInviteCreated { .. }
        | R::CloudSessionInviteShown { .. }
        | R::CloudSessionInviteAccepted { .. }
        | R::SliceProviderLoginStarted { .. }
        | R::PairingInviteCreated { .. }
        | R::PairingInviteJoined { .. }
        | R::TerminalPairingLinkCreated { .. }
        | R::TerminalPairingLinkJoined { .. }
        | R::ProviderLoginStarted { .. }
        | R::ProviderLoginStatus { .. }
        | R::ProviderLoginCancelled { .. }
        | R::ProviderLogoutStarted { .. }
        | R::CredentialEnrollmentInteractionResolved { .. }
        | R::EventGeneratorAuthorizationStarted { .. }
        | R::EventConnectionAuthorizationStarted { .. }
        | R::EventConnectionAuthorizationObserved { .. } => return Err(denied()),
        // Public runtime/provider/account summaries, Vault status and key
        // references contain no stored values. Session/history/workspace/App
        // artifacts retain normal user authority; this is not a file sandbox.
        R::KernelSudoRequested { .. }
        | R::KernelAccessGranted { .. }
        | R::KernelAccessGrantsListed { .. }
        | R::KernelAccessRevoked { .. }
        | R::RoomBrowserArtifact { .. }
        | R::AppPublisherEnrollmentStatus { .. }
        | R::AppInstallOperationStatus { .. }
        | R::AppPackageUploadStatus { .. }
        | R::AppInstallationsListed { .. }
        | R::AppInstallation { .. }
        | R::AppInstallationJournal { .. }
        | R::AppDataSnapshotRestored { .. }
        | R::AppRequestFailed { .. }
        | R::AppWorker { .. }
        | R::AppAutomations { .. }
        | R::AppAutomation { .. }
        | R::AppViewOpened { .. }
        | R::AppViewPanelSet { .. }
        | R::AppLogs { .. }
        | R::AppInboxRoutes { .. }
        | R::AppConnections { .. }
        | R::AppSet { .. }
        | R::DeploymentAppsPreview { .. }
        | R::DeploymentAppsConsent { .. }
        | R::AppFileGranted { .. }
        | R::AppFileExport { .. }
        | R::AppFileGrantsRevoked { .. }
        | R::AppHostActionAccepted { .. }
        | R::AppInboxOccurrenceAccepted { .. }
        | R::SessionCreated { .. }
        | R::SessionAttached { .. }
        | R::SessionDetached { .. }
        | R::SessionMembersListed { .. }
        | R::SessionInviteJoined { .. }
        | R::SessionInviteRevoked { .. }
        | R::WorkspaceLinkCreated { .. }
        | R::WorkspaceLinksListed { .. }
        | R::WorkspaceLinkShown { .. }
        | R::WorkspaceLinkAttached { .. }
        | R::WorkspaceLinkDetached { .. }
        | R::WorkspaceLiveSyncStatus { .. }
        | R::WorkspaceLiveSyncModeUpdated { .. }
        | R::ProviderRunLaunched { .. }
        | R::NativeProviderInteractionResolved { .. }
        | R::ProviderRunLaunchAccepted { .. }
        | R::ProviderRunsLaunchAccepted { .. }
        | R::ProjectsListed { .. }
        | R::ProjectRenamed { .. }
        | R::ProjectWorkspacesUpdated { .. }
        | R::ProjectArchived { .. }
        | R::ProjectDeleted { .. }
        | R::ProjectRestored { .. }
        | R::ProjectEnvironmentAdjustmentStarted { .. }
        | R::ProjectEnvironmentManifest { .. }
        | R::ProjectEnvironmentSetupStarted { .. }
        | R::ProjectEnvironmentSetupStatus { .. }
        | R::ProjectEnvironmentSetupCancelled { .. }
        | R::ProjectEnvironmentSetupRetried { .. }
        | R::SessionsListed { .. }
        | R::SessionResolved { .. }
        | R::SessionState { .. }
        | R::RoomEnvironmentState { .. }
        | R::BrowserImportConsent { .. }
        | R::RoomEnvironmentSlice { .. }
        | R::RoomEnvironmentResourceInventory { .. }
        | R::RoomEnvironmentTabAccessibility { .. }
        | R::RoomEnvironmentScreenshotCaptured { .. }
        | R::RoomEnvironmentScreenshotChunk { .. }
        | R::RoomEnvironmentEvents { .. }
        | R::RoomEnvironmentActionHistoryListed { .. }
        | R::RoomEnvironmentUpdated { .. }
        | R::RoomEnvironmentTakeoverUpdated { .. }
        | R::RoomEnvironmentInputReleased { .. }
        | R::RoomEnvironmentActionSubmitted { .. }
        | R::RoomEnvironmentClipboardRead { .. }
        | R::RoomEnvironmentActionCancellationUpdated { .. }
        | R::MetaagentTaskUpdated { .. }
        | R::MetaagentEventsListed { .. }
        | R::MetaagentCommandsSearched { .. }
        | R::MetaagentTurnOverview { .. }
        | R::MetaagentTurnBlob { .. }
        | R::MetaagentEventRead { .. }
        | R::MetaagentEventsAcked { .. }
        | R::TerminalCommandCatalog { .. }
        | R::DaemonHealth { .. }
        | R::KernelResourceTelemetry { .. }
        | R::ProviderRun { .. }
        | R::ProviderRunSelectionUpdated { .. }
        | R::ProviderCatalog { .. }
        | R::ProviderCommandCatalogs { .. }
        | R::McpServerUninstalled { .. }
        | R::EnvironmentRegistered { .. }
        | R::EnvironmentRemoved { .. }
        | R::Environment { .. }
        | R::EnvironmentsListed { .. }
        | R::ScriptValidated { .. }
        | R::ScriptRegistered { .. }
        | R::ScriptRemoved { .. }
        | R::Script { .. }
        | R::ScriptsListed { .. }
        | R::ConnectorTested { .. }
        | R::Skill { .. }
        | R::SkillInstalled { .. }
        | R::SkillUpserted { .. }
        | R::SkillUpdated { .. }
        | R::SkillUninstalled { .. }
        | R::SkillsImported { .. }
        | R::SkillsListed { .. }
        | R::RelayStatus { .. }
        | R::RelayConfigured { .. }
        | R::CloudRelayLoggedOut
        | R::CloudSessionInviteRevoked { .. }
        | R::CloudSessionMembersListed { .. }
        | R::CloudCollaboratorsListed { .. }
        | R::UserConfig { .. }
        | R::UserConfigSchema { .. }
        | R::UserConfigUpdated { .. }
        | R::CredentialSecretStored { .. }
        | R::CredentialSecretDeleted { .. }
        | R::CredentialVaultStatus { .. }
        | R::CredentialVaultLocked { .. }
        | R::CredentialVaultManaged { .. }
        | R::ManagedEnvironmentCatalog { .. }
        | R::DisposableWorker { .. }
        | R::DisposableWorkerContextTransferPrepared { .. }
        | R::ManagedEnvironmentKeptRunning { .. }
        | R::ManagedEnvironment { .. }
        | R::ManagedEnvironmentReimagePreflight { .. }
        | R::ManagedEnvironmentReimageReceipt { .. }
        | R::ManagedEnvironmentContextTransferPrepared { .. }
        | R::ManagedEnvironmentCreated { .. }
        | R::ManagedEnvironmentLifecycleRequested { .. }
        | R::ManagedEnvironmentReimageRequested { .. }
        | R::ManagedEnvironmentReleaseUpdateRequested { .. }
        | R::ManagedEnvironmentReleaseUpdateRead { .. }
        | R::ManagedEnvironmentPreReimageObserved { .. }
        | R::ManagedContextTransferStarted { .. }
        | R::ManagedContextTransferStatus { .. }
        | R::ManagedContextLaunchTarget { .. }
        | R::SlicesListed { .. }
        | R::SliceCreated { .. }
        | R::Slice { .. }
        | R::SliceStarted { .. }
        | R::SliceStopped { .. }
        | R::SliceDeleted { .. }
        | R::SliceProviderAuthImported { .. }
        | R::SliceProviderAuthRemoved { .. }
        | R::SliceDisplayEndpoint { .. }
        | R::SliceLogs { .. }
        | R::SliceStateSaved { .. }
        | R::SliceStateStatus { .. }
        | R::SliceStateReset { .. }
        | R::SliceBackupCreated { .. }
        | R::SliceBackupRestored { .. }
        | R::RemoteMachinesListed { .. }
        | R::RemoteMachineKernelsListed { .. }
        | R::FreshRemoteMachineKernelsObserved { .. }
        | R::WaitingRoomInventory { .. }
        | R::WaitingRoomPublicSnapshot { .. }
        | R::ExternalProviderSessionsListed { .. }
        | R::ExternalProviderSessionsRefreshed { .. }
        | R::ExternalProviderSessionImported { .. }
        | R::ExternalProviderAgentImported { .. }
        | R::WorkspaceDirectoriesSearched { .. }
        | R::WorkspaceDirectoryCreated { .. }
        | R::WorkspaceWorktreesListed { .. }
        | R::WorkspaceWorktreeCreated { .. }
        | R::WorkspaceWorktreeDeleted { .. }
        | R::WorkspacePullRequestCreated { .. }
        | R::WorkspaceGitOverview { .. }
        | R::WorkspaceFilesListed { .. }
        | R::WorkspaceFileContent { .. }
        | R::WorkspaceFileContentNotModified { .. }
        | R::AgentUtilityCompleted { .. }
        | R::WorkspaceCommitMessageGenerated { .. }
        | R::WorkspaceGitActionCompleted { .. }
        | R::RemoteMachineApproved { .. }
        | R::RemoteMachineForgotten { .. }
        | R::RemoteMachineRenamed { .. }
        | R::TerminalsListed { .. }
        | R::PairedClientsListed { .. }
        | R::PairedClientRecorded { .. }
        | R::PairedClientRevoked { .. }
        | R::ProviderAuthStatus { .. }
        | R::ProviderLoginInputSent { .. }
        | R::ProviderLoggedOut { .. }
        | R::ProviderAccountProfilesListed { .. }
        | R::ProviderAccountProfile { .. }
        | R::ProviderAccountProfileRemoved { .. }
        | R::ProviderAccountProfileDataDeleted { .. }
        | R::ProviderAccountCredentialStored { .. }
        | R::ProviderProcessesListed { .. }
        | R::ProviderProcessesTornDown { .. }
        | R::SessionHistoryOutline { .. }
        | R::SessionHistoryBlobContent { .. }
        | R::PromptInputHistory { .. }
        | R::PromptInputHistoryRecorded { .. }
        | R::PromptSettingsListed { .. }
        | R::PromptSetting { .. }
        | R::PromptSettingPreview { .. }
        | R::PromptSettingsReset { .. }
        | R::AgentPromptScheduleCreated { .. }
        | R::AgentPromptScheduleCancelled { .. }
        | R::RecallEvents { .. }
        | R::SemanticRecallEvents { .. }
        | R::RuntimeNotices { .. }
        | R::KernelAccessDecisionResponded { .. }
        | R::InteractionResponded { .. }
        | R::DeploymentCredentialEnrollmentArmed { .. }
        | R::PromptSubmitted { .. }
        | R::PromptsSubmitted { .. }
        | R::PromptCompleted { .. }
        | R::PromptCancelled { .. }
        | R::QueuedPromptSteered { .. }
        | R::QueuedPromptCancelled { .. }
        | R::QueuedPromptUpdated { .. }
        | R::SessionConfigUpdated { .. }
        | R::AgentConfigUpdated { .. }
        | R::TerminalResized { .. }
        | R::TerminalInputSent { .. }
        | R::TerminalOutput { .. }
        | R::ShellCommandCompleted { .. }
        | R::DirectoryTreeRead { .. }
        | R::FileRead { .. }
        | R::FileEdited { .. }
        | R::GitInspected { .. }
        | R::ScreenshotCaptured { .. }
        | R::FileTransferred { .. }
        | R::SessionEnded { .. }
        | R::SessionDeleted { .. }
        | R::KernelDeleted { .. }
        | R::SessionAliased { .. }
        | R::AgentAliased { .. }
        | R::AgentProfileUpdated { .. }
        | R::AgentSpawned { .. }
        | R::AgentsSpawned { .. }
        | R::TurnUndone { .. }
        | R::AgentForked { .. }
        | R::AgentMovedToRemote { .. }
        | R::AgentMovedToLocal { .. }
        | R::RemoteExtensionManifestSynced { .. }
        | R::HomeExtensionAuditListed { .. }
        | R::SliceAuditListed { .. }
        | R::AgentDestroyed { .. }
        | R::AgentFocused { .. }
        | R::AgentOutputSeenAcknowledged { .. }
        | R::AgentFocusCycled { .. }
        | R::AgentExtensionGranted { .. }
        | R::AgentExtensionRevoked { .. }
        | R::AgentsListed { .. }
        | R::WorkflowCreated { .. }
        | R::AgentWorkflowCreated { .. }
        | R::WorkflowCodeValidated { .. }
        | R::WorkflowCodeApplied { .. }
        | R::WorkflowCodeRun { .. }
        | R::WorkflowRegistryListed { .. }
        | R::WorkflowRegistryEntry { .. }
        | R::WorkflowRegistryEntryAdded { .. }
        | R::WorkflowRegistryEntryDeleted { .. }
        | R::WorkflowRegistryEntryLoaded { .. }
        | R::WorkflowRegistryEntryRun { .. }
        | R::WorkflowCodeArtifactCreated { .. }
        | R::WorkflowCodeArtifactUpdated { .. }
        | R::WorkflowCodeSourceBound { .. }
        | R::WorkflowCodeRebuildPreview { .. }
        | R::WorkflowCodeSourceRebuilt { .. }
        | R::WorkflowCodeSourceUpdatePreview { .. }
        | R::WorkflowCodeSourceUpdated { .. }
        | R::WorkflowCodeArtifact { .. }
        | R::WorkflowCodeArtifactsListed { .. }
        | R::WorkflowCodeArtifactDeleted { .. }
        | R::WorkflowCodeArtifactExported { .. }
        | R::WorkflowCodeArtifactImported { .. }
        | R::WorkflowCodePackageExported { .. }
        | R::WorkflowCodePackageImported { .. }
        | R::WorkflowCodeSourceExported { .. }
        | R::WorkflowAliased { .. }
        | R::WorkflowsListed { .. }
        | R::WorkflowResolved { .. }
        | R::WorkflowPublicationCreated { .. }
        | R::WorkflowPublicationsListed { .. }
        | R::EventGeneratorCatalogPage { .. }
        | R::EventGeneratorDetail { .. }
        | R::EventGeneratorEventsPage { .. }
        | R::EventGeneratorResourcesPage { .. }
        | R::EventConnectionsPage { .. }
        | R::EventConnection { .. }
        | R::EventConnectionResourcesPage { .. }
        | R::EventConnectionTested { .. }
        | R::EventConnectionDependencies { .. }
        | R::EventConnectionRemoved { .. }
        | R::EventDeliveryStatus { .. }
        | R::WorkflowPublication { .. }
        | R::WorkflowPublicationPackageExported { .. }
        | R::WorkflowPublicationDisabled { .. }
        | R::WorkflowPublicationRuntimeControlled { .. }
        | R::WorkflowPublicationDeploymentBound { .. }
        | R::WorkflowPublicationEndpointRegistered { .. }
        | R::WorkflowPublicationMaterialized { .. }
        | R::WorkflowPublicationRuntimeActivated { .. }
        | R::WorkflowDesignOpAccepted { .. }
        | R::WorkflowDesignOpRejected { .. }
        | R::WorkflowEndpointCreated { .. }
        | R::WorkflowEndpointAliased { .. }
        | R::WorkflowEndpointBound { .. }
        | R::WorkflowNodeAdded { .. }
        | R::WorkflowNodeRemoved { .. }
        | R::WorkflowNodeInstructionsUpdated { .. }
        | R::WorkflowNodeCanCompleteRunUpdated { .. }
        | R::WorkflowNodeCanEmitIntermediateOutputUpdated { .. }
        | R::WorkflowNodeWaitForAllInputsUpdated { .. }
        | R::WorkflowNodeIntermediateOutputSchemaUpdated { .. }
        | R::WorkflowNodeMaxTurnsUpdated { .. }
        | R::WorkflowEdgeAdded { .. }
        | R::WorkflowEdgeRemoved { .. }
        | R::WorkflowCanvasLayoutUpdated { .. }
        | R::WorkflowRunInvoked { .. }
        | R::WorkflowPromptEnqueued { .. }
        | R::WorkflowRunsListed { .. }
        | R::WorkflowRun { .. }
        | R::WorkflowRunCancelled { .. }
        | R::WorkflowRunPaused { .. }
        | R::WorkflowRunResumed { .. }
        | R::WorkflowWatchdogCreated { .. }
        | R::WorkflowWatchdogsListed { .. }
        | R::WorkflowWatchdogUpdated { .. }
        | R::WorkflowWatchdogRemoved { .. }
        | R::WorkflowScheduleCreated { .. }
        | R::WorkflowSchedulesListed { .. }
        | R::WorkflowScheduleUpdated { .. }
        | R::WorkflowScheduleRemoved { .. }
        | R::WorkflowSchedulePreviewed { .. }
        | R::WorkflowFlushContextUpdated { .. }
        | R::WorkflowRunOutputSchemaUpdated { .. }
        | R::WorkflowPromptQueuesListed { .. }
        | R::WorkflowPromptQueueCreated { .. }
        | R::WorkflowPromptQueueUpdated { .. }
        | R::WorkflowPromptQueueRemoved { .. }
        | R::QueuedWorkflowPromptsListed { .. }
        | R::QueuedWorkflowPromptUpdated { .. }
        | R::QueuedWorkflowPromptRemoved { .. }
        | R::WorkflowPromptQueueCleared { .. }
        | R::WorkflowHandoffValidated { .. }
        | R::WorkflowTurnAcknowledged { .. } => {}
    }
    // AgentInstance carries an optional worker relay admission token and is
    // nested in many session/workflow projections. Remove that exact binding
    // field across all reply shapes, including future nesting. Owner replies
    // bypass this projection. No workspace/history text is scanned or changed.
    let mut value = serde_json::to_value(response).map_err(|_| denied())?;
    redact_remote_bindings(&mut value);
    serde_json::from_value(value).map_err(|_| denied())
}

fn redact_remote_bindings(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(fields) => {
            if let Some(binding) = fields
                .get_mut("remote_execution")
                .and_then(|v| v.as_object_mut())
            {
                binding.remove("relay_token");
            }
            for value in fields.values_mut() {
                redact_remote_bindings(value);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                redact_remote_bindings(value);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests;
