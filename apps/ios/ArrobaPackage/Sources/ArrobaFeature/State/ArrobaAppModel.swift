import Foundation
import Observation

@MainActor
@Observable
public final class ArrobaAppModel {
    public var kernelURLText: String
    public var workspacePath: String
    public var worktreePath: String
    public var promptDraft: String
    public var selectedProviderID: String
    public var selectedModelID: String
    public var selectedVariantID: String
    public var responseLayout: MultiAgentResponseLayout
    public private(set) var sessions: [RuntimeSession]
    public private(set) var selectedSessionID: String?
    public private(set) var activeAttachment: RuntimeAttachment?
    public private(set) var subscribedSessionID: String?
    public private(set) var lastEventID: Int64?
    public private(set) var lastHeartbeatAt: Date?
    public private(set) var transcriptEntries: [TranscriptEntry]
    public private(set) var providerCatalog: ProviderCatalog?
    public private(set) var providerAuthStatuses: [String: ProviderAuthStatus]
    public private(set) var mcpServers: [ArrobaMcpServerConfig]
    public private(set) var skills: [ArrobaSkillMetadata]
    public private(set) var eventStreamState: EventStreamState
    public private(set) var connectionState: ConnectionState
    public private(set) var statusMessage: String

    private let client: KernelClientProtocol
    private let defaults: UserDefaults
    private let clientID: String
    private let heartbeatStaleAfterSeconds: TimeInterval

    @ObservationIgnored
    private var eventTask: Task<Void, Never>?

    @ObservationIgnored
    private var heartbeatMonitorTask: Task<Void, Never>?

    @ObservationIgnored
    private var eventStreamStartedAt: Date?

    public init(
        client: KernelClientProtocol = KernelClient(),
        defaults: UserDefaults = .standard,
        heartbeatStaleAfterSeconds: TimeInterval = 15
    ) {
        self.client = client
        self.defaults = defaults
        self.heartbeatStaleAfterSeconds = heartbeatStaleAfterSeconds
        clientID = Self.loadClientID(from: defaults)
        let defaultWorkspacePath = Self.defaultWorkspacePath()
        kernelURLText = defaults.string(forKey: DefaultsKey.kernelURL) ?? "ws://127.0.0.1:43118/kernel"
        workspacePath = defaults.string(forKey: DefaultsKey.workspacePath) ?? defaultWorkspacePath
        worktreePath = defaults.string(forKey: DefaultsKey.worktreePath) ?? defaultWorkspacePath
        selectedProviderID = defaults.string(forKey: DefaultsKey.providerID) ?? "opencode"
        selectedModelID = defaults.string(forKey: DefaultsKey.modelID) ?? "default"
        selectedVariantID = defaults.string(forKey: DefaultsKey.variantID) ?? "low"
        responseLayout = MultiAgentResponseLayout(
            rawValue: defaults.string(forKey: DefaultsKey.responseLayout) ?? ""
        ) ?? .individual
        promptDraft = ""
        sessions = []
        selectedSessionID = nil
        activeAttachment = nil
        subscribedSessionID = nil
        lastEventID = nil
        lastHeartbeatAt = nil
        transcriptEntries = []
        providerCatalog = nil
        providerAuthStatuses = [:]
        mcpServers = []
        skills = []
        eventStreamState = .idle
        connectionState = .idle
        statusMessage = "No kernel connection yet."
    }

    deinit {
        eventTask?.cancel()
        heartbeatMonitorTask?.cancel()
    }

    public var selectedSession: RuntimeSession? {
        guard let selectedSessionID else { return nil }
        return sessions.first { $0.id == selectedSessionID }
    }

    public var commandCenterItems: [CommandCenterItem] {
        CommandCenterCatalog.items(matching: promptDraft, session: selectedSession)
    }

    public func saveDraftConfiguration() {
        defaults.set(kernelURLText, forKey: DefaultsKey.kernelURL)
        defaults.set(workspacePath, forKey: DefaultsKey.workspacePath)
        defaults.set(worktreePath, forKey: DefaultsKey.worktreePath)
        defaults.set(selectedProviderID, forKey: DefaultsKey.providerID)
        defaults.set(selectedModelID, forKey: DefaultsKey.modelID)
        defaults.set(selectedVariantID, forKey: DefaultsKey.variantID)
        defaults.set(responseLayout.rawValue, forKey: DefaultsKey.responseLayout)
    }

    public func refreshSessions() async {
        await perform("Refreshing sessions") {
            let response = try await client.send(.listSessions, to: try endpointURL())
            guard case let .sessionsListed(nextSessions) = response else {
                throw KernelClientError.unexpectedResponse(String(describing: response))
            }
            sessions = nextSessions.sorted { lhs, rhs in
                (lhs.lastUsedAtMs ?? lhs.createdAtMs) > (rhs.lastUsedAtMs ?? rhs.createdAtMs)
            }
            if let selectedSessionID, sessions.contains(where: { $0.id == selectedSessionID }) {
                return "Loaded \(sessions.count) session\(sessions.count == 1 ? "" : "s")."
            }
            selectedSessionID = sessions.first?.id
            return sessions.isEmpty ? "Kernel connected. No sessions yet." : "Kernel connected. Selected latest session."
        }
    }

    public func createSession() async {
        let workspace = workspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        let worktree = worktreePath.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !workspace.isEmpty, !worktree.isEmpty else {
            connectionState = .failed
            statusMessage = "Workspace and worktree paths are required."
            return
        }
        await perform("Creating session") {
            let response = try await client.send(
                .createSession(workspaceID: workspace, worktreeID: worktree, alias: nil),
                to: try endpointURL()
            )
            guard case let .sessionCreated(session) = response else {
                throw KernelClientError.unexpectedResponse(String(describing: response))
            }
            selectedSessionID = session.id
            await refreshSessions()
            return "Created session \(session.shortDisplayID)."
        }
    }

    public func deleteSession(reference: String? = nil) async {
        let targetRef = reference?.nilIfBlank ?? selectedSession?.id
        guard let targetRef else {
            connectionState = .failed
            statusMessage = "Select a session before deleting."
            return
        }
        await perform("Deleting session") {
            let response = try await client.send(
                .deleteSession(sessionRef: targetRef, workspaceID: workspacePath.nilIfBlank),
                to: try endpointURL()
            )
            guard case let .sessionDeleted(session) = response else {
                throw KernelClientError.unexpectedResponse(String(describing: response))
            }
            sessions.removeAll { $0.id == session.id }
            if selectedSessionID == session.id {
                selectedSessionID = sessions.first?.id
            }
            if activeAttachment?.sessionID == session.id {
                activeAttachment = nil
                subscribedSessionID = nil
                stopEventStream(resetCursor: true)
                eventStreamState = .idle
            }
            return "Deleted session \(session.shortDisplayID)."
        }
    }

    public func refreshSelectedSessionState() async {
        guard let session = selectedSession else {
            connectionState = .failed
            statusMessage = "Select a session before refreshing state."
            return
        }
        await perform("Refreshing session") {
            let response = try await client.send(
                .getSessionState(sessionID: session.id),
                to: try endpointURL()
            )
            guard case let .sessionState(updatedSession) = response else {
                throw KernelClientError.unexpectedResponse(String(describing: response))
            }
            upsert(updatedSession)
            selectedSessionID = updatedSession.id
            return "Session state refreshed."
        }
    }

    public func refreshProviderCatalog() async {
        await perform("Loading providers") {
            let response = try await client.send(.getProviderCatalog, to: try endpointURL())
            guard case let .providerCatalog(catalog) = response else {
                throw KernelClientError.unexpectedResponse(String(describing: response))
            }
            providerCatalog = catalog
            return "Loaded \(catalog.connected.count) connected provider\(catalog.connected.count == 1 ? "" : "s")."
        }
    }

