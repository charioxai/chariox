import Foundation

public struct RuntimeSession: Identifiable, Equatable, Sendable, Decodable {
    public let id: String
    public let alias: String?
    public let workspaceID: String
    public let worktreeID: String
    public let status: String
    public let configState: SessionConfigState?
    public let activePrompt: PromptQueueItem?
    public let queuedPrompts: [PromptQueueItem]
    public let promptStates: [String: AgentPromptState]
    public let activeInteractions: [RuntimeInteraction]
    public let focusedAgentID: String?
    public let agents: [AgentInstance]
    public let createdAtMs: Int64
    public let lastUsedAtMs: Int64?

    public init(
        id: String,
        alias: String?,
        workspaceID: String,
        worktreeID: String,
        status: String,
        configState: SessionConfigState? = nil,
        activePrompt: PromptQueueItem? = nil,
        queuedPrompts: [PromptQueueItem] = [],
        promptStates: [String: AgentPromptState] = [:],
        activeInteractions: [RuntimeInteraction] = [],
        focusedAgentID: String?,
        agents: [AgentInstance],
        createdAtMs: Int64,
        lastUsedAtMs: Int64?
    ) {
        self.id = id
        self.alias = alias
        self.workspaceID = workspaceID
        self.worktreeID = worktreeID
        self.status = status
        self.configState = configState
        self.activePrompt = activePrompt
        self.queuedPrompts = queuedPrompts
        self.promptStates = promptStates
        self.activeInteractions = activeInteractions
        self.focusedAgentID = focusedAgentID
        self.agents = agents
        self.createdAtMs = createdAtMs
        self.lastUsedAtMs = lastUsedAtMs
    }

    enum CodingKeys: String, CodingKey {
        case id
        case alias
        case workspaceID = "workspace_id"
        case worktreeID = "worktree_id"
        case status
        case configState = "config_state"
        case activePrompt = "active_prompt"
        case queuedPrompts = "queued_prompts"
        case promptStates = "prompt_states"
        case activeInteractions = "active_interactions"
        case focusedAgentID = "focused_agent_id"
        case agents
        case createdAtMs = "created_at_ms"
        case lastUsedAtMs = "last_used_at_ms"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(String.self, forKey: .id)
        alias = try container.decodeIfPresent(String.self, forKey: .alias)
        workspaceID = try container.decode(String.self, forKey: .workspaceID)
        worktreeID = try container.decode(String.self, forKey: .worktreeID)
        status = try container.decode(String.self, forKey: .status)
        configState = try container.decodeIfPresent(SessionConfigState.self, forKey: .configState)
        activePrompt = try container.decodeIfPresent(PromptQueueItem.self, forKey: .activePrompt)
        queuedPrompts = try container.decodeIfPresent([PromptQueueItem].self, forKey: .queuedPrompts) ?? []
        promptStates = try container.decodeIfPresent([String: AgentPromptState].self, forKey: .promptStates) ?? [:]
        activeInteractions = try container.decodeIfPresent([RuntimeInteraction].self, forKey: .activeInteractions) ?? []
        focusedAgentID = try container.decodeIfPresent(String.self, forKey: .focusedAgentID)
        agents = try container.decode([AgentInstance].self, forKey: .agents)
        createdAtMs = try container.decode(Int64.self, forKey: .createdAtMs)
        lastUsedAtMs = try container.decodeIfPresent(Int64.self, forKey: .lastUsedAtMs)
    }
}

public struct SessionConfigState: Equatable, Sendable, Decodable {
    public let version: Int64
    public let values: [String: String]
    public let updatedByAttachmentID: String?

    enum CodingKeys: String, CodingKey {
        case version
        case values
        case updatedByAttachmentID = "updated_by_attachment_id"
    }
}

public struct AgentInstance: Identifiable, Equatable, Sendable, Decodable {
    public let id: String
    public let agentRef: String
    public let sessionID: String?
    public let alias: String?
    public let provider: String
    public let model: String?
    public let effort: String?
    public let executionModeOverride: String?
    public let permissionLevelOverride: String?
    public let worktreeID: String?
    public let state: String
    public let isProcessing: Bool

