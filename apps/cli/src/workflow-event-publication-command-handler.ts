import {
  createDefaultShellContext,
  executeWorkflowEventPublicationCommand,
  type RuntimeSession,
} from "@arroba/kernel-client"

type WorkflowEventPublicationCommandDeps = {
  sessionState: () => RuntimeSession
  currentWorkspaceTarget: () => string
  sendWorkflowEventPublicationRequest: (
    request: Record<string, unknown>,
  ) => Promise<Record<string, unknown>>
  applySessionState: (session: RuntimeSession) => void
  appendNotice: (message: string) => void
  flashFooter: (message: string, tone: "info" | "error") => void
}

export async function handleWorkflowEventPublicationCommand(
  deps: WorkflowEventPublicationCommandDeps,
  args: string[],
): Promise<void> {
  const session = deps.sessionState()
  const workspace = deps.currentWorkspaceTarget()
  const result = await executeWorkflowEventPublicationCommand(
    args,
    createDefaultShellContext({
      workspace,
      worktree: session.worktree_id ?? workspace,
      sessionId: session.id,
      agentId: session.focused_agent_id ?? undefined,
      workflowId: session.workflows?.[0]?.id,
    }),
    { send: deps.sendWorkflowEventPublicationRequest },
  )
  if (!result.ok) {
    deps.flashFooter(result.message ?? "event publication command failed", "error")
    return
  }
  const updatedSession = sessionFromResult(result.data)
  if (updatedSession) deps.applySessionState(updatedSession)
  if (result.message) deps.appendNotice(result.message)
}

function sessionFromResult(value: unknown): RuntimeSession | null {
  if (!value || typeof value !== "object") return null
  const session = (value as { session?: unknown }).session
  if (!session || typeof session !== "object" || typeof (session as { id?: unknown }).id !== "string") {
    return null
  }
  return session as RuntimeSession
}