    public func refreshProviderAuthStatus(provider: String) async {
        let provider = provider.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !provider.isEmpty else {
            connectionState = .failed
            statusMessage = "Provider id is required."
            return
        }
        await perform("Loading provider auth") {
            let response = try await client.send(
                .getProviderAuthStatus(provider: provider),
                to: try endpointURL()
            )
            guard case let .providerAuthStatus(status) = response else {
                throw KernelClientError.unexpectedResponse(String(describing: response))
            }
            providerAuthStatuses[status.provider] = status
            return providerAuthStatusText(status)
        }
    }

    public func startProviderLogin(provider: String) async {
        let provider = provider.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !provider.isEmpty else {
            connectionState = .failed
            statusMessage = "Provider id is required."
            return
        }
        await perform("Starting provider login") {
            let response = try await client.send(
                .startProviderLogin(provider: provider),
                to: try endpointURL()
            )
            guard case let .providerLoginStarted(login) = response else {
                throw KernelClientError.unexpectedResponse(String(describing: response))
            }
            return providerLoginText(login)
        }
    }

    public func logoutProvider(provider: String) async {
        let provider = provider.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !provider.isEmpty else {
            connectionState = .failed
            statusMessage = "Provider id is required."
            return
        }
        await perform("Logging out provider") {
            let response = try await client.send(
                .logoutProvider(provider: provider),
                to: try endpointURL()
            )
            guard case let .providerLoggedOut(provider) = response else {
                throw KernelClientError.unexpectedResponse(String(describing: response))
            }
            providerAuthStatuses[provider] = nil
            return "\(provider): logged out"
        }
    }

    public func refreshMcpServers() async {
        await perform("Loading MCPs") {
            let response = try await client.send(
                .listMcpServers(workspaceID: workspacePath.nilIfBlank),
                to: try endpointURL()
            )
            guard case let .mcpServersListed(mcps) = response else {
                throw KernelClientError.unexpectedResponse(String(describing: response))
            }
            mcpServers = mcps
            return "Loaded \(mcps.count) MCP server\(mcps.count == 1 ? "" : "s")."
        }
    }

    public func refreshSkills() async {
        await perform("Loading skills") {
            let response = try await client.send(
                .listSkills(workspaceID: workspacePath.nilIfBlank),
                to: try endpointURL()
            )
            guard case let .skillsListed(nextSkills) = response else {
                throw KernelClientError.unexpectedResponse(String(describing: response))
            }
            skills = nextSkills
            return "Loaded \(nextSkills.count) skill\(nextSkills.count == 1 ? "" : "s")."
        }
    }

    public func attachSelectedSession() async {
        guard let session = selectedSession else {
            connectionState = .failed
            statusMessage = "Select a session before attaching."
            return
        }
        await perform("Attaching session") {
            let endpoint = try endpointURL()
            let response = try await client.send(
                .attachToSession(sessionID: session.id, clientID: clientID),
                to: endpoint
            )
            guard case let .sessionAttached(attachment) = response else {
                throw KernelClientError.unexpectedResponse(String(describing: response))
            }
            activeAttachment = attachment
            let resumeFromEventID = subscribedSessionID == session.id ? lastEventID : nil
            subscribedSessionID = session.id
            startEventStream(
                sessionID: session.id,
                attachmentID: attachment.id,
                endpoint: endpoint,
                resumeFromEventID: resumeFromEventID
            )
            await Task.yield()
            await loadRecentHistory(endpoint: endpoint, session: session)
            return "Attached to session \(session.shortDisplayID)."
        }
    }

    public func detachActiveSession() async {
        guard let attachment = activeAttachment else {
            connectionState = .failed
            statusMessage = "No active attachment to detach."
            return
        }
        await perform("Detaching session") {
            let response = try await client.send(
                .detachFromSession(attachmentID: attachment.id),
                to: try endpointURL()
            )
            guard case .sessionDetached = response else {
                throw KernelClientError.unexpectedResponse(String(describing: response))
            }
            activeAttachment = nil
            subscribedSessionID = nil
            stopEventStream(resetCursor: true)
            eventStreamState = .idle
            return "Detached from session."
        }
    }

    public func submitPrompt() async {
        let trimmedPrompt = promptDraft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedPrompt.isEmpty else {
            connectionState = .failed
            statusMessage = "Prompt cannot be empty."
            return
        }
        if trimmedPrompt.hasPrefix("/") {
            await executeSlashCommand(trimmedPrompt)
            return
        }
        guard let session = selectedSession, let attachment = activeAttachment else {
            connectionState = .failed
            statusMessage = "Attach to a session before sending a prompt."
            return
        }
        let prompt = trimmedPrompt.hasSuffix("\n") ? trimmedPrompt : "\(trimmedPrompt)\n"
        await perform("Submitting prompt") {
            let response = try await client.send(
                .submitPrompt(
                    sessionID: session.id,
                    attachmentID: attachment.id,
                    targetAgentID: session.focusedAgentID,
                    prompt: prompt
                ),
                to: try endpointURL()
            )
            guard case let .promptSubmitted(updatedSession) = response else {
                throw KernelClientError.unexpectedResponse(String(describing: response))
            }
            upsert(updatedSession)
            selectedSessionID = updatedSession.id
            promptDraft = ""
            return "Prompt submitted."
        }
    }

    public func cancelActivePrompt() async {
        guard let session = selectedSession, let attachment = activeAttachment else {
            connectionState = .failed
            statusMessage = "Attach to a session before cancelling a prompt."
            return
        }
        await perform("Cancelling prompt") {
            let response = try await client.send(
                .cancelActivePrompt(sessionID: session.id, attachmentID: attachment.id),
                to: try endpointURL()
            )
            guard case .promptCancelled = response else {
                throw KernelClientError.unexpectedResponse(String(describing: response))
            }
            return "Cancellation requested."
        }
    }

    public func respondToInteraction(_ interaction: RuntimeInteraction, choice: RuntimeInteractionChoice) async {
        guard let session = selectedSession else {
            connectionState = .failed
            statusMessage = "Select a session before responding to an interaction."
            return
        }
        await perform("Responding") {
            let response = try await client.send(
                .respondToInteraction(
                    sessionID: session.id,
                    interactionID: interaction.id,
                    choiceID: choice.id
                ),
                to: try endpointURL()
            )
            guard case let .interactionResponded(_, updatedSession) = response else {
                throw KernelClientError.unexpectedResponse(String(describing: response))
            }
            upsert(updatedSession)
            selectedSessionID = updatedSession.id
            return "Responded to \(interaction.displayTitle)."
        }
    }

    public func executeCommandCenterItem(_ item: CommandCenterItem) async {
        promptDraft = item.value
        if item.submitsImmediately {
            await submitPrompt()
        }
    }

    public func focusAgent(_ agent: AgentInstance) async {
        guard let session = selectedSession else {
            connectionState = .failed
            statusMessage = "Select a session before focusing an agent."
            return
        }
        await perform("Focusing agent") {
            let response = try await client.send(
                .focusAgent(sessionID: session.id, agentID: agent.id),
                to: try endpointURL()
            )
            guard case let .agentFocused(focusedAgent) = response else {
                throw KernelClientError.unexpectedResponse(String(describing: response))
            }
            let nextAgent = focusedAgent ?? agent
            markFocusedAgent(sessionID: session.id, agent: nextAgent)
            return "Focused \(nextAgent.displayName)."
        }
    }

    public func cycleAgentFocus() async {
        guard let session = selectedSession else {
            connectionState = .failed
            statusMessage = "Select a session before cycling focus."
            return
        }
        await perform("Cycling agent focus") {
            let response = try await client.send(
                .cycleAgentFocus(sessionID: session.id),
                to: try endpointURL()
            )
            guard case let .agentFocused(agent) = response else {
                throw KernelClientError.unexpectedResponse(String(describing: response))
            }
            markFocusedAgent(sessionID: session.id, agent: agent)
            if let agent {
                return "Focused \(agent.displayName)."
            }
            return "No agents available to focus."
        }
    }