    enum CodingKeys: String, CodingKey {
        case id
        case agentRef = "agent_ref"
        case sessionID = "session_id"
        case alias
        case provider
        case model
        case effort
        case executionModeOverride = "execution_mode_override"
        case permissionLevelOverride = "permission_level_override"
        case worktreeID = "worktree_id"
        case state
        case isProcessing = "is_processing"
    }
}

public struct PromptQueueItem: Identifiable, Equatable, Sendable, Decodable {
    public let id: String
    public let sourceAttachmentID: String
    public let targetAgentID: String?
    public let prompt: String
    public let status: String

    enum CodingKeys: String, CodingKey {
        case id
        case sourceAttachmentID = "source_attachment_id"
        case targetAgentID = "target_agent_id"
        case prompt
        case status
    }
}

public struct AgentPromptState: Equatable, Sendable, Decodable {
    public let activePrompt: PromptQueueItem?
    public let queuedPrompts: [PromptQueueItem]

    enum CodingKeys: String, CodingKey {
        case activePrompt = "active_prompt"
        case queuedPrompts = "queued_prompts"
    }

    public init(activePrompt: PromptQueueItem?, queuedPrompts: [PromptQueueItem] = []) {
        self.activePrompt = activePrompt
        self.queuedPrompts = queuedPrompts
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        activePrompt = try container.decodeIfPresent(PromptQueueItem.self, forKey: .activePrompt)
        queuedPrompts = try container.decodeIfPresent([PromptQueueItem].self, forKey: .queuedPrompts) ?? []
    }
}

public struct RuntimeInteraction: Identifiable, Equatable, Sendable, Decodable {
    public let id: String
    public let agentID: String
    public let kind: String
    public let level: String
    public let title: String?
    public let message: String
    public let choices: [RuntimeInteractionChoice]
    public let timeoutSeconds: Int?
    public let defaultOnTimeout: String?
    public let requestedAtMs: Int64

    enum CodingKeys: String, CodingKey {
        case id
        case agentID = "agent_id"
        case kind
        case level
        case title
        case message
        case choices
        case timeoutSeconds = "timeout_sec"
        case defaultOnTimeout = "default_on_timeout"
        case requestedAtMs = "requested_at_ms"
    }
}

public struct RuntimeInteractionChoice: Identifiable, Equatable, Sendable, Decodable {
    public let id: String
    public let label: String
    public let reply: String
    public let style: String?
}

public struct WorkspaceLiveSyncStatus: Equatable, Sendable, Decodable {
    public let sessionID: String
    public let mode: String
    public let footerState: String
    public let targets: [WorkspaceLiveSyncTargetStatus]
    public let conflicts: [WorkspaceLiveSyncConflictSummary]
    public let ignore: WorkspaceLiveSyncIgnoreStatus

    enum CodingKeys: String, CodingKey {
        case sessionID = "session_id"
        case mode
        case footerState = "footer_state"
        case targets
        case conflicts
        case ignore
    }
}

public struct WorkspaceLiveSyncTargetStatus: Equatable, Sendable, Decodable {
    public let linkID: String
    public let linkName: String
    public let userID: String
    public let machineID: String
    public let kernelID: String
    public let repoRoot: String
    public let branch: String?
    public let repoFingerprint: String?
    public let status: String
    public let attachedAtMs: Int64

    enum CodingKeys: String, CodingKey {
        case linkID = "link_id"
        case linkName = "link_name"
        case userID = "user_id"
        case machineID = "machine_id"
        case kernelID = "kernel_id"
        case repoRoot = "repo_root"
        case branch
        case repoFingerprint = "repo_fingerprint"
        case status
        case attachedAtMs = "attached_at_ms"
    }
}

public struct WorkspaceLiveSyncConflictSummary: Equatable, Sendable, Decodable {
    public let conflictID: String
    public let linkID: String
    public let sourceAgentID: String
    public let targetUserID: String
    public let targetRepoRoot: String
    public let path: String
    public let nextAction: String

