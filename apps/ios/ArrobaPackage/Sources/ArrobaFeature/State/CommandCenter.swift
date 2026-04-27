import Foundation

public struct CommandCenterItem: Identifiable, Equatable, Sendable {
    public let id: String
    public let label: String
    public let detail: String
    public let value: String
    public let submitsImmediately: Bool

    public init(
        id: String,
        label: String,
        detail: String,
        value: String,
        submitsImmediately: Bool
    ) {
        self.id = id
        self.label = label
        self.detail = detail
        self.value = value
        self.submitsImmediately = submitsImmediately
    }
}

public enum CommandCenterCatalog {
    public static func items(matching input: String, session: RuntimeSession?) -> [CommandCenterItem] {
        let trimmed = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.hasPrefix("/") else { return [] }
        if trimmed.hasPrefix("/agent focus ") {
            return agentFocusItems(matching: trimmed, session: session)
        }
        if trimmed.hasPrefix("/agent destroy ") || trimmed.hasPrefix("/agent delete ") {
            return agentDestroyItems(matching: trimmed, session: session)
        }
        if trimmed.hasPrefix("/agent") {
            return filter(agentItems, query: trimmed.removingCommandPrefix("/agent"))
        }
        if trimmed.hasPrefix("/session") {
            return filter(sessionItems, query: trimmed.removingCommandPrefix("/session"))
        }
        if trimmed.hasPrefix("/provider") {
            return filter(providerItems, query: trimmed.removingCommandPrefix("/provider"))
        }
        if trimmed.hasPrefix("/mcp") {
            return filter(mcpItems, query: trimmed.removingCommandPrefix("/mcp"))
        }
        if trimmed.hasPrefix("/skill") {
            return filter(skillItems, query: trimmed.removingCommandPrefix("/skill"))
        }
        if trimmed.hasPrefix("/workspace") {
            return filter(workspaceItems, query: trimmed.removingCommandPrefix("/workspace"))
        }
        return filter(rootItems, query: String(trimmed.dropFirst()))
    }

    private static let rootItems: [CommandCenterItem] = [
        CommandCenterItem(
            id: "session",
            label: "/session",
            detail: "Create, attach, list, or detach sessions",
            value: "/session ",
            submitsImmediately: false
        ),
        CommandCenterItem(
            id: "agent",
            label: "/agent",
            detail: "List, spawn, destroy, focus, cycle, or configure agents",
            value: "/agent ",
            submitsImmediately: false
        ),
        CommandCenterItem(
            id: "provider",
            label: "/provider",
            detail: "Show connected providers and model inventory",
            value: "/provider ",
            submitsImmediately: false
        ),
        CommandCenterItem(
            id: "mcp",
            label: "/mcp",
            detail: "Show installed MCP servers",
            value: "/mcp ",
            submitsImmediately: false
        ),
        CommandCenterItem(
            id: "skill",
            label: "/skill",
            detail: "Show installed skills",
            value: "/skill ",
            submitsImmediately: false
        ),
        CommandCenterItem(
            id: "workspace",
            label: "/workspace",
            detail: "Show or set session creation paths",
            value: "/workspace ",
            submitsImmediately: false
        ),
        CommandCenterItem(
            id: "stop",
            label: "/stop",
            detail: "Request cancellation of the active provider turn",
            value: "/stop",
            submitsImmediately: true
        ),
        CommandCenterItem(
            id: "waiting",
            label: "/waiting",
            detail: "Show waiting-room context",
            value: "/waiting",
            submitsImmediately: true
        ),
    ]

    private static let sessionItems: [CommandCenterItem] = [
        CommandCenterItem(
            id: "session-new",
            label: "new",
            detail: "Create and attach a new session for the configured workspace/worktree",
            value: "/session new",
            submitsImmediately: true
        ),
        CommandCenterItem(
            id: "session-create",
            label: "create",
            detail: "Alias for /session new",
            value: "/session create",
            submitsImmediately: true
        ),
        CommandCenterItem(
            id: "session-list",
            label: "list",
            detail: "Refresh and list kernel sessions",
            value: "/session list",
            submitsImmediately: true
        ),
        CommandCenterItem(
            id: "session-attach",
            label: "attach",
            detail: "Attach to the selected session or a typed id/alias",
            value: "/session attach ",
            submitsImmediately: false
        ),
        CommandCenterItem(
            id: "session-detach",
            label: "detach",
            detail: "Detach the current iOS attachment",
            value: "/session detach",
            submitsImmediately: true
        ),
        CommandCenterItem(
            id: "session-mode",
            label: "mode",
            detail: "Show or set session default agent mode",
            value: "/session mode ",
            submitsImmediately: false
        ),
        CommandCenterItem(
            id: "session-permissions",
            label: "permissions",
            detail: "Show or set session default agent permissions",
            value: "/session permissions ",
            submitsImmediately: false
        ),
    ]