    public func spawnAgent(alias: String? = nil, modelOverride: String? = nil) async {
        guard let session = selectedSession else {
            connectionState = .failed
            statusMessage = "Select a session before spawning an agent."
            return
        }
        let sourceAgent = session.focusedAgent ?? session.agents.first
        let provider = sourceAgent?.provider ?? selectedProviderID
        let model = modelOverride ?? sourceAgent?.model ?? selectedModelID
        let effort = sourceAgent?.effort ?? selectedVariantID
        let normalizedAlias = alias?.nilIfBlank

        await perform("Spawning agent") {
            let response = try await client.send(
                .spawnAgent(
                    sessionID: session.id,
                    alias: normalizedAlias,
                    provider: provider,
                    model: model,
                    effort: effort,
                    worktreeID: nil
                ),
                to: try endpointURL()
            )
            guard case let .agentSpawned(agent) = response else {
                throw KernelClientError.unexpectedResponse(String(describing: response))
            }
            upsert(agent: agent, inSessionID: session.id, focused: true)
            return "Spawned agent \(agent.displayName)."
        }
    }

    public func destroyAgent(_ agent: AgentInstance) async {
        guard let session = selectedSession else {
            connectionState = .failed
            statusMessage = "Select a session before destroying an agent."
            return
        }
        await perform("Destroying agent") {
            let response = try await client.send(
                .destroyAgent(sessionID: session.id, agentID: agent.id),
                to: try endpointURL()
            )
            guard case let .agentDestroyed(destroyedAgent) = response else {
                throw KernelClientError.unexpectedResponse(String(describing: response))
            }
            remove(agent: destroyedAgent, fromSessionID: session.id)
            return "Destroyed agent \(destroyedAgent.displayName)."
        }
    }

    public func setAgentExecutionMode(agent: AgentInstance, mode: AgentExecutionMode?) async {
        await updateAgentConfig(
            agent: agent,
            executionMode: mode?.rawValue,
            clearExecutionMode: mode == nil,
            permissionLevel: nil,
            clearPermissionLevel: false,
            successMessage: { updatedAgent in
                if let mode {
                    return "\(updatedAgent.displayName) mode = \(mode.rawValue)."
                }
                return "\(updatedAgent.displayName) mode inherits from session."
            }
        )
    }

    public func setAgentPermissionLevel(agent: AgentInstance, level: AgentPermissionLevel?) async {
        await updateAgentConfig(
            agent: agent,
            executionMode: nil,
            clearExecutionMode: false,
            permissionLevel: level?.rawValue,
            clearPermissionLevel: level == nil,
            successMessage: { updatedAgent in
                if let level {
                    return "\(updatedAgent.displayName) permissions = \(level.rawValue)."
                }
                return "\(updatedAgent.displayName) permissions inherit from session."
            }
        )
    }

    public func setSessionExecutionMode(_ mode: AgentExecutionMode) async {
        await updateSessionConfig(
            values: ["agents.mode": mode.rawValue],
            successMessage: "Session mode = \(mode.rawValue)."
        )
    }

    public func setSessionPermissionLevel(_ level: AgentPermissionLevel) async {
        await updateSessionConfig(
            values: ["agents.permissions": level.rawValue],
            successMessage: "Session permissions = \(level.rawValue)."
        )
    }

    public func selectSession(_ session: RuntimeSession) {
        upsert(session)
        selectedSessionID = session.id
        statusMessage = "Selected session \(session.shortDisplayID)."
    }

    public func transcriptEntries(for agent: AgentInstance?) -> [TranscriptEntry] {
        guard let agent else { return transcriptEntries }
        let scoped = transcriptEntries.filter { entry in
            entry.agentID == nil || entry.agentID == agent.id || entry.agentID == agent.agentRef
        }
        return scoped.isEmpty ? transcriptEntries.filter { $0.agentID == nil } : scoped
    }

    public func setResponseLayout(_ layout: MultiAgentResponseLayout) {
        responseLayout = layout
        saveDraftConfiguration()
        appendCommandNotice("View = \(layout.rawValue)")
        promptDraft = ""
    }

    public func selectProvider(_ providerID: String) {
        let normalized = providerID.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalized.isEmpty else {
            connectionState = .failed
            statusMessage = "usage: /provider <name>"
            return
        }
        selectedProviderID = normalized
        if let defaultModel = providerCatalog?.default[normalized] {
            selectedModelID = defaultModel
        }
        saveDraftConfiguration()
        appendCommandNotice("Provider = \(selectedProviderID)")
        promptDraft = ""
    }

    public func selectModel(_ modelID: String) {
        let normalized = modelID.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalized.isEmpty else {
            connectionState = .failed
            statusMessage = "usage: /model <id>"
            return
        }
        selectedModelID = normalized
        saveDraftConfiguration()
        appendCommandNotice("Model = \(selectedModelID)")
        promptDraft = ""
    }

    public func selectVariant(_ variantID: String) {
        let normalized = variantID.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalized.isEmpty else {
            connectionState = .failed
            statusMessage = "usage: /variant <name>"
            return
        }
        selectedVariantID = normalized
        saveDraftConfiguration()
        appendCommandNotice("Variant = \(selectedVariantID)")
        promptDraft = ""
    }

    public func cycleAgentFocusBackward() async {
        guard let session = selectedSession else {
            connectionState = .failed
            statusMessage = "Select a session before cycling focus."
            return
        }
        guard !session.agents.isEmpty else {
            connectionState = .failed
            statusMessage = "No agents available to focus."
            return
        }
        let currentIndex = session.agents.firstIndex { $0.id == session.focusedAgentID } ?? 0
        let previousIndex = currentIndex == 0 ? session.agents.count - 1 : currentIndex - 1
        await focusAgent(session.agents[previousIndex])
    }

    private func executeSlashCommand(_ rawCommand: String) async {
        let tokens = rawCommand
            .split(whereSeparator: { $0.isWhitespace })
            .map(String.init)
        guard let command = tokens.first else { return }
        switch command {
        case "/stop":
            await cancelActivePrompt()
            clearCommandDraftOnSuccess()
        case "/waiting":
            if activeAttachment != nil {
                await detachActiveSession()
                clearCommandDraftOnSuccess()
            } else {
                appendCommandNotice("Waiting room is visible.")
                promptDraft = ""
            }
        case "/session":
            await executeSessionCommand(Array(tokens.dropFirst()))
        case "/agent":
            await executeAgentCommand(Array(tokens.dropFirst()))
        case "/provider":
            await executeProviderCommand(Array(tokens.dropFirst()))
        case "/model":
            selectModel(tokens.dropFirst().joined(separator: " "))
        case "/variant":
            selectVariant(tokens.dropFirst().joined(separator: " "))
        case "/view":
            executeViewCommand(Array(tokens.dropFirst()))
        case "/mcp":
            await executeMcpCommand(Array(tokens.dropFirst()))
        case "/skill":
            await executeSkillCommand(Array(tokens.dropFirst()))
        case "/workspace":
            await executeWorkspaceCommand(Array(tokens.dropFirst()))
        case "/config":
            await executeConfigCommand(Array(tokens.dropFirst()))
        default:
            connectionState = .failed
            statusMessage = "Unsupported iOS command: \(command)."
        }
    }

    private func executeMcpCommand(_ args: [String]) async {
        let action = args.first ?? "list"
        switch action {
        case "list", "ls":
            await refreshMcpServers()
            if connectionState == .connected {
                appendCommandNotice(mcpListText())
                promptDraft = ""
            }
        default:
            connectionState = .failed
            statusMessage = "usage: /mcp list"
        }
    }

    private func executeSkillCommand(_ args: [String]) async {
        let action = args.first ?? "list"
        switch action {
        case "list", "ls":
            await refreshSkills()
            if connectionState == .connected {
                appendCommandNotice(skillListText())
                promptDraft = ""
            }
        default:
            connectionState = .failed
            statusMessage = "usage: /skill list"
        }
    }

