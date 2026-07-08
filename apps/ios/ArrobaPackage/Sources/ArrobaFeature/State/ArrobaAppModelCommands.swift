import Foundation

@MainActor
extension ArrobaAppModel {
  func executeSlashCommand(_ rawCommand: String) async {
    let tokens =
      rawCommand
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

  func executeMcpCommand(_ args: [String]) async {
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

  func executeSkillCommand(_ args: [String]) async {
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

  func executeConfigCommand(_ args: [String]) async {
    let action = args.first ?? ""
    switch action {
    case "workspace-live-sync":
      let policy = args.dropFirst().first ?? "off"
      guard args.dropFirst(2).isEmpty,
        ["off", "managed", "tracked"].contains(policy)
      else {
        connectionState = .failed
        statusMessage = "usage: /config workspace-live-sync off|managed|tracked"
        return
      }
      await setWorkspaceLiveSyncPolicy(
        policy,
        notice: "Workspace live sync set to \(policy)",
        status: "Workspace live sync set to \(policy)."
      )
    default:
      connectionState = .failed
      statusMessage = "usage: /config workspace-live-sync off|managed|tracked"
    }
  }

  func executeWorkspaceCommand(_ args: [String]) async {
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

  func executeWorkspaceSyncCommand(_ args: [String]) async {
    let action = args.first ?? "status"
    switch action {
    case "status", "targets", "conflicts", "ignore":
      await showWorkspaceLiveSyncStatus(filter: action)
    case "off":
      await setWorkspaceLiveSyncMode("unrestricted")
    case "managed", "tracked":
      await setWorkspaceLiveSyncMode(action)
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
        ["off", "managed", "tracked"].contains(mode)
      else {
        connectionState = .failed
        statusMessage = "usage: /workspace sync mode off|managed|tracked"
        return
      }
      await setWorkspaceLiveSyncMode(mode == "off" ? "unrestricted" : mode)
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
      statusMessage =
        "usage: /workspace sync status|targets|conflicts|ignore|off|managed|tracked|enable|disable|mode|link"
    }
  }

  func showWorkspaceLiveSyncStatus(filter: String) async {
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
      guard case .workspaceLiveSyncStatus(let status) = response else {
        throw KernelClientError.unexpectedResponse(String(describing: response))
      }
      appendCommandNotice(workspaceLiveSyncText(status, filter: filter))
      promptDraft = ""
      return "Workspace live sync \(status.mode)."
    }
  }

  func setWorkspaceLiveSyncMode(
    _ mode: String,
    notice: String? = nil,
    status: String? = nil
  ) async {
    guard let selectedSessionID else {
      connectionState = .failed
      statusMessage = "Attach to a session before updating workspace live sync."
      return
    }
    await perform("Updating workspace live sync") {
      let response = try await client.send(
        .setWorkspaceLiveSyncMode(sessionID: selectedSessionID, mode: mode),
        to: try endpointURL()
      )
      guard case .workspaceLiveSyncModeUpdated(let session) = response else {
        throw KernelClientError.unexpectedResponse(String(describing: response))
      }
      upsert(session)
      self.selectedSessionID = session.id
      appendCommandNotice(notice ?? "Workspace live sync mode = \(mode)")
      promptDraft = ""
      return status ?? "Workspace live sync set to \(mode)."
    }
  }

  func setWorkspaceLiveSyncPolicy(
    _ policy: String,
    notice: String,
    status: String
  ) async {
    await perform("Updating workspace live sync policy") {
      let response = try await client.send(
        .setUserConfigValue(path: "providers.workspace_live_sync", value: policy),
        to: try endpointURL()
      )
      guard case .userConfigUpdated = response else {
        throw KernelClientError.unexpectedResponse(String(describing: response))
      }
      appendCommandNotice(notice)
      promptDraft = ""
      return status
    }
  }

  func attachWorkspaceLiveSyncLink(linkRef: String, repoRoot: String?) async {
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
      guard case .workspaceLinkAttached(let updatedSession) = response else {
        throw KernelClientError.unexpectedResponse(String(describing: response))
      }
      upsert(updatedSession)
      selectedSessionID = updatedSession.id
      appendCommandNotice(
        "Workspace live sync linked \(linkRef) -> \(targetRoot). Recommended mode: managed.")
      promptDraft = ""
      return "Workspace live sync linked. Recommended mode: managed."
    }
  }

  func executeProviderCommand(_ args: [String]) async {
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

  func executeViewCommand(_ args: [String]) {
    guard let rawValue = args.first?.lowercased(),
      let layout = MultiAgentResponseLayout(rawValue: rawValue)
    else {
      connectionState = .failed
      statusMessage = "usage: /view split|individual"
      return
    }
    setResponseLayout(layout)
  }

  func executeSessionCommand(_ args: [String]) async {
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
      statusMessage =
        "usage: /session new|create|list|attach [session-ref]|detach|mode [build|plan]|permissions [required|yolo]"
    }
  }

  func executeSessionModeCommand(_ args: [String]) async {
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

  func executeSessionPermissionsCommand(_ args: [String]) async {
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

  func executeAgentCommand(_ args: [String]) async {
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
      let target =
        reference.flatMap { resolveAgent(reference: $0, in: session) } ?? session.focusedAgent
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
      statusMessage =
        "usage: /agent list|spawn [alias] [model]|destroy [agent-id]|focus <agent-id>|cycle|mode|permissions"
    }
  }

  func executeAgentModeCommand(_ args: [String]) async {
    guard
      let parsed = parseAgentConfigCommand(
        args,
        usage: "usage: /agent mode [agent-id] <build|plan|inherit>"
      )
    else {
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

  func executeAgentPermissionsCommand(_ args: [String]) async {
    guard
      let parsed = parseAgentConfigCommand(
        args,
        usage: "usage: /agent permissions [agent-id] <required|yolo|inherit>"
      )
    else {
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

}
