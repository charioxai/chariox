import type { ResponsePaneAgent } from "@chariox/kernel-client/response-pane-selection"

export type WorkspaceScreenMode = "agents" | "workflow"

export function toggleWorkspaceScreenMode(mode: WorkspaceScreenMode): WorkspaceScreenMode {
  return mode === "agents" ? "workflow" : "agents"
}

export function resolveWorkspaceVisibleAgents<T extends ResponsePaneAgent>(
  mode: WorkspaceScreenMode,
  agents: readonly T[],
) {
  return mode === "workflow" ? [] : [...agents]
}

export function resolveWorkspaceVisibleTranscriptAgentId(
  mode: WorkspaceScreenMode,
  agentId: string | null,
) {
  return mode === "workflow" ? null : agentId
}