    enum CodingKeys: String, CodingKey {
        case conflictID = "conflict_id"
        case linkID = "link_id"
        case sourceAgentID = "source_agent_id"
        case targetUserID = "target_user_id"
        case targetRepoRoot = "target_repo_root"
        case path
        case nextAction = "next_action"
    }
}

public struct WorkspaceLiveSyncIgnoreStatus: Equatable, Sendable, Decodable {
    public let ignoreFile: String?
    public let rules: [String]
    public let forceExcludes: [String]

    enum CodingKeys: String, CodingKey {
        case ignoreFile = "ignore_file"
        case rules
        case forceExcludes = "force_excludes"
    }
}

public struct UserConfigMutationEffect: Equatable, Sendable, Decodable {
    public let kind: String
    public let path: String
    public let message: String
}

public struct ProviderCatalog: Equatable, Sendable, Decodable {
    public let all: [ProviderInfo]
    public let `default`: [String: String]
    public let connected: [String]
}

public struct ProviderInfo: Identifiable, Equatable, Sendable, Decodable {
    public let id: String
    public let name: String
    public let remoteMachineAliases: [String]
    public let models: [String: ProviderModel]

    enum CodingKeys: String, CodingKey {
        case id
        case name
        case remoteMachineAliases = "remote_machine_aliases"
        case models
    }
}

public struct ProviderModel: Identifiable, Equatable, Sendable, Decodable {
    public let id: String
    public let name: String
    public let status: String
    public let limit: ProviderModelLimit?
    public let variants: [String: JSONValue]
}

public struct ProviderModelLimit: Equatable, Sendable, Decodable {
    public let context: Int64
    public let input: Int64?
    public let output: Int64?
}

public struct ProviderAuthStatus: Equatable, Sendable, Decodable {
    public let provider: String
    public let authState: String
    public let accountProfile: String?
    public let loginHint: String?
    public let detectedVersion: String?

    enum CodingKeys: String, CodingKey {
        case provider
        case authState = "auth_state"
        case accountProfile = "account_profile"
        case loginHint = "login_hint"
        case detectedVersion = "detected_version"
    }
}

public struct ProviderLoginStart: Equatable, Sendable, Decodable {
    public let provider: String
    public let loginKind: String
    public let loginID: String?
    public let authURL: String?
    public let verificationURL: String?
    public let userCode: String?

    enum CodingKeys: String, CodingKey {
        case provider
        case loginKind = "login_kind"
        case loginID = "login_id"
        case authURL = "auth_url"
        case verificationURL = "verification_url"
        case userCode = "user_code"
    }
}

public struct ArrobaMcpServerConfig: Equatable, Sendable, Decodable {
    public let name: String
    public let transport: [String: JSONValue]
    public let enabled: Bool?
    public let required: Bool?
    public let startupTimeoutSeconds: Int?
    public let toolTimeoutSeconds: Int?
    public let enabledTools: [String]?
    public let disabledTools: [String]?
    public let tools: [String: JSONValue]?

    enum CodingKeys: String, CodingKey {
        case name
        case transport
        case enabled
        case required
        case startupTimeoutSeconds = "startup_timeout_sec"
        case toolTimeoutSeconds = "tool_timeout_sec"
        case enabledTools = "enabled_tools"
        case disabledTools = "disabled_tools"
        case tools
    }
}

public struct ArrobaSkillMetadata: Equatable, Sendable, Decodable {
    public let name: String
    public let description: String
    public let shortDescription: String?
    public let path: String

    enum CodingKeys: String, CodingKey {
        case name
        case description
        case shortDescription = "short_description"
        case path
    }
}

public enum JSONValue: Equatable, Sendable, Decodable {
    case string(String)
    case number(Double)
    case bool(Bool)
    case object([String: JSONValue])
    case array([JSONValue])
    case null

    public init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let value = try? container.decode(Bool.self) {
            self = .bool(value)
        } else if let value = try? container.decode(Double.self) {
            self = .number(value)
        } else if let value = try? container.decode(String.self) {
            self = .string(value)
        } else if let value = try? container.decode([String: JSONValue].self) {
            self = .object(value)
        } else {
            self = .array(try container.decode([JSONValue].self))
        }
    }
}

