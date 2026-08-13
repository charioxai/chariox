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
    public let workspaceLiveSyncMode: String?
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
        workspaceLiveSyncMode: String? = nil,
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
        self.workspaceLiveSyncMode = workspaceLiveSyncMode
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
        case workspaceLiveSyncMode = "workspace_live_sync_mode"
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
        workspaceLiveSyncMode = try container.decodeIfPresent(String.self, forKey: .workspaceLiveSyncMode)
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
    public let syncGroups: [WorkspaceLiveSyncGroupStatus]
    public let targets: [WorkspaceLiveSyncTargetStatus]
    public let conflicts: [WorkspaceLiveSyncConflictSummary]
    public let ignore: WorkspaceLiveSyncIgnoreStatus

    enum CodingKeys: String, CodingKey {
        case sessionID = "session_id"
        case mode
        case footerState = "footer_state"
        case syncGroups = "sync_groups"
        case targets
        case conflicts
        case ignore
    }
}

public struct WorkspaceLiveSyncGroupStatus: Equatable, Sendable, Decodable {
    public let groupID: String
    public let groupName: String
    public let targetCount: Int
    public let readyTargets: Int
    public let degradedTargets: Int
    public let conflictedTargets: Int

    enum CodingKeys: String, CodingKey {
        case groupID = "group_id"
        case groupName = "group_name"
        case targetCount = "target_count"
        case readyTargets = "ready_targets"
        case degradedTargets = "degraded_targets"
        case conflictedTargets = "conflicted_targets"
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

public struct CharioxMcpServerConfig: Equatable, Sendable, Decodable {
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

public struct CharioxSkillMetadata: Equatable, Sendable, Decodable {
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
