import Foundation

public enum LocalDaemonRequest: Encodable, Sendable {
    case listSessions
    case createSession(workspaceID: String, worktreeID: String, alias: String?)
    case deleteSession(sessionRef: String, workspaceID: String?)
    case attachToSession(sessionID: String, clientID: String)
    case detachFromSession(attachmentID: String)
    case getSessionState(sessionID: String)
    case updateSessionConfig(sessionID: String, attachmentID: String, values: [String: String], requiresIdle: Bool)
    case getProviderCatalog
    case getProviderAuthStatus(provider: String)
    case startProviderLogin(provider: String)
    case logoutProvider(provider: String)
    case listMcpServers(workspaceID: String?)
    case listSkills(workspaceID: String?)
    case submitPrompt(sessionID: String, attachmentID: String, targetAgentID: String?, prompt: String)
    case cancelActivePrompt(sessionID: String, attachmentID: String)
    case respondToInteraction(sessionID: String, interactionID: String, choiceID: String)
    case getWorkspaceLiveSyncStatus(sessionID: String)
    case attachWorkspaceLink(sessionID: String, linkRef: String, repoRoot: String?)
    case setWorkspaceLiveSyncMode(sessionID: String, mode: String)
    case setUserConfigValue(path: String, value: String)
    case getSessionHistory(sessionID: String, agentID: String?, roundCount: Int, maxChars: Int)
    case spawnAgent(sessionID: String, alias: String?, provider: String, model: String?, effort: String?, worktreeID: String?)
    case destroyAgent(sessionID: String, agentID: String)
    case aliasAgent(sessionID: String, agentID: String, alias: String)
    case updateAgentProfile(
        sessionID: String,
        agentID: String,
        provider: String?,
        model: String?,
        effort: String?,
        clearEffort: Bool
    )
    case updateAgentConfig(
        sessionID: String,
        agentID: String,
        executionMode: String?,
        clearExecutionMode: Bool,
        permissionLevel: String?,
        clearPermissionLevel: Bool
    )
    case focusAgent(sessionID: String, agentID: String)
    case cycleAgentFocus(sessionID: String)

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: DynamicCodingKey.self)
        switch self {
        case .listSessions:
            try container.encodeNil(forKey: DynamicCodingKey("ListSessions"))
        case let .createSession(workspaceID, worktreeID, alias):
            try container.encode(
                CreateSessionPayload(
                    workspaceID: workspaceID,
                    worktreeID: worktreeID,
                    alias: alias
                ),
                forKey: DynamicCodingKey("CreateSession")
            )
        case let .deleteSession(sessionRef, workspaceID):
            try container.encode(
                DeleteSessionPayload(sessionRef: sessionRef, workspaceID: workspaceID),
                forKey: DynamicCodingKey("DeleteSession")
            )
        case let .attachToSession(sessionID, clientID):
            try container.encode(
                AttachToSessionPayload(sessionID: sessionID, clientID: clientID),
                forKey: DynamicCodingKey("AttachToSession")
            )
        case let .detachFromSession(attachmentID):
            try container.encode(
                DetachFromSessionPayload(attachmentID: attachmentID),
                forKey: DynamicCodingKey("DetachFromSession")
            )
        case let .getSessionState(sessionID):
            try container.encode(
                GetSessionStatePayload(sessionID: sessionID),
                forKey: DynamicCodingKey("GetSessionState")
            )
        case let .updateSessionConfig(sessionID, attachmentID, values, requiresIdle):
            try container.encode(
                UpdateSessionConfigPayload(
                    sessionID: sessionID,
                    attachmentID: attachmentID,
                    values: values,
                    requiresIdle: requiresIdle
                ),
                forKey: DynamicCodingKey("UpdateSessionConfig")
            )
        case .getProviderCatalog:
            try container.encodeNil(forKey: DynamicCodingKey("GetProviderCatalog"))
        case let .getProviderAuthStatus(provider):
            try container.encode(
                GetProviderAuthStatusPayload(provider: provider),
                forKey: DynamicCodingKey("GetProviderAuthStatus")
            )
        case let .startProviderLogin(provider):
            try container.encode(
                GetProviderAuthStatusPayload(provider: provider),
                forKey: DynamicCodingKey("StartProviderLogin")
            )
        case let .logoutProvider(provider):
            try container.encode(
                GetProviderAuthStatusPayload(provider: provider),
                forKey: DynamicCodingKey("LogoutProvider")
            )
        case let .listMcpServers(workspaceID):
            try container.encode(
                WorkspaceScopedPayload(workspaceID: workspaceID),
                forKey: DynamicCodingKey("ListMcpServers")
            )
        case let .listSkills(workspaceID):
            try container.encode(
                WorkspaceScopedPayload(workspaceID: workspaceID),
                forKey: DynamicCodingKey("ListSkills")
            )
        case let .submitPrompt(sessionID, attachmentID, targetAgentID, prompt):
            try container.encode(
                SubmitPromptPayload(
                    sessionID: sessionID,
                    attachmentID: attachmentID,
                    targetAgentID: targetAgentID,
                    prompt: prompt,
                    attachments: []
                ),
                forKey: DynamicCodingKey("SubmitPrompt")
            )
        case let .cancelActivePrompt(sessionID, attachmentID):
            try container.encode(
                CancelActivePromptPayload(sessionID: sessionID, attachmentID: attachmentID),
                forKey: DynamicCodingKey("CancelActivePrompt")
            )
        case let .respondToInteraction(sessionID, interactionID, choiceID):
            try container.encode(
                RespondToInteractionPayload(
                    sessionID: sessionID,
                    interactionID: interactionID,
                    choiceID: choiceID
                ),
                forKey: DynamicCodingKey("RespondToInteraction")
            )
        case let .getWorkspaceLiveSyncStatus(sessionID):
            try container.encode(
                GetWorkspaceLiveSyncStatusPayload(sessionID: sessionID),
                forKey: DynamicCodingKey("GetWorkspaceLiveSyncStatus")
            )
        case let .attachWorkspaceLink(sessionID, linkRef, repoRoot):
            try container.encode(
                AttachWorkspaceLinkPayload(sessionID: sessionID, linkRef: linkRef, repoRoot: repoRoot),
                forKey: DynamicCodingKey("AttachWorkspaceLink")
            )
        case let .setWorkspaceLiveSyncMode(sessionID, mode):
            try container.encode(
                SetWorkspaceLiveSyncModePayload(sessionID: sessionID, mode: mode),
                forKey: DynamicCodingKey("SetWorkspaceLiveSyncMode")
            )
        case let .setUserConfigValue(path, value):
            try container.encode(
                SetUserConfigValuePayload(path: path, value: value),
                forKey: DynamicCodingKey("SetUserConfigValue")
            )
        case let .getSessionHistory(sessionID, agentID, roundCount, maxChars):
            try container.encode(
                GetSessionHistoryPayload(
                    sessionID: sessionID,
                    agentID: agentID,
                    roundCount: roundCount,
                    maxChars: maxChars
                ),
                forKey: DynamicCodingKey("GetSessionHistory")
            )
        case let .spawnAgent(sessionID, alias, provider, model, effort, worktreeID):
            try container.encode(
                SpawnAgentPayload(
                    sessionID: sessionID,
                    alias: alias,
                    provider: provider,
                    model: model,
                    effort: effort,
                    worktreeID: worktreeID
                ),
                forKey: DynamicCodingKey("SpawnAgent")
            )
        case let .destroyAgent(sessionID, agentID):
            try container.encode(
                DestroyAgentPayload(sessionID: sessionID, agentID: agentID),
                forKey: DynamicCodingKey("DestroyAgent")
            )
        case let .aliasAgent(sessionID, agentID, alias):
            try container.encode(
                AliasAgentPayload(sessionID: sessionID, agentID: agentID, alias: alias),
                forKey: DynamicCodingKey("AliasAgent")
            )
        case let .updateAgentProfile(sessionID, agentID, provider, model, effort, clearEffort):
            try container.encode(
                UpdateAgentProfilePayload(
                    sessionID: sessionID,
                    agentID: agentID,
                    provider: provider,
                    model: model,
                    effort: effort,
                    clearEffort: clearEffort
                ),
                forKey: DynamicCodingKey("UpdateAgentProfile")
            )
        case let .updateAgentConfig(
            sessionID,
            agentID,
            executionMode,
            clearExecutionMode,
            permissionLevel,
            clearPermissionLevel
        ):
            try container.encode(
                UpdateAgentConfigPayload(
                    sessionID: sessionID,
                    agentID: agentID,
                    executionMode: executionMode,
                    clearExecutionMode: clearExecutionMode,
                    permissionLevel: permissionLevel,
                    clearPermissionLevel: clearPermissionLevel
                ),
                forKey: DynamicCodingKey("UpdateAgentConfig")
            )
        case let .focusAgent(sessionID, agentID):
            try container.encode(
                FocusAgentPayload(sessionID: sessionID, agentID: agentID),
                forKey: DynamicCodingKey("FocusAgent")
            )
        case let .cycleAgentFocus(sessionID):
            try container.encode(
                CycleAgentFocusPayload(sessionID: sessionID),
                forKey: DynamicCodingKey("CycleAgentFocus")
            )
        }
    }
}