    private func executeConfigCommand(_ args: [String]) async {
        let action = args.first ?? ""
        switch action {
        case "workspace-live-sync":
            let policy = args.dropFirst().first ?? "required"
            guard args.dropFirst(2).isEmpty,
                  policy == "required" || policy == "unrestricted"
            else {
                connectionState = .failed
                statusMessage = "usage: /config workspace-live-sync required|unrestricted"
                return
            }
            let mode = policy == "required" ? "managed" : "unrestricted"
            await setWorkspaceLiveSyncMode(
                mode,
                notice: "Workspace live sync set to \(policy)",
                status: "Workspace live sync set to \(policy)."
            )
        default:
            connectionState = .failed
            statusMessage = "usage: /config workspace-live-sync required|unrestricted"
        }
    }

    private func executeWorkspaceCommand(_ args: [String]) async {
        let action = args.first ?? "show"
        switch action {
        case "sync":
            await executeWorkspaceSyncCommand(Array(args.dropFirst()))
        case "show", "status":
            appendCommandNotice(workspaceText())
            promptDraft = ""
        case "path", "workspace":
            let nextPath = args.dropFirst().joined(separator: " ").nilIfBlank
            guard let nextPath else {
                connectionState = .failed
                statusMessage = "usage: /workspace path <path>"
                return
            }
            workspacePath = nextPath
            saveDraftConfiguration()
            appendCommandNotice("Workspace path = \(nextPath)")
            promptDraft = ""
        case "worktree":
            let nextPath = args.dropFirst().joined(separator: " ").nilIfBlank
            guard let nextPath else {
                connectionState = .failed
                statusMessage = "usage: /workspace worktree <path>"
                return
            }
            worktreePath = nextPath
            saveDraftConfiguration()
            appendCommandNotice("Worktree path = \(nextPath)")
            promptDraft = ""
        case "set":
            let nextPath = args.dropFirst().joined(separator: " ").nilIfBlank
            guard let nextPath else {
                connectionState = .failed
                statusMessage = "usage: /workspace set <path>"
                return
            }
            workspacePath = nextPath
            worktreePath = nextPath
            saveDraftConfiguration()
            appendCommandNotice("Workspace/worktree path = \(nextPath)")
            promptDraft = ""
        default:
            connectionState = .failed
            statusMessage = "usage: /workspace show|sync|path <path>|worktree <path>|set <path>"
        }
    }

    private func executeWorkspaceSyncCommand(_ args: [String]) async {
        let action = args.first ?? "status"
        switch action {
        case "status", "targets", "conflicts", "ignore":
            await showWorkspaceLiveSyncStatus(filter: action)
        case "enable":
            let mode = args.dropFirst().first ?? "managed"
            guard mode == "managed" || mode == "tracked" else {
                connectionState = .failed
                statusMessage = "usage: /workspace sync enable [managed|tracked]"
                return
            }
            await setWorkspaceLiveSyncMode(mode)
        case "disable":
            await setWorkspaceLiveSyncMode("unrestricted")
        case "mode":
            guard let mode = args.dropFirst().first,
                  ["managed", "tracked", "unrestricted"].contains(mode)
            else {
                connectionState = .failed
                statusMessage = "usage: /workspace sync mode managed|tracked|unrestricted"
                return
            }
            await setWorkspaceLiveSyncMode(mode)
        case "link":
            guard let linkRef = args.dropFirst().first?.nilIfBlank else {
                connectionState = .failed
                statusMessage = "usage: /workspace sync link <name-or-id> [repo-root]"
                return
            }
            let repoRoot = args.dropFirst(2).joined(separator: " ").nilIfBlank
            await attachWorkspaceLiveSyncLink(linkRef: linkRef, repoRoot: repoRoot)
        default:
            connectionState = .failed
            statusMessage = "usage: /workspace sync status|targets|conflicts|ignore|enable|disable|mode|link"
        }
    }

    private func showWorkspaceLiveSyncStatus(filter: String) async {
        guard let session = selectedSession else {
            connectionState = .failed
            statusMessage = "Select a session before inspecting workspace live sync."
            return
        }
        await perform("Loading workspace live sync") {
            let response = try await client.send(
                .getWorkspaceLiveSyncStatus(sessionID: session.id),
                to: try endpointURL()
            )
            guard case let .workspaceLiveSyncStatus(status) = response else {
                throw KernelClientError.unexpectedResponse(String(describing: response))
            }
            appendCommandNotice(workspaceLiveSyncText(status, filter: filter))
            promptDraft = ""
            return "Workspace live sync \(status.mode)."
        }
    }

    private func setWorkspaceLiveSyncMode(
        _ mode: String,
        notice: String? = nil,
        status: String? = nil
    ) async {
        await perform("Updating workspace live sync") {
            let response = try await client.send(
                .setWorkspaceLiveSyncMode(mode: mode),
                to: try endpointURL()
            )
            guard case .userConfigUpdated = response else {
                throw KernelClientError.unexpectedResponse(String(describing: response))
            }
            appendCommandNotice(notice ?? "Workspace live sync mode = \(mode)")
            promptDraft = ""
            return status ?? "Workspace live sync set to \(mode)."
        }
    }

    private func attachWorkspaceLiveSyncLink(linkRef: String, repoRoot: String?) async {
        guard let session = selectedSession else {
            connectionState = .failed
            statusMessage = "Select a session before linking workspace live sync."
            return
        }
        let targetRoot = repoRoot ?? session.worktreeID
        await perform("Linking workspace live sync") {
            let response = try await client.send(
                .attachWorkspaceLink(sessionID: session.id, linkRef: linkRef, repoRoot: targetRoot),
                to: try endpointURL()
            )
            guard case let .workspaceLinkAttached(updatedSession) = response else {
                throw KernelClientError.unexpectedResponse(String(describing: response))
            }
            upsert(updatedSession)
            selectedSessionID = updatedSession.id
            appendCommandNotice("Workspace live sync linked \(linkRef) -> \(targetRoot). Recommended mode: managed.")
            promptDraft = ""
            return "Workspace live sync linked. Recommended mode: managed."
        }
    }

    private func executeProviderCommand(_ args: [String]) async {
        let action = args.first ?? "list"
        switch action {
        case "list", "ls":
            await refreshProviderCatalog()
            if connectionState == .connected {
                appendCommandNotice(providerCatalogText())
                promptDraft = ""
            }
        case "status":
            if let provider = args.dropFirst().first {
                await refreshProviderAuthStatus(provider: provider)
                if connectionState == .connected {
                    appendCommandNotice(statusMessage)
                    promptDraft = ""
                }
            } else {
                await refreshProviderCatalog()
                if connectionState == .connected {
                    appendCommandNotice(providerCatalogText())
                    promptDraft = ""
                }
            }
        case "auth":
            let provider = args.dropFirst().first ?? providerCatalog?.connected.first ?? "codex"
            await refreshProviderAuthStatus(provider: provider)
            if connectionState == .connected {
                appendCommandNotice(statusMessage)
                promptDraft = ""
            }
        case "login":
            let provider = args.dropFirst().first ?? selectedProviderID
            await startProviderLogin(provider: provider)
            if connectionState == .connected {
                appendCommandNotice(statusMessage)
                promptDraft = ""
            }
        case "logout":
            let provider = args.dropFirst().first ?? selectedProviderID
            await logoutProvider(provider: provider)
            if connectionState == .connected {
                appendCommandNotice(statusMessage)
                promptDraft = ""
            }
        case "reauth":
            let provider = args.dropFirst().first ?? selectedProviderID
            await logoutProvider(provider: provider)
            guard connectionState == .connected else { return }
            await startProviderLogin(provider: provider)
            if connectionState == .connected {
                appendCommandNotice(statusMessage)
                promptDraft = ""
            }
        default:
            selectProvider(args.joined(separator: " "))
        }
    }

    private func executeViewCommand(_ args: [String]) {
        guard let rawValue = args.first?.lowercased(),
              let layout = MultiAgentResponseLayout(rawValue: rawValue)
        else {
            connectionState = .failed
            statusMessage = "usage: /view split|individual"
            return
        }
        setResponseLayout(layout)
    }