public struct RuntimeAttachment: Equatable, Sendable, Decodable {
    public let id: String
    public let sessionID: String

    enum CodingKeys: String, CodingKey {
        case id
        case sessionID = "session_id"
    }
}

public struct TerminalOutputRecord: Equatable, Sendable, Decodable {
    public let agentID: String?
    public let kind: String
    public let mergeKey: String?
    public let bytes: [UInt8]

    public var text: String {
        String(decoding: bytes, as: UTF8.self)
    }

    enum CodingKeys: String, CodingKey {
        case agentID = "agent_id"
        case kind
        case mergeKey = "merge_key"
        case bytes
    }
}

public struct RuntimeNoticeRecord: Equatable, Sendable, Decodable {
    public let message: String
}

public struct SessionHistoryPageEntry: Equatable, Sendable, Decodable {
    public let entryIndex: Int
    public let fragmentStart: Int
    public let fragmentEnd: Int
    public let totalChars: Int
    public let entry: SessionHistoryEntry

    enum CodingKeys: String, CodingKey {
        case entryIndex = "entry_index"
        case fragmentStart = "fragment_start"
        case fragmentEnd = "fragment_end"
        case totalChars = "total_chars"
        case entry
    }
}

public struct SessionHistoryEntry: Equatable, Sendable, Decodable {
    public let agentID: String?
    public let providerRunID: String?
    public let kind: String
    public let mergeKey: String?
    public let text: String

    enum CodingKeys: String, CodingKey {
        case agentID = "agent_id"
        case providerRunID = "provider_run_id"
        case kind
        case mergeKey = "merge_key"
        case text
    }
}

public struct ReplayGap: Equatable, Sendable, Decodable {
    public let requestedFromEventID: Int64
    public let firstRetainedEventID: Int64?
    public let latestEventID: Int64?
    public let message: String?

    enum CodingKeys: String, CodingKey {
        case requestedFromEventID = "requested_from_event_id"
        case firstRetainedEventID = "first_retained_event_id"
        case latestEventID = "latest_event_id"
        case message
    }
}

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
    case setWorkspaceLiveSyncMode(mode: String)
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
        case let .setWorkspaceLiveSyncMode(mode):
            try container.encode(
                SetWorkspaceLiveSyncModePayload(mode: mode),
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

public struct KernelSubscribeFrame: Encodable, Sendable {
    public let type = "subscribe"
    public let requestID: String
    public let sessionID: String
    public let attachmentID: String
    public let resumeFromEventID: Int64?

    public init(
        requestID: String = UUID().uuidString,
        sessionID: String,
        attachmentID: String,
        resumeFromEventID: Int64?
    ) {
        self.requestID = requestID
        self.sessionID = sessionID
        self.attachmentID = attachmentID
        self.resumeFromEventID = resumeFromEventID
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(type, forKey: .type)
        try container.encode(requestID, forKey: .requestID)
        try container.encode(sessionID, forKey: .sessionID)
        try container.encode(attachmentID, forKey: .attachmentID)
        if let resumeFromEventID {
            try container.encode(resumeFromEventID, forKey: .resumeFromEventID)
        } else {
            try container.encodeNil(forKey: .resumeFromEventID)
        }
    }

    enum CodingKeys: String, CodingKey {
        case type
        case requestID = "request_id"
        case sessionID = "session_id"
        case attachmentID = "attachment_id"
        case resumeFromEventID = "resume_from_event_id"
    }
}

public struct KernelUnsubscribeFrame: Encodable, Sendable {
    public let type = "unsubscribe"
    public let requestID: String

    public init(requestID: String = UUID().uuidString) {
        self.requestID = requestID
    }

    enum CodingKeys: String, CodingKey {
        case type
        case requestID = "request_id"
    }
}

public struct KernelRequestFrame: Encodable, Sendable {
    public let type = "request"
    public let requestID: String
    public let request: LocalDaemonRequest

    public init(requestID: String = UUID().uuidString, request: LocalDaemonRequest) {
        self.requestID = requestID
        self.request = request
    }

    enum CodingKeys: String, CodingKey {
        case type
        case requestID = "request_id"
        case request
    }
}

public enum KernelTransportFrame: Decodable, Sendable {
    case response(KernelResponseFrame)
    case event(KernelEventFrame)

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: FrameCodingKeys.self)
        let type = try container.decode(String.self, forKey: .type)
        switch type {
        case "response":
            self = .response(try KernelResponseFrame(from: decoder))
        case "event":
            self = .event(try KernelEventFrame(from: decoder))
        default:
            throw DecodingError.dataCorruptedError(
                forKey: .type,
                in: container,
                debugDescription: "Unsupported kernel transport frame type: \(type)"
            )
        }
    }
}

