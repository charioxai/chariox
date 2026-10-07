//! MP-08 / MP-10 / MP-11: one response boundary for local grants and sudo MCP.
//! Secret-bearing replies are classified by response type, never request name.
//! The exhaustive match forces review when a response variant is added.
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
        if public_error_message(message))
    {
        return error;
    }
    denied()
}

pub(crate) fn public_error_message(message: &str) -> bool {
    matches!(
        message,
        "grant revoked or expired"
            | "sudo request refused"
            | "sudo request revoked"
            | "sudo request cancelled"
            | "sudo request expired; answer the popup in a Chariox terminal"
            | "queued sudo was revoked"
            | "sudo authorization revoked before dispatch"
            | "Unix peer has no live authority for this request"
    )
}

fn denied() -> DaemonError {
    crate::runtime::kernel_access::error(
        "external response withheld: operation failed or reply may contain credentials; use the host terminal",
    )
}

// MP-11: one inventory drives typed router/MCP and raw cached socket replies.
// Added response variants make this typed match non-exhaustive; unknown wire
// variants fail closed at the final connection boundary.
#[derive(Clone, Copy)]
enum ResponsePolicy {
    Public,
    Credential,
    Withhold,
}
macro_rules! response_policies {
    ($($policy:ident => [$($variant:ident),* $(,)?]),* $(,)?) => {
        fn response_policy(response: &LocalDaemonResponse) -> ResponsePolicy {
            match response { $($(LocalDaemonResponse::$variant { .. } => ResponsePolicy::$policy,)*)* }
        }
        fn serialized_response_policy(name: &str) -> Result<ResponsePolicy, DaemonError> {
            match name { $($(stringify!($variant) => Ok(ResponsePolicy::$policy),)*)* _ => Err(denied()) }
        }
    };
}
response_policies! {
    Credential => [
        CredentialRegistered,
        CredentialUpserted,
        CredentialRemoved,
        Credential,
        CredentialsListed,
    ],
    Withhold => [
        SessionInviteCreated,
        DebugBundleExported,
        McpServerInstalled,
        McpServerUpdated,
        McpServersImported,
        ProviderCapabilitiesImported,
        McpServer,
        McpServersListed,
        ConnectorRegistered,
        ConnectorUpserted,
        ConnectorAdapterRegistered,
        ConnectorAdapterRemoved,
        ConnectorAdapter,
        ConnectorAdaptersListed,
        ConnectorRemoved,
        Connector,
        ConnectorsListed,
        CloudRelayStatus,
        CloudRelayLoginStarted,
        CloudRelayLoginPolled,
        CloudRelayClientPaired,
        CloudRelayMachinePaired,
        CloudRelayConnected,
        CloudRelayClientTokenIssued,
        KernelClientConnectionResolved,
        CloudSessionInviteCreated,
        CloudSessionInviteShown,
        CloudSessionInviteAccepted,
        SliceProviderLoginStarted,
        PairingInviteCreated,
        PairingInviteJoined,
        TerminalPairingLinkCreated,
        TerminalPairingLinkJoined,
        ProviderLoginStarted,
        ProviderLoginStatus,
        ProviderLoginCancelled,
        ProviderLogoutStarted,
        CredentialEnrollmentInteractionResolved,
        EventGeneratorAuthorizationStarted,
        EventConnectionAuthorizationStarted,
        EventConnectionAuthorizationObserved,
    ],
    Public => [
        CloudRelayLoggedOut,
        KernelSudoRequested,
        KernelAccessGranted,
        KernelAccessGrantsListed,
        KernelAccessRevoked,
        RoomBrowserArtifact,
        AppPublisherEnrollmentStatus,
        AppInstallOperationStatus,
        AppPackageUploadStatus,
        AppInstallationsListed,
        AppInstallation,
        AppInstallationJournal,
        AppDataSnapshotRestored,
        AppRequestFailed,
        AppWorker,
        AppAutomations,
        AppAutomation,
        AppViewOpened,
        AppViewPanelSet,
        AppLogs,
        AppInboxRoutes,
        AppConnections,
        AppSet,
        DeploymentAppsPreview,
        DeploymentAppsConsent,
        AppFileGranted,
        AppFileExport,
        AppFileGrantsRevoked,
        AppHostActionAccepted,
        AppInboxOccurrenceAccepted,
        SessionCreated,
        SessionAttached,
        SessionDetached,
        SessionMembersListed,
        SessionInviteJoined,
        SessionInviteRevoked,
        WorkspaceLinkCreated,
        WorkspaceLinksListed,
        WorkspaceLinkShown,
        WorkspaceLinkAttached,
        WorkspaceLinkDetached,
        WorkspaceLiveSyncStatus,
        WorkspaceLiveSyncModeUpdated,
        ProviderRunLaunched,
        NativeProviderInteractionResolved,
        ProviderRunLaunchAccepted,
        ProviderRunsLaunchAccepted,
        ProjectsListed,
        ProjectRenamed,
        ProjectWorkspacesUpdated,
        ProjectArchived,
        ProjectDeleted,
        ProjectRestored,
        ProjectEnvironmentAdjustmentStarted,
        ProjectEnvironmentManifest,
        ProjectEnvironmentSetupStarted,
        ProjectEnvironmentSetupStatus,
        ProjectEnvironmentSetupCancelled,
        ProjectEnvironmentSetupRetried,
        SessionsListed,
        SessionResolved,
        SessionState,
        RoomEnvironmentState,
        BrowserImportConsent,
        RoomEnvironmentSlice,
        RoomEnvironmentResourceInventory,
        RoomEnvironmentTabAccessibility,
        RoomEnvironmentScreenshotCaptured,
        RoomEnvironmentScreenshotChunk,
        RoomEnvironmentEvents,
        RoomEnvironmentActionHistoryListed,
        RoomEnvironmentUpdated,
        RoomEnvironmentTakeoverUpdated,
        RoomEnvironmentInputReleased,
        RoomEnvironmentActionSubmitted,
        RoomEnvironmentClipboardRead,
        RoomEnvironmentActionCancellationUpdated,
        MetaagentTaskUpdated,
        MetaagentEventsListed,
        MetaagentCommandsSearched,
        MetaagentTurnOverview,
        MetaagentTurnBlob,
        MetaagentEventRead,
        MetaagentEventsAcked,
        TerminalCommandCatalog,
        DaemonHealth,
        KernelResourceTelemetry,
        ProviderRun,
        ProviderRunSelectionUpdated,
        ProviderCatalog,
        ProviderCommandCatalogs,
        McpServerUninstalled,
        EnvironmentRegistered,
        EnvironmentRemoved,
        Environment,
        EnvironmentsListed,
        ScriptValidated,
        ScriptRegistered,
        ScriptRemoved,
        Script,
        ScriptsListed,
        ConnectorTested,
        Skill,
        SkillInstalled,
        SkillUpserted,
        SkillUpdated,
        SkillUninstalled,
        SkillsImported,
        SkillsListed,
        RelayStatus,
        RelayConfigured,
        CloudSessionInviteRevoked,
        CloudSessionMembersListed,
        CloudCollaboratorsListed,
        UserConfig,
        UserConfigSchema,
        UserConfigUpdated,
        CredentialSecretStored,
        CredentialSecretDeleted,
        CredentialVaultStatus,
        CredentialVaultLocked,
        CredentialVaultManaged,
        ManagedEnvironmentCatalog,
        DisposableWorker,
        DisposableWorkerContextTransferPrepared,
        ManagedEnvironmentKeptRunning,
        ManagedEnvironment,
        ManagedEnvironmentReimagePreflight,
        ManagedEnvironmentReimageReceipt,
        ManagedEnvironmentContextTransferPrepared,
        ManagedEnvironmentCreated,
        ManagedEnvironmentLifecycleRequested,
        ManagedEnvironmentReimageRequested,
        ManagedEnvironmentReleaseUpdateRequested,
        ManagedEnvironmentReleaseUpdateRead,
        ManagedEnvironmentPreReimageObserved,
        ManagedContextTransferStarted,
        ManagedContextTransferStatus,
        ManagedContextLaunchTarget,
        SlicesListed,
        SliceCreated,
        Slice,
        SliceStarted,
        SliceStopped,
        SliceDeleted,
        SliceProviderAuthImported,
        SliceProviderAuthRemoved,
        SliceDisplayEndpoint,
        SliceLogs,
        SliceStateSaved,
        SliceStateStatus,
        SliceStateReset,
        SliceBackupCreated,
        SliceBackupRestored,
        RemoteMachinesListed,
        RemoteMachineKernelsListed,
        FreshRemoteMachineKernelsObserved,
        WaitingRoomInventory,
        WaitingRoomPublicSnapshot,
        ExternalProviderSessionsListed,
        ExternalProviderSessionsRefreshed,
        ExternalProviderSessionImported,
        ExternalProviderAgentImported,
        WorkspaceDirectoriesSearched,
        WorkspaceDirectoryCreated,
        WorkspaceWorktreesListed,
        WorkspaceWorktreeCreated,
        WorkspaceWorktreeDeleted,
        WorkspacePullRequestCreated,
        WorkspaceGitOverview,
        WorkspaceFilesListed,
        WorkspaceFileContent,
        WorkspaceFileContentNotModified,
        AgentUtilityCompleted,
        WorkspaceCommitMessageGenerated,
        WorkspaceGitActionCompleted,
        RemoteMachineApproved,
        RemoteMachineForgotten,
        RemoteMachineRenamed,
        TerminalsListed,
        PairedClientsListed,
        PairedClientRecorded,
        PairedClientRevoked,
        ProviderAuthStatus,
        ProviderLoginInputSent,
        ProviderLoggedOut,
        ProviderAccountProfilesListed,
        ProviderAccountProfile,
        ProviderAccountProfileRemoved,
        ProviderAccountProfileDataDeleted,
        ProviderAccountCredentialStored,
        ProviderProcessesListed,
        ProviderProcessesTornDown,
        SessionHistoryOutline,
        SessionHistoryBlobContent,
        PromptInputHistory,
        PromptInputHistoryRecorded,
        PromptSettingsListed,
        PromptSetting,
        PromptSettingPreview,
        PromptSettingsReset,
        AgentPromptScheduleCreated,
        AgentPromptScheduleCancelled,
        RecallEvents,
        SemanticRecallEvents,
        RuntimeNotices,
        KernelAccessDecisionResponded,
        InteractionResponded,
        DeploymentCredentialEnrollmentArmed,
        PromptSubmitted,
        PromptsSubmitted,
        PromptCompleted,
        PromptCancelled,
        QueuedPromptSteered,
        QueuedPromptCancelled,
        QueuedPromptUpdated,
        SessionConfigUpdated,
        AgentConfigUpdated,
        TerminalResized,
        TerminalInputSent,
        TerminalOutput,
        ShellCommandCompleted,
        DirectoryTreeRead,
        FileRead,
        FileEdited,
        GitInspected,
        ScreenshotCaptured,
        FileTransferred,
        SessionEnded,
        SessionDeleted,
        KernelDeleted,
        SessionAliased,
        AgentAliased,
        AgentProfileUpdated,
        AgentSpawned,
        AgentsSpawned,
        TurnUndone,
        AgentForked,
        AgentMovedToRemote,
        AgentMovedToLocal,
        RemoteExtensionManifestSynced,
        HomeExtensionAuditListed,
        SliceAuditListed,
        AgentDestroyed,
        AgentFocused,
        AgentOutputSeenAcknowledged,
        AgentFocusCycled,
        AgentExtensionGranted,
        AgentExtensionRevoked,
        AgentsListed,
        WorkflowCreated,
        AgentWorkflowCreated,
        WorkflowCodeValidated,
        WorkflowCodeApplied,
        WorkflowCodeRun,
        WorkflowRegistryListed,
        WorkflowRegistryEntry,
        WorkflowRegistryEntryAdded,
        WorkflowRegistryEntryDeleted,
        WorkflowRegistryEntryLoaded,
        WorkflowRegistryEntryRun,
        WorkflowCodeArtifactCreated,
        WorkflowCodeArtifactUpdated,
        WorkflowCodeSourceBound,
        WorkflowCodeRebuildPreview,
        WorkflowCodeSourceRebuilt,
        WorkflowCodeSourceUpdatePreview,
        WorkflowCodeSourceUpdated,
        WorkflowCodeArtifact,
        WorkflowCodeArtifactsListed,
        WorkflowCodeArtifactDeleted,
        WorkflowCodeArtifactExported,
        WorkflowCodeArtifactImported,
        WorkflowCodePackageExported,
        WorkflowCodePackageImported,
        WorkflowCodeSourceExported,
        WorkflowAliased,
        WorkflowsListed,
        WorkflowResolved,
        WorkflowPublicationCreated,
        WorkflowPublicationsListed,
        EventGeneratorCatalogPage,
        EventGeneratorDetail,
        EventGeneratorEventsPage,
        EventGeneratorResourcesPage,
        EventConnectionsPage,
        EventConnection,
        EventConnectionResourcesPage,
        EventConnectionTested,
        EventConnectionDependencies,
        EventConnectionRemoved,
        EventDeliveryStatus,
        WorkflowPublication,
        WorkflowPublicationPackageExported,
        WorkflowPublicationDisabled,
        WorkflowPublicationRuntimeControlled,
        WorkflowPublicationDeploymentBound,
        WorkflowPublicationEndpointRegistered,
        WorkflowPublicationMaterialized,
        WorkflowPublicationRuntimeActivated,
        WorkflowDesignOpAccepted,
        WorkflowDesignOpRejected,
        WorkflowEndpointCreated,
        WorkflowEndpointAliased,
        WorkflowEndpointBound,
        WorkflowNodeAdded,
        WorkflowNodeRemoved,
        WorkflowNodeInstructionsUpdated,
        WorkflowNodeCanCompleteRunUpdated,
        WorkflowNodeCanEmitIntermediateOutputUpdated,
        WorkflowNodeWaitForAllInputsUpdated,
        WorkflowNodeIntermediateOutputSchemaUpdated,
        WorkflowNodeMaxTurnsUpdated,
        WorkflowEdgeAdded,
        WorkflowEdgeRemoved,
        WorkflowCanvasLayoutUpdated,
        WorkflowRunInvoked,
        WorkflowPromptEnqueued,
        WorkflowRunsListed,
        WorkflowRun,
        WorkflowRunCancelled,
        WorkflowRunPaused,
        WorkflowRunResumed,
        WorkflowWatchdogCreated,
        WorkflowWatchdogsListed,
        WorkflowWatchdogUpdated,
        WorkflowWatchdogRemoved,
        WorkflowScheduleCreated,
        WorkflowSchedulesListed,
        WorkflowScheduleUpdated,
        WorkflowScheduleRemoved,
        WorkflowSchedulePreviewed,
        WorkflowFlushContextUpdated,
        WorkflowRunOutputSchemaUpdated,
        WorkflowPromptQueuesListed,
        WorkflowPromptQueueCreated,
        WorkflowPromptQueueUpdated,
        WorkflowPromptQueueRemoved,
        QueuedWorkflowPromptsListed,
        QueuedWorkflowPromptUpdated,
        QueuedWorkflowPromptRemoved,
        WorkflowPromptQueueCleared,
        WorkflowHandoffValidated,
        WorkflowTurnAcknowledged,
    ],
}
fn project(response: LocalDaemonResponse) -> Result<LocalDaemonResponse, DaemonError> {
    let policy = response_policy(&response);
    let mut value = serde_json::to_value(response).map_err(|_| denied())?;
    project_response_body(&mut value, policy)?;
    serde_json::from_value(value).map_err(|_| denied())
}
pub(crate) fn project_response_value(value: &mut serde_json::Value) -> Result<(), DaemonError> {
    // MP-11: serde encodes unit variants as strings. Deserialize only this
    // closed shape and reuse the exhaustive typed policy rather than allowing
    // arbitrary strings to bypass classification.
    if value.is_string() {
        let response: LocalDaemonResponse =
            serde_json::from_value(value.clone()).map_err(|_| denied())?;
        return project_response_body(value, response_policy(&response));
    }
    let fields = value
        .as_object()
        .filter(|f| f.len() == 1)
        .ok_or_else(denied)?;
    let name = fields.keys().next().ok_or_else(denied)?;
    let policy = serialized_response_policy(name)?;
    project_response_body(value, policy)
}
fn project_response_body(
    value: &mut serde_json::Value,
    policy: ResponsePolicy,
) -> Result<(), DaemonError> {
    match policy {
        ResponsePolicy::Withhold => return Err(denied()),
        ResponsePolicy::Credential | ResponsePolicy::Public => {}
    }
    redact_secret_values(value);
    Ok(())
}
// Exact credential shapes only. Free-form terminal/history text and credential
// handles/references remain intact; raw configuration variants are withheld.
pub(crate) fn redact_secret_values(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(fields) => {
            if let Some(binding) = fields
                .get_mut("remote_execution")
                .and_then(|v| v.as_object_mut())
            {
                binding.remove("relay_token");
            }
            if let Some(injection) = fields.get_mut("injection").and_then(|v| v.as_object_mut()) {
                if injection.get("kind").and_then(|v| v.as_str()) == Some("header") {
                    injection.insert("value".into(), "[REDACTED]".into());
                }
            }
            for value in fields.values_mut() {
                redact_secret_values(value);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                redact_secret_values(value);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests;