    private func executeSessionCommand(_ args: [String]) async {
        let action = args.first ?? "list"
        switch action {
        case "list", "ls":
            await refreshSessions()
            if connectionState == .connected {
                appendCommandNotice(sessionListText())
                promptDraft = ""
            }
        case "new", "create":
            if args.count > 1 {
                let path = args.dropFirst().joined(separator: " ")
                workspacePath = path
                worktreePath = path
            }
            await createSession()
            if connectionState == .connected {
                await attachSelectedSession()
            }
            clearCommandDraftOnSuccess()
        case "attach":
            if args.count > 1 {
                let reference = args.dropFirst().joined(separator: " ")
                guard let session = resolveSession(reference: reference) else {
                    connectionState = .failed
                    statusMessage = "Session `\(reference)` was not found."
                    return
                }
                selectSession(session)
            }
            await attachSelectedSession()
            clearCommandDraftOnSuccess()
        case "detach":
            await detachActiveSession()
            clearCommandDraftOnSuccess()
        case "delete", "destroy":
            let reference = args.dropFirst().joined(separator: " ").nilIfBlank
            await deleteSession(reference: reference)
            clearCommandDraftOnSuccess()
        case "mode":
            await executeSessionModeCommand(Array(args.dropFirst()))
        case "permissions":
            await executeSessionPermissionsCommand(Array(args.dropFirst()))
        default:
            connectionState = .failed
            statusMessage = "usage: /session new|create|list|attach [session-ref]|detach|mode [build|plan]|permissions [required|yolo]"
        }
    }

    private func executeSessionModeCommand(_ args: [String]) async {
        if args.isEmpty {
            await appendSessionConfigNotice(
                key: "agents.mode",
                fallback: "build",
                label: "session mode"
            )
            return
        }
        guard args.count == 1, let mode = AgentExecutionMode(rawValue: args[0].lowercased()) else {
            connectionState = .failed
            statusMessage = "usage: /session mode [build|plan]"
            return
        }
        await setSessionExecutionMode(mode)
        clearCommandDraftOnSuccess()
    }

    private func executeSessionPermissionsCommand(_ args: [String]) async {
        if args.isEmpty {
            await appendSessionConfigNotice(
                key: "agents.permissions",
                fallback: "yolo",
                label: "session permissions"
            )
            return
        }
        guard args.count == 1, let level = AgentPermissionLevel(rawValue: args[0].lowercased()) else {
            connectionState = .failed
            statusMessage = "usage: /session permissions [required|yolo]"
            return
        }
        await setSessionPermissionLevel(level)
        clearCommandDraftOnSuccess()
    }

    private func executeAgentCommand(_ args: [String]) async {
        let action = args.first ?? "list"
        switch action {
        case "list", "ls":
            guard let session = selectedSession else {
                connectionState = .failed
                statusMessage = "Select a session before listing agents."
                return
            }
            appendCommandNotice(agentListText(session: session))
            promptDraft = ""
        case "focus":
            guard let reference = args.dropFirst().first else {
                connectionState = .failed
                statusMessage = "usage: /agent focus <agent-id>"
                return
            }
            guard let session = selectedSession else {
                connectionState = .failed
                statusMessage = "Select a session before focusing an agent."
                return
            }
            guard let agent = resolveAgent(reference: reference, in: session) else {
                connectionState = .failed
                statusMessage = "Agent `\(reference)` was not found."
                return
            }
            await focusAgent(agent)
            clearCommandDraftOnSuccess()
        case "cycle":
            await cycleAgentFocus()
            clearCommandDraftOnSuccess()
        case "spawn":
            let spawnArgs = Array(args.dropFirst())
            let alias = spawnArgs.first
            let model = spawnArgs.dropFirst().first
            if spawnArgs.count > 2 {
                connectionState = .failed
                statusMessage = "usage: /agent spawn [alias] [model]"
                return
            }
            await spawnAgent(alias: alias, modelOverride: model)
            clearCommandDraftOnSuccess()
        case "destroy", "delete":
            guard let session = selectedSession else {
                connectionState = .failed
                statusMessage = "Select a session before destroying an agent."
                return
            }
            let reference = args.dropFirst().first
            let target = reference.flatMap { resolveAgent(reference: $0, in: session) } ?? session.focusedAgent
            guard let target else {
                connectionState = .failed
                statusMessage = "usage: /agent destroy <agent-id>"
                return
            }
            await destroyAgent(target)
            clearCommandDraftOnSuccess()
        case "mode":
            await executeAgentModeCommand(Array(args.dropFirst()))
        case "permissions":
            await executeAgentPermissionsCommand(Array(args.dropFirst()))
        default:
            connectionState = .failed
            statusMessage = "usage: /agent list|spawn [alias] [model]|destroy [agent-id]|focus <agent-id>|cycle|mode|permissions"
        }
    }

    private func executeAgentModeCommand(_ args: [String]) async {
        guard let parsed = parseAgentConfigCommand(
            args,
            usage: "usage: /agent mode [agent-id] <build|plan|inherit>"
        ) else {
            return
        }
        guard parsed.value == "inherit" || AgentExecutionMode(rawValue: parsed.value) != nil else {
            connectionState = .failed
            statusMessage = "usage: /agent mode [agent-id] <build|plan|inherit>"
            return
        }
        await setAgentExecutionMode(
            agent: parsed.agent,
            mode: parsed.value == "inherit" ? nil : AgentExecutionMode(rawValue: parsed.value)
        )
        clearCommandDraftOnSuccess()
    }

    private func executeAgentPermissionsCommand(_ args: [String]) async {
        guard let parsed = parseAgentConfigCommand(
            args,
            usage: "usage: /agent permissions [agent-id] <required|yolo|inherit>"
        ) else {
            return
        }
        guard parsed.value == "inherit" || AgentPermissionLevel(rawValue: parsed.value) != nil else {
            connectionState = .failed
            statusMessage = "usage: /agent permissions [agent-id] <required|yolo|inherit>"
            return
        }
        await setAgentPermissionLevel(
            agent: parsed.agent,
            level: parsed.value == "inherit" ? nil : AgentPermissionLevel(rawValue: parsed.value)
        )
        clearCommandDraftOnSuccess()
    }

    private func perform(_ label: String, operation: () async throws -> String) async {
        saveDraftConfiguration()
        connectionState = .working(label)
        do {
            statusMessage = try await operation()
            connectionState = .connected
        } catch {
            connectionState = .failed
            statusMessage = Self.describe(error)
        }
    }

    private func endpointURL() throws -> URL {
        guard let url = URL(string: kernelURLText.trimmingCharacters(in: .whitespacesAndNewlines)),
              url.scheme == "ws" || url.scheme == "wss"
        else {
            throw KernelClientError.invalidEndpoint
        }
        return url
    }

    private static func describe(_ error: Error) -> String {
        if let transportError = error as? KernelTransportError {
            return "\(transportError.code): \(transportError.message)"
        }
        if let clientError = error as? KernelClientError {
            switch clientError {
            case .invalidEndpoint:
                return "Kernel URL must start with ws:// or wss://."
            case .invalidRequestEncoding:
                return "Could not encode kernel request."
            case .unsupportedMessage:
                return "Kernel returned an unsupported WebSocket message."
            case .emptyResponse:
                return "Kernel response was empty."
            case let .unexpectedResponse(name):
                return "Unexpected kernel response: \(name)."
            }
        }
        return error.localizedDescription
    }

