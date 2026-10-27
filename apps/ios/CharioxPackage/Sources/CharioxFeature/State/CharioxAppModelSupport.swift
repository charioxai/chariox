import Foundation

@MainActor
extension CharioxAppModel {
  func sessionListText() -> String {
    guard !sessions.isEmpty else {
      return "No sessions found."
    }
    let lines = sessions.map { session in
      let selected = selectedSessionID == session.id ? "*" : "-"
      return "\(selected) \(session.shortDisplayID) \(session.status) \(session.worktreeID)"
    }
    return "Sessions\n\(lines.joined(separator: "\n"))"
  }

  func agentListText(session: RuntimeSession) -> String {
    guard !session.agents.isEmpty else {
      return "No agents reported for \(session.shortDisplayID)."
    }
    let lines = session.agents.map { agent in
      let selected = session.focusedAgentID == agent.id ? "*" : "-"
      return "\(selected) \(agent.displayName) \(agent.providerModelText) \(agent.state)"
    }
    return "Agents\n\(lines.joined(separator: "\n"))"
  }

  func providerCatalogText() -> String {
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
      return
        "\(marker) \(provider.name) (\(provider.id)) \(provider.models.count) model\(provider.models.count == 1 ? "" : "s")\(defaultModel)"
    }
    return "Providers\n\(lines.joined(separator: "\n"))"
  }

  func providerAuthStatusText(_ status: ProviderAuthStatus) -> String {
    [
      status.accountProfile.map { "\(status.provider): \(status.authState) as \($0)" }
        ?? "\(status.provider): \(status.authState)",
      status.detectedVersion.map { "version \($0)" },
      status.loginHint,
    ]
    .compactMap { $0?.nilIfBlank }
    .joined(separator: " • ")
  }

  func providerLoginText(_ login: ProviderLoginStart) -> String {
    [
      "\(login.provider): \(login.loginKind)",
      login.userCode.map { "code \($0)" },
      login.verificationURL ?? login.authURL,
    ]
    .compactMap { $0?.nilIfBlank }
    .joined(separator: " • ")
  }

  func mcpListText() -> String {
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

  func skillListText() -> String {
    guard !skills.isEmpty else {
      return "No skills installed."
    }
    let lines = skills.map { skill in
      let description =
        skill.shortDescription?.nilIfBlank ?? skill.description.nilIfBlank ?? skill.path
      return "\(skill.name) - \(description)"
    }
    return "Skills\n\(lines.joined(separator: "\n"))"
  }

  func workspaceText() -> String {
    "Workspace\nworkspace: \(workspacePath)\nworktree: \(worktreePath)"
  }

  func workspaceLiveSyncText(_ status: WorkspaceLiveSyncStatus, filter: String) -> String {
    let header = "Workspace live sync\nmode: \(status.mode)\nstate: \(status.footerState)"
    switch filter {
    case "targets":
      guard !status.targets.isEmpty else { return "\(header)\ntargets: none" }
      let groupLines = status.syncGroups.map { group in
        "group \(group.groupName) targets=\(group.targetCount) ready=\(group.readyTargets) degraded=\(group.degradedTargets) conflicts=\(group.conflictedTargets)"
      }
      let lines = status.targets.map { target in
        let branch = target.branch.map { " @ \($0)" } ?? ""
        return "- \(target.linkName) \(target.status) \(target.userID) \(target.repoRoot)\(branch)"
      }
      return "\(header)\ntargets\n\((groupLines + lines).joined(separator: "\n"))"
    case "conflicts":
      guard !status.conflicts.isEmpty else { return "\(header)\nconflicts: none" }
      let lines = status.conflicts.map { conflict in
        "- \(conflict.path) -> \(conflict.targetUserID): \(conflict.nextAction)"
      }
      return "\(header)\nconflicts\n\(lines.joined(separator: "\n"))"
    case "ignore":
      let ignoreFile = status.ignore.ignoreFile ?? ".charioxignore"
      let rules =
        status.ignore.rules.isEmpty
        ? "none"
        : status.ignore.rules.joined(separator: ", ")
      let excludes =
        status.ignore.forceExcludes.isEmpty
        ? "none"
        : status.ignore.forceExcludes.joined(separator: ", ")
      return "\(header)\nignore: \(ignoreFile)\nrules: \(rules)\nforce excludes: \(excludes)"
    default:
      return
        "\(header)\nsync groups: \(status.syncGroups.count)\ntargets: \(status.targets.count)\nconflicts: \(status.conflicts.count)\nignore: \(status.ignore.ignoreFile ?? ".charioxignore")\nrules: \(status.ignore.rules.count)"
    }
  }

  func resolveSession(reference: String) -> RuntimeSession? {
    let normalized = reference.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    guard !normalized.isEmpty else { return selectedSession }
    return sessions.first { session in
      session.id.lowercased() == normalized
        || session.id.lowercased().hasPrefix(normalized)
        || session.alias?.lowercased() == normalized
        || session.shortDisplayID.lowercased() == normalized
    }
  }

  func resolveAgent(reference: String, in session: RuntimeSession) -> AgentInstance? {
    let normalized = reference.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    guard !normalized.isEmpty else { return session.focusedAgent }
    return session.agents.first { agent in
      agent.id.lowercased() == normalized
        || agent.id.lowercased().hasPrefix(normalized)
        || agent.agentRef.lowercased() == normalized
        || agent.alias?.lowercased() == normalized
    }
  }

  func parseAgentConfigCommand(_ args: [String], usage: String) -> (
    agent: AgentInstance, value: String
  )? {
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
      statusMessage =
        args.count == 1 ? "No focused agent to configure." : "Agent `\(args[0])` was not found."
      return nil
    }
    return (agent, value.lowercased())
  }

  func appendSessionConfigNotice(key: String, fallback: String, label: String) async {
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

  func loadRecentHistory(endpoint: URL, session: RuntimeSession) async {
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
      guard case .sessionHistory(let entries) = response else {
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

  func upsert(_ session: RuntimeSession) {
    if let index = sessions.firstIndex(where: { $0.id == session.id }) {
      sessions[index] = session
    } else {
      sessions.insert(session, at: 0)
    }
  }

  func upsert(agent: AgentInstance, inSessionID sessionID: String, focused: Bool) {
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
    sessions[index] = session.replacingAgents(
      agents, focusedAgentID: focused ? agent.id : session.focusedAgentID)
    selectedSessionID = sessionID
  }

  func remove(agent: AgentInstance, fromSessionID sessionID: String) {
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

  func updateAgentConfig(
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
      guard case .agentConfigUpdated(let updatedAgent, let updatedSession) = response else {
        throw KernelClientError.unexpectedResponse(String(describing: response))
      }
      upsert(updatedSession)
      selectedSessionID = updatedSession.id
      return successMessage(updatedAgent)
    }
  }

  func updateSessionConfig(values: [String: String], successMessage: String) async {
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
      guard case .sessionConfigUpdated(let updatedSession, _) = response else {
        throw KernelClientError.unexpectedResponse(String(describing: response))
      }
      upsert(updatedSession)
      selectedSessionID = updatedSession.id
      return successMessage
    }
  }

  func markFocusedAgent(sessionID: String, agent: AgentInstance?) {
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

  static func loadClientID(from defaults: UserDefaults) -> String {
    if let existing = defaults.string(forKey: DefaultsKey.clientID), !existing.isEmpty {
      return existing
    }
    let next = "chariox-ios-\(UUID().uuidString)"
    defaults.set(next, forKey: DefaultsKey.clientID)
    return next
  }

  static func defaultWorkspacePath() -> String {
    ProcessInfo.processInfo.environment["CHARIOX_IOS_DEFAULT_WORKSPACE"]?.nilIfBlank ?? ""
  }
}
