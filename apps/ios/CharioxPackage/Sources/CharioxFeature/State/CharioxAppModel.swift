import Foundation
import Observation

@MainActor
@Observable
public final class CharioxAppModel {
  public var kernelURLText: String
  public var workspacePath: String
  public var worktreePath: String
  public var promptDraft: String
  public var selectedProviderID: String
  public var selectedModelID: String
  public var selectedVariantID: String
  public var responseLayout: MultiAgentResponseLayout
  public internal(set) var sessions: [RuntimeSession]
  public internal(set) var selectedSessionID: String?
  public internal(set) var activeAttachment: RuntimeAttachment?
  public internal(set) var subscribedSessionID: String?
  public internal(set) var lastEventID: Int64?
  public internal(set) var lastHeartbeatAt: Date?
  public internal(set) var transcriptEntries: [TranscriptEntry]
  public internal(set) var providerCatalog: ProviderCatalog?
  public internal(set) var providerAuthStatuses: [String: ProviderAuthStatus]
  public internal(set) var mcpServers: [CharioxMcpServerConfig]
  public internal(set) var skills: [CharioxSkillMetadata]
  public internal(set) var eventStreamState: EventStreamState
  public internal(set) var connectionState: ConnectionState
  public internal(set) var statusMessage: String

  let client: KernelClientProtocol
  let defaults: UserDefaults
  let clientID: String
  let heartbeatStaleAfterSeconds: TimeInterval

  @ObservationIgnored
  var eventTask: Task<Void, Never>?

  @ObservationIgnored
  var heartbeatMonitorTask: Task<Void, Never>?

  @ObservationIgnored
  var eventStreamStartedAt: Date?

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
    responseLayout =
      MultiAgentResponseLayout(
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
      guard case .sessionsListed(let nextSessions) = response else {
        throw KernelClientError.unexpectedResponse(String(describing: response))
      }
      sessions = nextSessions.sorted { lhs, rhs in
        (lhs.lastUsedAtMs ?? lhs.createdAtMs) > (rhs.lastUsedAtMs ?? rhs.createdAtMs)
      }
      if let selectedSessionID, sessions.contains(where: { $0.id == selectedSessionID }) {
        return "Loaded \(sessions.count) session\(sessions.count == 1 ? "" : "s")."
      }
      selectedSessionID = sessions.first?.id
      return sessions.isEmpty
        ? "Kernel connected. No sessions yet." : "Kernel connected. Selected latest session."
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
      guard case .sessionCreated(let session) = response else {
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
      guard case .sessionDeleted(let session) = response else {
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
      guard case .sessionState(let updatedSession) = response else {
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
      guard case .providerCatalog(let catalog) = response else {
        throw KernelClientError.unexpectedResponse(String(describing: response))
      }
      providerCatalog = catalog
      return
        "Loaded \(catalog.connected.count) connected provider\(catalog.connected.count == 1 ? "" : "s")."
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
      guard case .providerAuthStatus(let status) = response else {
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
      guard case .providerLoginStarted(let login) = response else {
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
      guard case .providerLoggedOut(let provider) = response else {
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
      guard case .mcpServersListed(let mcps) = response else {
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
      guard case .skillsListed(let nextSkills) = response else {
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
      guard case .sessionAttached(let attachment) = response else {
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
      guard case .promptSubmitted(let updatedSession) = response else {
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

  public func respondToInteraction(
    _ interaction: RuntimeInteraction, choice: RuntimeInteractionChoice
  ) async {
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
      guard case .interactionResponded(_, let updatedSession) = response else {
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
      guard case .agentFocused(let focusedAgent) = response else {
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
      guard case .agentFocused(let agent) = response else {
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
      guard case .agentSpawned(let agent) = response else {
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
      guard case .agentDestroyed(let destroyedAgent) = response else {
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
    case .working(let label):
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

extension RuntimeSession {
  public var shortDisplayID: String {
    alias ?? String(id.prefix(10))
  }

  public var activeAgentCountText: String {
    "\(agents.count) agent\(agents.count == 1 ? "" : "s")"
  }

  public var focusedAgent: AgentInstance? {
    guard let focusedAgentID else { return nil }
    return agents.first { $0.id == focusedAgentID }
  }

  public var focusedPromptState: AgentPromptState? {
    guard let focusedAgentID else { return nil }
    return promptStates[focusedAgentID]
  }

  public var focusedPromptActivityText: String {
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

  public func replacingAgents(_ agents: [AgentInstance], focusedAgentID: String?) -> RuntimeSession
  {
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

extension AgentInstance {
  public var displayName: String {
    alias ?? agentRef
  }

  public var providerModelText: String {
    if let model, !model.isEmpty {
      return "\(provider) / \(model)"
    }
    return provider
  }

  public var executionModeText: String {
    executionModeOverride ?? "inherit"
  }

  public var permissionLevelText: String {
    permissionLevelOverride ?? "inherit"
  }

  public func withState(_ state: String) -> AgentInstance {
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

extension RuntimeInteraction {
  public var displayTitle: String {
    title ?? (kind == "permission" ? "Permission request" : "Action required")
  }

  public var levelText: String {
    level.uppercased()
  }
}

extension String {
  var nilIfBlank: String? {
    let trimmed = trimmingCharacters(in: .whitespacesAndNewlines)
    return trimmed.isEmpty ? nil : trimmed
  }
}

enum DefaultsKey {
  static let kernelURL = "dev.chariox.ios.kernelURL"
  static let workspacePath = "dev.chariox.ios.workspacePath"
  static let worktreePath = "dev.chariox.ios.worktreePath"
  static let providerID = "dev.chariox.ios.providerID"
  static let modelID = "dev.chariox.ios.modelID"
  static let variantID = "dev.chariox.ios.variantID"
  static let responseLayout = "dev.chariox.ios.responseLayout"
  static let clientID = "dev.chariox.ios.clientID"
}
