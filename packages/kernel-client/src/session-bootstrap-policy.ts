export type AttachableSessionInput = {
  id: string
  workspace_id?: string | null
  worktree_id: string
  status: string
  created_at_ms?: number | null
}

export type SessionBootstrapDecision =
  | { action: "create" }
  | { action: "resolve"; sessionRef: string }
  | { action: "attach_existing"; sessionId: string }
  | { action: "none" }

export function selectAttachableSession<T extends AttachableSessionInput>(
  sessions: readonly T[],
  workspace: string,
  worktree: string,
): T | null {
  return sessions
    .filter((session) => session.workspace_id === workspace && session.worktree_id === worktree && session.status !== "Ended")
    .slice()
    .sort((left, right) => (right.created_at_ms ?? 0) - (left.created_at_ms ?? 0))[0] ?? null
}

export function decideBootstrapAction(
  options: { createSession?: boolean; sessionId?: string | null | undefined },
  sessions: readonly AttachableSessionInput[],
  workspace: string,
  worktree: string,
): SessionBootstrapDecision {
  if (options.createSession) {
    return { action: "create" }
  }
  if (options.sessionId) {
    return { action: "resolve", sessionRef: options.sessionId }
  }
  const existing = selectAttachableSession(sessions, workspace, worktree)
  if (existing) {
    return { action: "none" }
  }
  return { action: "none" }
}