private struct CreateSessionPayload: Encodable {
    let workspaceID: String
    let worktreeID: String
    let alias: String?

    enum CodingKeys: String, CodingKey {
        case workspaceID = "workspace_id"
        case worktreeID = "worktree_id"
        case alias
    }
}

private struct DeleteSessionPayload: Encodable {
    let sessionRef: String
    let workspaceID: String?

    enum CodingKeys: String, CodingKey {
        case sessionRef = "session_ref"
        case workspaceID = "workspace_id"
    }
}

private struct AttachToSessionPayload: Encodable {
    let sessionID: String
    let clientID: String
    let capabilityLevel = "FullTerminal"

    enum CodingKeys: String, CodingKey {
        case sessionID = "session_id"
        case clientID = "client_id"
        case capabilityLevel = "capability_level"
    }
}

private struct DetachFromSessionPayload: Encodable {
    let attachmentID: String

    enum CodingKeys: String, CodingKey {
        case attachmentID = "attachment_id"
    }
}

private struct GetSessionStatePayload: Encodable {
    let sessionID: String

    enum CodingKeys: String, CodingKey {
        case sessionID = "session_id"
    }
}

private struct GetWorkspaceLiveSyncStatusPayload: Encodable {
    let sessionID: String

    enum CodingKeys: String, CodingKey {
        case sessionID = "session_id"
    }
}