    private static let agentItems: [CommandCenterItem] = [
        CommandCenterItem(
            id: "agent-list",
            label: "list",
            detail: "List agents in the selected session",
            value: "/agent list",
            submitsImmediately: true
        ),
        CommandCenterItem(
            id: "agent-spawn",
            label: "spawn",
            detail: "Spawn a new agent cloned from the focused agent context",
            value: "/agent spawn ",
            submitsImmediately: false
        ),
        CommandCenterItem(
            id: "agent-destroy",
            label: "destroy",
            detail: "Destroy the focused agent or a typed id/ref/alias",
            value: "/agent destroy ",
            submitsImmediately: false
        ),
        CommandCenterItem(
            id: "agent-focus",
            label: "focus",
            detail: "Focus a specific agent by id, ref, or alias",
            value: "/agent focus ",
            submitsImmediately: false
        ),
        CommandCenterItem(
            id: "agent-cycle",
            label: "cycle",
            detail: "Cycle to the next agent",
            value: "/agent cycle",
            submitsImmediately: true
        ),
        CommandCenterItem(
            id: "agent-mode",
            label: "mode",
            detail: "Set focused agent mode: build, plan, or inherit",
            value: "/agent mode ",
            submitsImmediately: false
        ),
        CommandCenterItem(
            id: "agent-permissions",
            label: "permissions",
            detail: "Set focused agent permissions: required, yolo, or inherit",
            value: "/agent permissions ",
            submitsImmediately: false
        ),
    ]

    private static let providerItems: [CommandCenterItem] = [
        CommandCenterItem(
            id: "provider-list",
            label: "list",
            detail: "Load connected providers and model counts",
            value: "/provider list",
            submitsImmediately: true
        ),
        CommandCenterItem(
            id: "provider-status",
            label: "status",
            detail: "Alias for /provider list",
            value: "/provider status",
            submitsImmediately: true
        ),
        CommandCenterItem(
            id: "provider-auth",
            label: "auth",
            detail: "Show auth state for a provider id",
            value: "/provider auth ",
            submitsImmediately: false
        ),
    ]

    private static let mcpItems: [CommandCenterItem] = [
        CommandCenterItem(
            id: "mcp-list",
            label: "list",
            detail: "Load installed MCP servers",
            value: "/mcp list",
            submitsImmediately: true
        ),
    ]

    private static let skillItems: [CommandCenterItem] = [
        CommandCenterItem(
            id: "skill-list",
            label: "list",
            detail: "Load installed skills",
            value: "/skill list",
            submitsImmediately: true
        ),
    ]

    private static let workspaceItems: [CommandCenterItem] = [
        CommandCenterItem(
            id: "workspace-show",
            label: "show",
            detail: "Show current workspace and worktree paths",
            value: "/workspace show",
            submitsImmediately: true
        ),
        CommandCenterItem(
            id: "workspace-set",
            label: "set",
            detail: "Set workspace and worktree to the same path",
            value: "/workspace set ",
            submitsImmediately: false
        ),
        CommandCenterItem(
            id: "workspace-path",
            label: "path",
            detail: "Set workspace path only",
            value: "/workspace path ",
            submitsImmediately: false
        ),
        CommandCenterItem(
            id: "workspace-worktree",
            label: "worktree",
            detail: "Set worktree path only",
            value: "/workspace worktree ",
            submitsImmediately: false
        ),
    ]

    private static func agentFocusItems(matching input: String, session: RuntimeSession?) -> [CommandCenterItem] {
        guard let session else { return [] }
        let query = input.replacingOccurrences(of: "/agent focus ", with: "")
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
        let items = session.agents.map { agent in
            CommandCenterItem(
                id: "agent-focus-\(agent.id)",
                label: agent.displayName,
                detail: "\(agent.providerModelText) - \(agent.state)",
                value: "/agent focus \(agent.id)",
                submitsImmediately: true
            )
        }
        return filter(items, query: query)
    }

    private static func agentDestroyItems(matching input: String, session: RuntimeSession?) -> [CommandCenterItem] {
        guard let session else { return [] }
        let query = input
            .replacingOccurrences(of: "/agent destroy ", with: "")
            .replacingOccurrences(of: "/agent delete ", with: "")
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
        let items = session.agents.map { agent in
            CommandCenterItem(
                id: "agent-destroy-\(agent.id)",
                label: agent.displayName,
                detail: "\(agent.providerModelText) - \(agent.state)",
                value: "/agent destroy \(agent.id)",
                submitsImmediately: true
            )
        }
        return filter(items, query: query)
    }

    private static func filter(_ items: [CommandCenterItem], query: String) -> [CommandCenterItem] {
        let normalized = query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !normalized.isEmpty else { return items }
        return items.filter { item in
            item.label.lowercased().contains(normalized)
                || item.detail.lowercased().contains(normalized)
                || item.value.lowercased().contains(normalized)
        }
    }
}

private extension String {
    func removingCommandPrefix(_ prefix: String) -> String {
        replacingOccurrences(of: prefix, with: "")
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