    private func startEventStream(
        sessionID: String,
        attachmentID: String,
        endpoint: URL,
        resumeFromEventID: Int64?
    ) {
        stopEventStream(resetCursor: false)
        eventStreamStartedAt = Date()
        eventStreamState = .connecting
        startHeartbeatMonitor()
        eventTask = Task { @MainActor [weak self] in
            guard let self else { return }
            var nextResumeFromEventID = resumeFromEventID
            while !Task.isCancelled {
                eventStreamState = .connecting
                let currentStream = client.events(
                    sessionID: sessionID,
                    attachmentID: attachmentID,
                    endpoint: endpoint,
                    resumeFromEventID: nextResumeFromEventID
                )
                do {
                    for try await frame in currentStream {
                        handle(eventFrame: frame)
                        nextResumeFromEventID = lastEventID
                    }
                    if !Task.isCancelled {
                        eventStreamState = .disconnected
                    }
                    return
                } catch is CancellationError {
                    return
                } catch {
                    eventStreamState = .disconnected
                    if case .working = connectionState {
                        // Keep the active command's outcome visible; the stream loop will keep retrying.
                    } else {
                        statusMessage = "Event stream interrupted. Reconnecting with replay cursor."
                    }
                    try? await Task.sleep(for: .seconds(1))
                    nextResumeFromEventID = lastEventID
                }
            }
        }
    }

    func evaluateHeartbeatStaleness(now: Date = Date()) {
        guard heartbeatStaleAfterSeconds > 0,
              activeAttachment != nil,
              subscribedSessionID != nil,
              eventStreamState == .live || eventStreamState == .connecting
        else {
            return
        }
        guard let referenceDate = lastHeartbeatAt ?? eventStreamStartedAt else {
            return
        }
        let elapsed = now.timeIntervalSince(referenceDate)
        guard elapsed >= heartbeatStaleAfterSeconds else {
            return
        }
        eventStreamState = .stale
        statusMessage = "No kernel heartbeat for \(Int(elapsed.rounded()))s. Waiting for stream recovery."
    }

