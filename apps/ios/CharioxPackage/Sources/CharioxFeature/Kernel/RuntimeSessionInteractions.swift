import Foundation

extension RuntimeSession {
    func replacingActiveInteractions(_ interactions: [RuntimeInteraction]) -> RuntimeSession {
        RuntimeSession(id: id, alias: alias, workspaceID: workspaceID, projectID: projectID,
            worktreeID: worktreeID, status: status, configState: configState,
            activePrompt: activePrompt, queuedPrompts: queuedPrompts, promptStates: promptStates,
            activeInteractions: interactions, focusedAgentID: focusedAgentID, agents: agents,
            workspaceLiveSyncMode: workspaceLiveSyncMode, createdAtMs: createdAtMs,
            lastUsedAtMs: lastUsedAtMs, lastActivityAtMs: lastActivityAtMs)
    }
}