public struct KernelResponseFrame: Decodable, Sendable {
    public let type: String
    public let requestID: String
    public let response: LocalDaemonResponse?
    public let error: KernelTransportError?

    enum CodingKeys: String, CodingKey {
        case type
        case requestID = "request_id"
        case response
        case error
    }
}

public struct KernelTransportError: Error, Equatable, Sendable, Decodable {
    public let code: String
    public let message: String
    public let retryable: Bool
}

public enum LocalDaemonResponse: Equatable, Sendable, Decodable {
    case sessionsListed([RuntimeSession])
    case sessionCreated(RuntimeSession)
    case sessionDeleted(RuntimeSession)
    case sessionState(RuntimeSession)
    case sessionConfigUpdated(session: RuntimeSession, config: SessionConfigState)
    case sessionAttached(RuntimeAttachment)
    case sessionDetached(RuntimeAttachment)
    case providerCatalog(ProviderCatalog)
    case providerAuthStatus(ProviderAuthStatus)
    case providerLoginStarted(ProviderLoginStart)
    case providerLoggedOut(String)
    case mcpServersListed([ArrobaMcpServerConfig])
    case skillsListed([ArrobaSkillMetadata])
    case promptSubmitted(RuntimeSession)
    case promptCancelled
    case interactionResponded(interactionID: String, session: RuntimeSession)
    case workspaceLiveSyncStatus(WorkspaceLiveSyncStatus)
    case workspaceLinkAttached(session: RuntimeSession)
    case userConfigUpdated(path: String, effects: [UserConfigMutationEffect])
    case sessionHistory([SessionHistoryPageEntry])
    case agentSpawned(AgentInstance)
    case agentDestroyed(AgentInstance)
    case agentAliased(agent: AgentInstance, session: RuntimeSession)
    case agentProfileUpdated(agent: AgentInstance, session: RuntimeSession)
    case agentConfigUpdated(agent: AgentInstance, session: RuntimeSession)
    case agentFocused(AgentInstance?)
    case unknown(String)

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: DynamicCodingKey.self)
        if let key = container.allKeys.first(where: { $0.stringValue == "SessionsListed" }) {
            let payload = try container.decode(SessionsListedPayload.self, forKey: key)
            self = .sessionsListed(payload.sessions)
            return
        }
        if let key = container.allKeys.first(where: { $0.stringValue == "SessionCreated" }) {
            let payload = try container.decode(SessionPayload.self, forKey: key)
            self = .sessionCreated(payload.session)
            return
        }
        if let key = container.allKeys.first(where: { $0.stringValue == "SessionDeleted" }) {
            let payload = try container.decode(SessionPayload.self, forKey: key)
            self = .sessionDeleted(payload.session)
            return
        }
        if let key = container.allKeys.first(where: { $0.stringValue == "SessionState" }) {
            let payload = try container.decode(SessionPayload.self, forKey: key)
            self = .sessionState(payload.session)
            return
        }
        if let key = container.allKeys.first(where: { $0.stringValue == "SessionConfigUpdated" }) {
            let payload = try container.decode(SessionConfigUpdatedPayload.self, forKey: key)
            self = .sessionConfigUpdated(session: payload.session, config: payload.config)
            return
        }
        if let key = container.allKeys.first(where: { $0.stringValue == "SessionAttached" }) {
            let payload = try container.decode(SessionAttachedPayload.self, forKey: key)
            self = .sessionAttached(payload.attachment)
            return
        }
        if let key = container.allKeys.first(where: { $0.stringValue == "SessionDetached" }) {
            let payload = try container.decode(SessionAttachedPayload.self, forKey: key)
            self = .sessionDetached(payload.attachment)
            return
        }
        if let key = container.allKeys.first(where: { $0.stringValue == "ProviderCatalog" }) {
            let payload = try container.decode(ProviderCatalogPayload.self, forKey: key)
            self = .providerCatalog(payload.catalog)
            return
        }
        if let key = container.allKeys.first(where: { $0.stringValue == "ProviderAuthStatus" }) {
            let payload = try container.decode(ProviderAuthStatusPayload.self, forKey: key)
            self = .providerAuthStatus(payload.status)
            return
        }
        if let key = container.allKeys.first(where: { $0.stringValue == "ProviderLoginStarted" }) {
            let payload = try container.decode(ProviderLoginStartedPayload.self, forKey: key)
            self = .providerLoginStarted(payload.login)
            return
        }
        if let key = container.allKeys.first(where: { $0.stringValue == "ProviderLoggedOut" }) {
            let payload = try container.decode(ProviderLoggedOutPayload.self, forKey: key)
            self = .providerLoggedOut(payload.provider)
            return
        }
        if let key = container.allKeys.first(where: { $0.stringValue == "McpServersListed" }) {
            let payload = try container.decode(McpServersListedPayload.self, forKey: key)
            self = .mcpServersListed(payload.mcps)
            return
        }
        if let key = container.allKeys.first(where: { $0.stringValue == "SkillsListed" }) {
            let payload = try container.decode(SkillsListedPayload.self, forKey: key)
            self = .skillsListed(payload.skills)
            return
        }
        if let key = container.allKeys.first(where: { $0.stringValue == "PromptSubmitted" }) {
            let payload = try container.decode(PromptSubmittedPayload.self, forKey: key)
            self = .promptSubmitted(payload.session)
            return
        }
        if container.allKeys.contains(where: { $0.stringValue == "PromptCancelled" }) {
            self = .promptCancelled
            return
        }
        if let key = container.allKeys.first(where: { $0.stringValue == "InteractionResponded" }) {
            let payload = try container.decode(InteractionRespondedPayload.self, forKey: key)
            self = .interactionResponded(interactionID: payload.interactionID, session: payload.session)
            return
        }
        if let key = container.allKeys.first(where: { $0.stringValue == "WorkspaceLiveSyncStatus" }) {
            let payload = try container.decode(WorkspaceLiveSyncStatusPayload.self, forKey: key)
            self = .workspaceLiveSyncStatus(payload.status)
            return
        }
        if let key = container.allKeys.first(where: { $0.stringValue == "WorkspaceLinkAttached" }) {
            let payload = try container.decode(WorkspaceLinkAttachedPayload.self, forKey: key)
            self = .workspaceLinkAttached(session: payload.session)
            return
        }
        if let key = container.allKeys.first(where: { $0.stringValue == "UserConfigUpdated" }) {
            let payload = try container.decode(UserConfigUpdatedPayload.self, forKey: key)
            self = .userConfigUpdated(path: payload.path, effects: payload.effects)
            return
        }
        if let key = container.allKeys.first(where: { $0.stringValue == "SessionHistory" }) {
            let payload = try container.decode(SessionHistoryPayload.self, forKey: key)
            self = .sessionHistory(payload.entries)
            return
        }
        if let key = container.allKeys.first(where: { $0.stringValue == "AgentSpawned" }) {
            let payload = try container.decode(AgentPayload.self, forKey: key)
            self = .agentSpawned(payload.agent)
            return
        }
        if let key = container.allKeys.first(where: { $0.stringValue == "AgentDestroyed" }) {
            let payload = try container.decode(AgentPayload.self, forKey: key)
            self = .agentDestroyed(payload.agent)
            return
        }
        if let key = container.allKeys.first(where: { $0.stringValue == "AgentConfigUpdated" }) {
            let payload = try container.decode(AgentConfigUpdatedPayload.self, forKey: key)
            self = .agentConfigUpdated(agent: payload.agent, session: payload.session)
            return
        }
        if let key = container.allKeys.first(where: { $0.stringValue == "AgentProfileUpdated" }) {
            let payload = try container.decode(AgentConfigUpdatedPayload.self, forKey: key)
            self = .agentProfileUpdated(agent: payload.agent, session: payload.session)
            return
        }
        if let key = container.allKeys.first(where: { $0.stringValue == "AgentAliased" }) {
            let payload = try container.decode(AgentConfigUpdatedPayload.self, forKey: key)
            self = .agentAliased(agent: payload.agent, session: payload.session)
            return
        }
        if let key = container.allKeys.first(where: { $0.stringValue == "AgentFocused" }) {
            let payload = try container.decode(OptionalAgentPayload.self, forKey: key)
            self = .agentFocused(payload.agent)
            return
        }
        if let key = container.allKeys.first(where: { $0.stringValue == "AgentFocusCycled" }) {
            let payload = try container.decode(OptionalAgentPayload.self, forKey: key)
            self = .agentFocused(payload.agent)
            return
        }
        self = .unknown(container.allKeys.first?.stringValue ?? "UnknownResponse")
    }
}