    private func startHeartbeatMonitor() {
        heartbeatMonitorTask?.cancel()
        let interval = max(0.25, min(heartbeatStaleAfterSeconds / 3, 5))
        let sleepNanoseconds = UInt64(interval * 1_000_000_000)
        heartbeatMonitorTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: sleepNanoseconds)
                guard !Task.isCancelled else { return }
                self?.evaluateHeartbeatStaleness()
            }
        }
    }

    private func stopEventStream(resetCursor: Bool) {
        eventTask?.cancel()
        eventTask = nil
        heartbeatMonitorTask?.cancel()
        heartbeatMonitorTask = nil
        eventStreamStartedAt = nil
        if resetCursor {
            lastEventID = nil
            lastHeartbeatAt = nil
        }
    }

    private func handle(eventFrame: KernelEventFrame) {
        lastEventID = eventFrame.eventID
        switch eventFrame.event {
        case let .terminalOutput(records):
            appendTerminalOutput(records)
        case let .runtimeNotices(notices):
            for notice in notices {
                appendTranscript(kind: .notice, agentID: nil, text: notice.message)
            }
        case let .assistantMessageCompleted(_, _, agentID, messageID, _):
            appendTranscript(
                kind: .completion,
                agentID: agentID,
                text: "assistant message completed: \(messageID)"
            )
        case let .sessionSnapshot(session):
            upsert(session)
            selectedSessionID = session.id
            eventStreamState = .live
            statusMessage = "Session snapshot received."
        case let .heartbeat(sessionID):
            if sessionID == subscribedSessionID {
                lastHeartbeatAt = Date()
                eventStreamState = .live
            }
        case let .sessionUnavailable(sessionID, message):
            if selectedSessionID == sessionID {
                eventStreamState = .failed
                statusMessage = message
            }
        case let .transportResumed(sessionID, _):
            if sessionID == subscribedSessionID {
                eventStreamState = .live
                lastHeartbeatAt = Date()
                statusMessage = "Kernel event stream resumed."
            }
        case let .replayGap(gap):
            eventStreamState = .failed
            statusMessage = gap.message ?? "Replay gap detected. Refresh sessions before continuing."
        case let .unknown(name):
            statusMessage = "Kernel event received: \(name)."
        }
    }

    private func appendTerminalOutput(_ records: [TerminalOutputRecord]) {
        var pendingKind: TranscriptEntry.Kind?
        var pendingAgentID: String?
        var pendingText = ""

        func flushPending() {
            guard let kind = pendingKind, !pendingText.isEmpty else { return }
            appendTranscript(kind: kind, agentID: pendingAgentID, text: pendingText)
            pendingKind = nil
            pendingAgentID = nil
            pendingText = ""
        }

        for record in records where !record.text.isEmpty {
            let kind = TranscriptEntry.Kind(terminalKind: record.kind)
            if pendingKind == kind, pendingAgentID == record.agentID {
                pendingText += record.text
            } else {
                flushPending()
                pendingKind = kind
                pendingAgentID = record.agentID
                pendingText = record.text
            }
        }
        flushPending()
    }

    private func appendTranscript(kind: TranscriptEntry.Kind, agentID: String?, text: String) {
        guard !text.isEmpty else { return }
        if kind.mergesAdjacentOutput,
           let lastIndex = transcriptEntries.indices.last,
           transcriptEntries[lastIndex].kind == kind,
           transcriptEntries[lastIndex].agentID == agentID
        {
            transcriptEntries[lastIndex].text += text
            return
        }
        transcriptEntries.append(
            TranscriptEntry(kind: kind, agentID: agentID, text: text)
        )
        if transcriptEntries.count > 500 {
            transcriptEntries.removeFirst(transcriptEntries.count - 500)
        }
    }

    private func appendCommandNotice(_ message: String) {
        statusMessage = message
        appendTranscript(kind: .notice, agentID: nil, text: message)
    }

    private func clearCommandDraftOnSuccess() {
        if connectionState == .connected || connectionState == .idle {
            promptDraft = ""
        }
    }

    private func sessionListText() -> String {
        guard !sessions.isEmpty else {
            return "No sessions found."
        }
        let lines = sessions.map { session in
            let selected = selectedSessionID == session.id ? "*" : "-"
            return "\(selected) \(session.shortDisplayID) \(session.status) \(session.worktreeID)"
        }
        return "Sessions\n\(lines.joined(separator: "\n"))"
    }

    private func agentListText(session: RuntimeSession) -> String {
        guard !session.agents.isEmpty else {
            return "No agents reported for \(session.shortDisplayID)."
        }
        let lines = session.agents.map { agent in
            let selected = session.focusedAgentID == agent.id ? "*" : "-"
            return "\(selected) \(agent.displayName) \(agent.providerModelText) \(agent.state)"
        }
        return "Agents\n\(lines.joined(separator: "\n"))"
    }

    private func providerCatalogText() -> String {
        guard let providerCatalog else {
            return "Provider catalog has not been loaded."
        }
        if providerCatalog.all.isEmpty {
            return "No providers reported by the kernel."
        }
        let connected = Set(providerCatalog.connected)
        let lines = providerCatalog.all.map { provider in
            let marker = connected.contains(provider.id) ? "*" : "-"
            let defaultModel = providerCatalog.default[provider.id].map { " default=\($0)" } ?? ""
            return "\(marker) \(provider.name) (\(provider.id)) \(provider.models.count) model\(provider.models.count == 1 ? "" : "s")\(defaultModel)"
        }
        return "Providers\n\(lines.joined(separator: "\n"))"
    }

    private func providerAuthStatusText(_ status: ProviderAuthStatus) -> String {
        [
            status.accountProfile.map { "\(status.provider): \(status.authState) as \($0)" }
                ?? "\(status.provider): \(status.authState)",
            status.detectedVersion.map { "version \($0)" },
            status.loginHint,
        ]
        .compactMap { $0?.nilIfBlank }
        .joined(separator: " • ")
    }

    private func providerLoginText(_ login: ProviderLoginStart) -> String {
        [
            "\(login.provider): \(login.loginKind)",
            login.userCode.map { "code \($0)" },
            login.verificationURL ?? login.authURL,
        ]
        .compactMap { $0?.nilIfBlank }
        .joined(separator: " • ")
    }

    private func mcpListText() -> String {
        guard !mcpServers.isEmpty else {
            return "No MCP servers installed."
        }
        let lines = mcpServers.map { mcp in
            let enabled = mcp.enabled == false ? "disabled" : "enabled"
            let transport = mcp.transport.keys.sorted().first ?? "transport"
            return "\(mcp.name) [\(enabled)] \(transport)"
        }
        return "MCP servers\n\(lines.joined(separator: "\n"))"
    }

    private func skillListText() -> String {
        guard !skills.isEmpty else {
            return "No skills installed."
        }
        let lines = skills.map { skill in
            let description = skill.shortDescription?.nilIfBlank ?? skill.description.nilIfBlank ?? skill.path
            return "\(skill.name) - \(description)"
        }
        return "Skills\n\(lines.joined(separator: "\n"))"
    }

    private func workspaceText() -> String {
        "Workspace\nworkspace: \(workspacePath)\nworktree: \(worktreePath)"
    }

    private func workspaceLiveSyncText(_ status: WorkspaceLiveSyncStatus, filter: String) -> String {
        let header = "Workspace live sync\nmode: \(status.mode)\nstate: \(status.footerState)"
        switch filter {
        case "targets":
            guard !status.targets.isEmpty else { return "\(header)\ntargets: none" }
            let lines = status.targets.map { target in
                let branch = target.branch.map { " @ \($0)" } ?? ""
                return "- \(target.linkName) \(target.status) \(target.repoRoot)\(branch)"
            }
            return "\(header)\ntargets\n\(lines.joined(separator: "\n"))"
        case "conflicts":
            guard !status.conflicts.isEmpty else { return "\(header)\nconflicts: none" }
            let lines = status.conflicts.map { conflict in
                "- \(conflict.path) -> \(conflict.targetUserID): \(conflict.nextAction)"
            }
            return "\(header)\nconflicts\n\(lines.joined(separator: "\n"))"
        case "ignore":
            let ignoreFile = status.ignore.ignoreFile ?? ".arrobaignore"
            let excludes = status.ignore.forceExcludes.isEmpty
                ? "none"
                : status.ignore.forceExcludes.joined(separator: ", ")
            return "\(header)\nignore: \(ignoreFile)\nforce excludes: \(excludes)"
        default:
            return "\(header)\ntargets: \(status.targets.count)\nconflicts: \(status.conflicts.count)\nignore: \(status.ignore.ignoreFile ?? ".arrobaignore")"
        }
    }

    private func resolveSession(reference: String) -> RuntimeSession? {
        let normalized = reference.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !normalized.isEmpty else { return selectedSession }
        return sessions.first { session in
            session.id.lowercased() == normalized
                || session.id.lowercased().hasPrefix(normalized)
                || session.alias?.lowercased() == normalized
                || session.shortDisplayID.lowercased() == normalized
        }
    }

    private func resolveAgent(reference: String, in session: RuntimeSession) -> AgentInstance? {
        let normalized = reference.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !normalized.isEmpty else { return session.focusedAgent }
        return session.agents.first { agent in
            agent.id.lowercased() == normalized
                || agent.id.lowercased().hasPrefix(normalized)
                || agent.agentRef.lowercased() == normalized
                || agent.alias?.lowercased() == normalized
        }
    }

    private func parseAgentConfigCommand(_ args: [String], usage: String) -> (agent: AgentInstance, value: String)? {
        guard let session = selectedSession else {
            connectionState = .failed
            statusMessage = "Select a session before configuring an agent."
            return nil
        }
        guard !args.isEmpty, args.count <= 2 else {
            connectionState = .failed
            statusMessage = usage
            return nil
        }
        let value: String
        let agent: AgentInstance?
        if args.count == 1 {
            value = args[0]
            agent = session.focusedAgent
        } else {
            agent = resolveAgent(reference: args[0], in: session)
            value = args[1]
        }
        guard let agent else {
            connectionState = .failed
            statusMessage = args.count == 1 ? "No focused agent to configure." : "Agent `\(args[0])` was not found."
            return nil
        }
        return (agent, value.lowercased())
    }

    private func appendSessionConfigNotice(key: String, fallback: String, label: String) async {
        guard selectedSession != nil else {
            connectionState = .failed
            statusMessage = "Select a session before reading session config."
            return
        }
        if selectedSession?.configState?.values[key] == nil {
            await refreshSelectedSessionState()
        }
        guard connectionState == .connected || connectionState == .idle else {
            return
        }
        let value = selectedSession?.configState?.values[key] ?? fallback
        appendCommandNotice("\(label) = \(value)")
        promptDraft = ""
    }

    private func loadRecentHistory(endpoint: URL, session: RuntimeSession) async {
        do {
            let response = try await client.send(
                .getSessionHistory(
                    sessionID: session.id,
                    agentID: session.focusedAgentID,
                    roundCount: 8,
                    maxChars: 80_000
                ),
                to: endpoint
            )
            guard case let .sessionHistory(entries) = response else {
                return
            }
            transcriptEntries = []
            for entry in entries {
                appendTranscript(
                    kind: TranscriptEntry.Kind(historyKind: entry.entry.kind),
                    agentID: entry.entry.agentID,
                    text: entry.entry.text
                )
            }
        } catch {
            appendTranscript(
                kind: .notice,
                agentID: nil,
                text: "Could not load recent history: \(Self.describe(error))"
            )
        }
    }

    private func upsert(_ session: RuntimeSession) {
        if let index = sessions.firstIndex(where: { $0.id == session.id }) {
            sessions[index] = session
        } else {
            sessions.insert(session, at: 0)
        }
    }

    private func upsert(agent: AgentInstance, inSessionID sessionID: String, focused: Bool) {
        guard let index = sessions.firstIndex(where: { $0.id == sessionID }) else {
            return
        }
        let session = sessions[index]
        var agents = session.agents.map { existing in
            if existing.id == agent.id {
                return agent
            }
            return focused ? existing.withState("Idle") : existing
        }
        if !agents.contains(where: { $0.id == agent.id }) {
            agents.append(focused ? agent.withState("Focused") : agent)
        }
        sessions[index] = session.replacingAgents(agents, focusedAgentID: focused ? agent.id : session.focusedAgentID)
        selectedSessionID = sessionID
    }

    private func remove(agent: AgentInstance, fromSessionID sessionID: String) {
        guard let index = sessions.firstIndex(where: { $0.id == sessionID }) else {
            return
        }
        let session = sessions[index]
        var agents = session.agents.filter { $0.id != agent.id }
        let nextFocusedAgentID: String?
        if session.focusedAgentID == agent.id {
            nextFocusedAgentID = agents.first?.id
            if let nextFocusedAgentID {
                agents = agents.map { $0.id == nextFocusedAgentID ? $0.withState("Focused") : $0 }
            }
        } else {
            nextFocusedAgentID = session.focusedAgentID
        }
        sessions[index] = session.replacingAgents(agents, focusedAgentID: nextFocusedAgentID)
        selectedSessionID = sessionID
    }

    private func updateAgentConfig(
        agent: AgentInstance,
        executionMode: String?,
        clearExecutionMode: Bool,
        permissionLevel: String?,
        clearPermissionLevel: Bool,
        successMessage: (AgentInstance) -> String
    ) async {
        guard let session = selectedSession else {
            connectionState = .failed
            statusMessage = "Select a session before configuring an agent."
            return
        }
        await perform("Updating agent") {
            let response = try await client.send(
                .updateAgentConfig(
                    sessionID: session.id,
                    agentID: agent.id,
                    executionMode: executionMode,
                    clearExecutionMode: clearExecutionMode,
                    permissionLevel: permissionLevel,
                    clearPermissionLevel: clearPermissionLevel
                ),
                to: try endpointURL()
            )
            guard case let .agentConfigUpdated(updatedAgent, updatedSession) = response else {
                throw KernelClientError.unexpectedResponse(String(describing: response))
            }
            upsert(updatedSession)
            selectedSessionID = updatedSession.id
            return successMessage(updatedAgent)
        }
    }

    private func updateSessionConfig(values: [String: String], successMessage: String) async {
        guard let session = selectedSession, let attachment = activeAttachment else {
            connectionState = .failed
            statusMessage = "Attach to a session before updating session config."
            return
        }
        await perform("Updating session") {
            let response = try await client.send(
                .updateSessionConfig(
                    sessionID: session.id,
                    attachmentID: attachment.id,
                    values: values,
                    requiresIdle: false
                ),
                to: try endpointURL()
            )
            guard case let .sessionConfigUpdated(updatedSession, _) = response else {
                throw KernelClientError.unexpectedResponse(String(describing: response))
            }
            upsert(updatedSession)
            selectedSessionID = updatedSession.id
            return successMessage
        }
    }

    private func markFocusedAgent(sessionID: String, agent: AgentInstance?) {
        guard let index = sessions.firstIndex(where: { $0.id == sessionID }) else {
            return
        }
        let session = sessions[index]
        let agents = session.agents.map { existing in
            if existing.id == agent?.id, let agent {
                return agent
            }
            return existing
        }
        sessions[index] = RuntimeSession(
            id: session.id,
            alias: session.alias,
            workspaceID: session.workspaceID,
            worktreeID: session.worktreeID,
            status: session.status,
            configState: session.configState,
            activePrompt: session.activePrompt,
            queuedPrompts: session.queuedPrompts,
            promptStates: session.promptStates,
            activeInteractions: session.activeInteractions,
            focusedAgentID: agent?.id,
            agents: agents,
            createdAtMs: session.createdAtMs,
            lastUsedAtMs: session.lastUsedAtMs
        )
        selectedSessionID = sessionID
    }

    private static func loadClientID(from defaults: UserDefaults) -> String {
        if let existing = defaults.string(forKey: DefaultsKey.clientID), !existing.isEmpty {
            return existing
        }
        let next = "arroba-ios-\(UUID().uuidString)"
        defaults.set(next, forKey: DefaultsKey.clientID)
        return next
    }

    private static func defaultWorkspacePath() -> String {
        ProcessInfo.processInfo.environment["ARROBA_IOS_DEFAULT_WORKSPACE"]?.nilIfBlank ?? ""
    }
}