private struct AttachWorkspaceLinkPayload: Encodable {
    let sessionID: String
    let linkRef: String
    let repoRoot: String?
    let branch: String? = nil
    let repoFingerprint: String? = nil

    enum CodingKeys: String, CodingKey {
        case sessionID = "session_id"
        case linkRef = "link_ref"
        case repoRoot = "repo_root"
        case branch
        case repoFingerprint = "repo_fingerprint"
    }
}

private struct SetUserConfigValuePayload: Encodable {
    let path: String
    let value: String
}

private struct SetWorkspaceLiveSyncModePayload: Encodable {
    let sessionID: String
    let mode: String

    enum CodingKeys: String, CodingKey {
        case sessionID = "session_id"
        case mode
    }
}

private struct UpdateSessionConfigPayload: Encodable {
    let sessionID: String
    let attachmentID: String
    let values: [String: String]
    let requiresIdle: Bool

    enum CodingKeys: String, CodingKey {
        case sessionID = "session_id"
        case attachmentID = "attachment_id"
        case values
        case requiresIdle = "requires_idle"
    }
}

private struct GetProviderAuthStatusPayload: Encodable {
    let provider: String
}

private struct WorkspaceScopedPayload: Encodable {
    let workspaceID: String?

    enum CodingKeys: String, CodingKey {
        case workspaceID = "workspace_id"
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encodeOptional(workspaceID, forKey: .workspaceID)
    }
}

private struct SubmitPromptPayload: Encodable {
    let sessionID: String
    let attachmentID: String
    let targetAgentID: String?
    let prompt: String
    let attachments: [PromptAttachmentPayload]

    enum CodingKeys: String, CodingKey {
        case sessionID = "session_id"
        case attachmentID = "attachment_id"
        case targetAgentID = "target_agent_id"
        case prompt
        case attachments
    }
}

private struct PromptAttachmentPayload: Encodable {}

private struct CancelActivePromptPayload: Encodable {
    let sessionID: String
    let attachmentID: String

    enum CodingKeys: String, CodingKey {
        case sessionID = "session_id"
        case attachmentID = "attachment_id"
    }
}

private struct RespondToInteractionPayload: Encodable {
    let sessionID: String
    let interactionID: String
    let choiceID: String

    enum CodingKeys: String, CodingKey {
        case sessionID = "session_id"
        case interactionID = "interaction_id"
        case choiceID = "choice_id"
    }
}

private struct GetSessionHistoryPayload: Encodable {
    let sessionID: String
    let agentID: String?
    let roundCount: Int
    let maxChars: Int
    let beforeEntryIndex: Int? = nil
    let beforeEntryCharOffset: Int? = nil