public struct KernelEventFrame: Equatable, Sendable, Decodable {
    public let type: String
    public let eventID: Int64
    public let event: KernelEvent

    enum CodingKeys: String, CodingKey {
        case type
        case eventID = "event_id"
        case event
    }
}

public enum KernelEvent: Equatable, Sendable, Decodable {
    case terminalOutput([TerminalOutputRecord])
    case runtimeNotices([RuntimeNoticeRecord])
    case assistantMessageCompleted(sessionID: String, providerRunID: String, agentID: String?, messageID: String, completedAtMs: Int64)
    case sessionSnapshot(RuntimeSession)
    case sessionUnavailable(sessionID: String, message: String)
    case heartbeat(sessionID: String)
    case transportResumed(sessionID: String, resumedFromEventID: Int64?)
    case replayGap(ReplayGap)
    case unknown(String)

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: DynamicCodingKey.self)
        let eventName = try container.decode(String.self, forKey: DynamicCodingKey("event"))
        switch eventName {
        case "terminal_output":
            let records = try container.decode([TerminalOutputRecord].self, forKey: DynamicCodingKey("records"))
            self = .terminalOutput(records)
        case "runtime_notices":
            let notices = try container.decode([RuntimeNoticeRecord].self, forKey: DynamicCodingKey("notices"))
            self = .runtimeNotices(notices)
        case "assistant_message_completed":
            let sessionID = try container.decode(String.self, forKey: DynamicCodingKey("session_id"))
            let providerRunID = try container.decode(String.self, forKey: DynamicCodingKey("provider_run_id"))
            let agentID = try container.decodeIfPresent(String.self, forKey: DynamicCodingKey("agent_id"))
            let messageID = try container.decode(String.self, forKey: DynamicCodingKey("message_id"))
            let completedAtMs = try container.decode(Int64.self, forKey: DynamicCodingKey("completed_at_ms"))
            self = .assistantMessageCompleted(
                sessionID: sessionID,
                providerRunID: providerRunID,
                agentID: agentID,
                messageID: messageID,
                completedAtMs: completedAtMs
            )
        case "session_snapshot":
            let session = try container.decode(RuntimeSession.self, forKey: DynamicCodingKey("session"))
            self = .sessionSnapshot(session)
        case "session_unavailable":
            let sessionID = try container.decode(String.self, forKey: DynamicCodingKey("session_id"))
            let message = try container.decode(String.self, forKey: DynamicCodingKey("message"))
            self = .sessionUnavailable(sessionID: sessionID, message: message)
        case "heartbeat":
            let sessionID = try container.decode(String.self, forKey: DynamicCodingKey("session_id"))
            self = .heartbeat(sessionID: sessionID)
        case "transport_resumed":
            let sessionID = try container.decode(String.self, forKey: DynamicCodingKey("session_id"))
            let resumedFromEventID = try container.decodeIfPresent(
                Int64.self,
                forKey: DynamicCodingKey("resumed_from_event_id")
            )
            self = .transportResumed(sessionID: sessionID, resumedFromEventID: resumedFromEventID)
        case "replay_gap":
            self = .replayGap(try ReplayGap(from: decoder))
        default:
            self = .unknown(eventName)
        }
    }
}