public enum AgentExecutionMode: String, CaseIterable, Equatable, Sendable {
    case build
    case plan
}

public enum AgentPermissionLevel: String, CaseIterable, Equatable, Sendable {
    case required
    case yolo
}

public enum MultiAgentResponseLayout: String, CaseIterable, Equatable, Sendable {
    case individual
    case split
}

public struct TranscriptEntry: Identifiable, Equatable, Sendable {
    public let id: UUID
    public let kind: Kind
    public let agentID: String?
    public var text: String
    public let createdAt: Date

    public init(
        id: UUID = UUID(),
        kind: Kind,
        agentID: String?,
        text: String,
        createdAt: Date = Date()
    ) {
        self.id = id
        self.kind = kind
        self.agentID = agentID
        self.text = text
        self.createdAt = createdAt
    }

    public enum Kind: String, Equatable, Sendable {
        case prompt
        case output
        case reasoning
        case tool
        case error
        case status
        case notice
        case completion

        init(terminalKind: String) {
            switch terminalKind {
            case "prompt_echo":
                self = .prompt
            case "provider_reasoning":
                self = .reasoning
            case "provider_tool":
                self = .tool
            case "provider_error":
                self = .error
            case "provider_status":
                self = .status
            default:
                self = .output
            }
        }

        init(historyKind: String) {
            switch historyKind {
            case "user_prompt":
                self = .prompt
            case "provider_reasoning":
                self = .reasoning
            case "provider_tool":
                self = .tool
            case "provider_error":
                self = .error
            case "provider_status":
                self = .status
            case "notice":
                self = .notice
            default:
                self = .output
            }
        }

        var mergesAdjacentOutput: Bool {
            switch self {
            case .prompt, .output, .reasoning, .tool:
                true
            case .error, .status, .notice, .completion:
                false
            }
        }
    }
}

public enum ConnectionState: Equatable, Sendable {
    case idle
    case working(String)
    case connected
    case failed

    public var label: String {
        switch self {
        case .idle:
            "IDLE"
        case let .working(label):
            label.uppercased()
        case .connected:
            "CONNECTED"
        case .failed:
            "ERROR"
        }
    }
}

public enum EventStreamState: Equatable, Sendable {
    case idle
    case connecting
    case live
    case stale
    case disconnected
    case failed

    public var label: String {
        switch self {
        case .idle:
            "DETACHED"
        case .connecting:
            "SUBSCRIBING"
        case .live:
            "LIVE"
        case .stale:
            "STALE"
        case .disconnected:
            "DISCONNECTED"
        case .failed:
            "STREAM ERROR"
        }
    }
}

public extension RuntimeSession {
    var shortDisplayID: String {
        alias ?? String(id.prefix(10))
    }

    var activeAgentCountText: String {
        "\(agents.count) agent\(agents.count == 1 ? "" : "s")"
    }

    var focusedAgent: AgentInstance? {
        guard let focusedAgentID else { return nil }
        return agents.first { $0.id == focusedAgentID }
    }

    var focusedPromptState: AgentPromptState? {
        guard let focusedAgentID else { return nil }
        return promptStates[focusedAgentID]
    }

    var focusedPromptActivityText: String {
        let active = focusedPromptState?.activePrompt ?? activePrompt
        let queuedCount = focusedPromptState?.queuedPrompts.count ?? queuedPrompts.count
        if active != nil, queuedCount > 0 {
            return "active + \(queuedCount) queued"
        }
        if active != nil {
            return "active"
        }
        if queuedCount > 0 {
            return "\(queuedCount) queued"
        }
        return "idle"
    }

    func replacingAgents(_ agents: [AgentInstance], focusedAgentID: String?) -> RuntimeSession {
        RuntimeSession(
            id: id,
            alias: alias,
            workspaceID: workspaceID,
            worktreeID: worktreeID,
            status: status,
            configState: configState,
            activePrompt: activePrompt,
            queuedPrompts: queuedPrompts,
            promptStates: promptStates,
            activeInteractions: activeInteractions,
            focusedAgentID: focusedAgentID,
            agents: agents,
            createdAtMs: createdAtMs,
            lastUsedAtMs: lastUsedAtMs
        )
    }
}

public extension AgentInstance {
    var displayName: String {
        alias ?? agentRef
    }

    var providerModelText: String {
        if let model, !model.isEmpty {
            return "\(provider) / \(model)"
        }
        return provider
    }

    var executionModeText: String {
        executionModeOverride ?? "inherit"
    }

    var permissionLevelText: String {
        permissionLevelOverride ?? "inherit"
    }

    func withState(_ state: String) -> AgentInstance {
        AgentInstance(
            id: id,
            agentRef: agentRef,
            sessionID: sessionID,
            alias: alias,
            provider: provider,
            model: model,
            effort: effort,
            executionModeOverride: executionModeOverride,
            permissionLevelOverride: permissionLevelOverride,
            worktreeID: worktreeID,
            state: state,
            isProcessing: isProcessing
        )
    }
}

public extension RuntimeInteraction {
    var displayTitle: String {
        title ?? (kind == "permission" ? "Permission request" : "Action required")
    }

    var levelText: String {
        level.uppercased()
    }
}

private extension String {
    var nilIfBlank: String? {
        let trimmed = trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }
}

private enum DefaultsKey {
    static let kernelURL = "dev.arroba.ios.kernelURL"
    static let workspacePath = "dev.arroba.ios.workspacePath"
    static let worktreePath = "dev.arroba.ios.worktreePath"
    static let providerID = "dev.arroba.ios.providerID"
    static let modelID = "dev.arroba.ios.modelID"
    static let variantID = "dev.arroba.ios.variantID"
    static let responseLayout = "dev.arroba.ios.responseLayout"
    static let clientID = "dev.arroba.ios.clientID"
}