    enum CodingKeys: String, CodingKey {
        case sessionID = "session_id"
        case agentID = "agent_id"
        case roundCount = "round_count"
        case maxChars = "max_chars"
        case beforeEntryIndex = "before_entry_index"
        case beforeEntryCharOffset = "before_entry_char_offset"
    }
}

private struct SpawnAgentPayload: Encodable {
    let sessionID: String
    let alias: String?
    let provider: String
    let model: String?
    let effort: String?
    let executionMode: String? = nil
    let permissionLevel: String? = nil
    let worktreeID: String?
    let kernelRef: String? = nil
    let worktreePlacement: EmptyWorktreePlacementPayload? = nil

    enum CodingKeys: String, CodingKey {
        case sessionID = "session_id"
        case alias
        case provider
        case model
        case effort
        case executionMode = "execution_mode"
        case permissionLevel = "permission_level"
        case worktreeID = "worktree_id"
        case kernelRef = "kernel_ref"
        case worktreePlacement = "worktree_placement"
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(sessionID, forKey: .sessionID)
        try container.encode(provider, forKey: .provider)
        try container.encodeOptional(alias, forKey: .alias)
        try container.encodeOptional(model, forKey: .model)
        try container.encodeOptional(effort, forKey: .effort)
        try container.encodeOptional(executionMode, forKey: .executionMode)
        try container.encodeOptional(permissionLevel, forKey: .permissionLevel)
        try container.encodeOptional(worktreeID, forKey: .worktreeID)
        try container.encodeOptional(kernelRef, forKey: .kernelRef)
        try container.encodeOptional(worktreePlacement, forKey: .worktreePlacement)
    }
}

private struct EmptyWorktreePlacementPayload: Encodable {}

private struct DestroyAgentPayload: Encodable {
    let sessionID: String
    let agentID: String

    enum CodingKeys: String, CodingKey {
        case sessionID = "session_id"
        case agentID = "agent_id"
    }
}

private struct AliasAgentPayload: Encodable {
    let sessionID: String
    let agentID: String
    let alias: String

    enum CodingKeys: String, CodingKey {
        case sessionID = "session_id"
        case agentID = "agent_id"
        case alias
    }
}

private struct UpdateAgentProfilePayload: Encodable {
    let sessionID: String
    let agentID: String
    let provider: String?
    let model: String?
    let effort: String?
    let clearEffort: Bool

    enum CodingKeys: String, CodingKey {
        case sessionID = "session_id"
        case agentID = "agent_id"
        case provider
        case model
        case effort
        case clearEffort = "clear_effort"
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(sessionID, forKey: .sessionID)
        try container.encode(agentID, forKey: .agentID)
        try container.encodeOptional(provider, forKey: .provider)
        try container.encodeOptional(model, forKey: .model)
        try container.encodeOptional(effort, forKey: .effort)
        try container.encode(clearEffort, forKey: .clearEffort)
    }
}

private struct UpdateAgentConfigPayload: Encodable {
    let sessionID: String
    let agentID: String
    let executionMode: String?
    let clearExecutionMode: Bool
    let permissionLevel: String?
    let clearPermissionLevel: Bool

    enum CodingKeys: String, CodingKey {
        case sessionID = "session_id"
        case agentID = "agent_id"
        case executionMode = "execution_mode"
        case clearExecutionMode = "clear_execution_mode"
        case permissionLevel = "permission_level"
        case clearPermissionLevel = "clear_permission_level"
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(sessionID, forKey: .sessionID)
        try container.encode(agentID, forKey: .agentID)
        try container.encodeOptional(executionMode, forKey: .executionMode)
        try container.encode(clearExecutionMode, forKey: .clearExecutionMode)
        try container.encodeOptional(permissionLevel, forKey: .permissionLevel)
        try container.encode(clearPermissionLevel, forKey: .clearPermissionLevel)
    }
}

private struct FocusAgentPayload: Encodable {
    let sessionID: String
    let agentID: String

    enum CodingKeys: String, CodingKey {
        case sessionID = "session_id"
        case agentID = "agent_id"
    }
}

private struct CycleAgentFocusPayload: Encodable {
    let sessionID: String

    enum CodingKeys: String, CodingKey {
        case sessionID = "session_id"
    }
}

private extension KeyedEncodingContainer {
    mutating func encodeOptional<T: Encodable>(_ value: T?, forKey key: Key) throws {
        if let value {
            try encode(value, forKey: key)
        } else {
            try encodeNil(forKey: key)
        }
    }
}