public enum KernelProtocolCodec {
    public static func encodeRequestFrame(_ frame: KernelRequestFrame) throws -> Data {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        return try encoder.encode(frame)
    }

    public static func encodeSubscribeFrame(_ frame: KernelSubscribeFrame) throws -> Data {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        return try encoder.encode(frame)
    }

    public static func encodeUnsubscribeFrame(_ frame: KernelUnsubscribeFrame) throws -> Data {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        return try encoder.encode(frame)
    }

    public static func decodeResponseFrame(_ data: Data) throws -> KernelResponseFrame {
        try JSONDecoder().decode(KernelResponseFrame.self, from: data)
    }

    public static func decodeTransportFrame(_ data: Data) throws -> KernelTransportFrame {
        try JSONDecoder().decode(KernelTransportFrame.self, from: data)
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
    let mode: String
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

private struct SessionsListedPayload: Decodable, Equatable, Sendable {
    let sessions: [RuntimeSession]
}

private struct SessionPayload: Decodable, Equatable, Sendable {
    let session: RuntimeSession
}

private struct SessionConfigUpdatedPayload: Decodable, Equatable, Sendable {
    let config: SessionConfigState
    let session: RuntimeSession
}

private struct WorkspaceLiveSyncStatusPayload: Decodable, Equatable, Sendable {
    let status: WorkspaceLiveSyncStatus
}

private struct WorkspaceLinkAttachedPayload: Decodable, Equatable, Sendable {
    let session: RuntimeSession
}

private struct UserConfigUpdatedPayload: Decodable, Equatable, Sendable {
    let path: String
    let effects: [UserConfigMutationEffect]
}

private struct AgentPayload: Decodable, Equatable, Sendable {
    let agent: AgentInstance
}

private struct OptionalAgentPayload: Decodable, Equatable, Sendable {
    let agent: AgentInstance?
}

private struct AgentConfigUpdatedPayload: Decodable, Equatable, Sendable {
    let agent: AgentInstance
    let session: RuntimeSession
}

private struct SessionAttachedPayload: Decodable, Equatable, Sendable {
    let attachment: RuntimeAttachment
}

private struct ProviderCatalogPayload: Decodable, Equatable, Sendable {
    let catalog: ProviderCatalog
}

private struct ProviderAuthStatusPayload: Decodable, Equatable, Sendable {
    let status: ProviderAuthStatus
}

private struct ProviderLoginStartedPayload: Decodable, Equatable, Sendable {
    let login: ProviderLoginStart
}

private struct ProviderLoggedOutPayload: Decodable, Equatable, Sendable {
    let provider: String
}

private struct McpServersListedPayload: Decodable, Equatable, Sendable {
    let mcps: [ArrobaMcpServerConfig]
}

private struct SkillsListedPayload: Decodable, Equatable, Sendable {
    let skills: [ArrobaSkillMetadata]
}

private struct PromptSubmittedPayload: Decodable, Equatable, Sendable {
    let session: RuntimeSession
}

private struct InteractionRespondedPayload: Decodable, Equatable, Sendable {
    let interactionID: String
    let session: RuntimeSession

    enum CodingKeys: String, CodingKey {
        case interactionID = "interaction_id"
        case session
    }
}

private struct SessionHistoryPayload: Decodable, Equatable, Sendable {
    let entries: [SessionHistoryPageEntry]
}

private enum FrameCodingKeys: String, CodingKey {
    case type
}

private struct DynamicCodingKey: CodingKey {
    let stringValue: String
    let intValue: Int?

    init(_ stringValue: String) {
        self.stringValue = stringValue
        intValue = nil
    }

    init?(stringValue: String) {
        self.init(stringValue)
    }

    init?(intValue: Int) {
        stringValue = "\(intValue)"
        self.intValue = intValue
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
