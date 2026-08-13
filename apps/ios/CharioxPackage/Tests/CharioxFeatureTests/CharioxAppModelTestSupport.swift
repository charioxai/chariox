import Foundation
@testable import CharioxFeature

extension ProviderCatalog {
    static func fixture() -> ProviderCatalog {
        ProviderCatalog(
            all: [
                ProviderInfo(
                    id: "codex",
                    name: "Codex",
                    remoteMachineAliases: [],
                    models: [
                        "gpt-5.4": ProviderModel(
                            id: "gpt-5.4",
                            name: "GPT-5.4",
                            status: "active",
                            limit: nil,
                            variants: ["medium": .object([:])]
                        ),
                    ]
                ),
            ],
            default: ["codex": "gpt-5.4"],
            connected: ["codex"]
        )
    }
}

struct MockKernelClient: KernelClientProtocol {
    let response: LocalDaemonResponse
    var eventFrames: [KernelEventFrame] = []
    var finishEvents = true

    func send(_ request: LocalDaemonRequest, to endpoint: URL) async throws -> LocalDaemonResponse {
        response
    }

    func events(
        sessionID: String,
        attachmentID: String,
        endpoint: URL,
        resumeFromEventID: Int64?
    ) -> AsyncThrowingStream<KernelEventFrame, Error> {
        AsyncThrowingStream { continuation in
            for frame in eventFrames {
                continuation.yield(frame)
            }
            if finishEvents {
                continuation.finish()
            }
        }
    }
}

struct SequencedMockKernelClient: KernelClientProtocol {
    private let queue: ResponseQueue

    init(responses: [LocalDaemonResponse]) {
        queue = ResponseQueue(responses: responses)
    }

    func send(_ request: LocalDaemonRequest, to endpoint: URL) async throws -> LocalDaemonResponse {
        await queue.next()
    }

    func events(
        sessionID: String,
        attachmentID: String,
        endpoint: URL,
        resumeFromEventID: Int64?
    ) -> AsyncThrowingStream<KernelEventFrame, Error> {
        AsyncThrowingStream { continuation in
            continuation.finish()
        }
    }
}

private actor ResponseQueue {
    private var responses: [LocalDaemonResponse]

    init(responses: [LocalDaemonResponse]) {
        self.responses = responses
    }

    func next() -> LocalDaemonResponse {
        responses.isEmpty ? .unknown("NoMockResponse") : responses.removeFirst()
    }
}

extension RuntimeSession {
    static func fixture(
        id: String,
        lastUsedAtMs: Int64,
        focusedAgentID: String? = nil,
        agents: [AgentInstance] = [],
        configState: SessionConfigState? = nil,
        activePrompt: PromptQueueItem? = nil,
        queuedPrompts: [PromptQueueItem] = [],
        promptStates: [String: AgentPromptState] = [:],
        activeInteractions: [RuntimeInteraction] = []
    ) -> RuntimeSession {
        RuntimeSession(
            id: id,
            alias: nil,
            workspaceID: "/repo",
            worktreeID: "/repo",
            status: "Active",
            configState: configState,
            activePrompt: activePrompt,
            queuedPrompts: queuedPrompts,
            promptStates: promptStates,
            activeInteractions: activeInteractions,
            focusedAgentID: focusedAgentID,
            agents: agents,
            createdAtMs: 1,
            lastUsedAtMs: lastUsedAtMs
        )
    }
}

extension RuntimeInteraction {
    static func fixture(id: String, agentID: String) -> RuntimeInteraction {
        RuntimeInteraction(
            id: id,
            agentID: agentID,
            kind: "permission",
            level: "warning",
            title: nil,
            message: "Allow command?",
            choices: [
                RuntimeInteractionChoice(
                    id: "approve",
                    label: "Approve",
                    reply: "approved",
                    style: "primary"
                ),
            ],
            timeoutSeconds: nil,
            defaultOnTimeout: nil,
            requestedAtMs: 1
        )
    }
}

extension AgentInstance {
    static func fixture(
        id: String,
        agentRef: String,
        alias: String,
        state: String,
        executionModeOverride: String? = nil,
        permissionLevelOverride: String? = nil
    ) -> AgentInstance {
        AgentInstance(
            id: id,
            agentRef: agentRef,
            sessionID: "session-1",
            alias: alias,
            provider: "opencode",
            model: "default",
            effort: nil,
            executionModeOverride: executionModeOverride,
            permissionLevelOverride: permissionLevelOverride,
            worktreeID: nil,
            state: state,
            isProcessing: false
        )
    }
}
