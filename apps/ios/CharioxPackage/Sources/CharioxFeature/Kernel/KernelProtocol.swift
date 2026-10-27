import Foundation

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
    case mcpServersListed([CharioxMcpServerConfig])
    case skillsListed([CharioxSkillMetadata])
    case promptSubmitted(RuntimeSession)
    case promptCancelled
    case interactionResponded(interactionID: String, session: RuntimeSession)
    case workspaceLiveSyncStatus(WorkspaceLiveSyncStatus)
    case workspaceLinkAttached(session: RuntimeSession)
    case workspaceLiveSyncModeUpdated(session: RuntimeSession)
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
        if let key = container.allKeys.first(where: { $0.stringValue == "WorkspaceLiveSyncModeUpdated" }) {
            let payload = try container.decode(SessionPayload.self, forKey: key)
            self = .workspaceLiveSyncModeUpdated(session: payload.session)
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
    let mcps: [CharioxMcpServerConfig]
}

private struct SkillsListedPayload: Decodable, Equatable, Sendable {
    let skills: [CharioxSkillMetadata]
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

struct DynamicCodingKey: CodingKey {
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
