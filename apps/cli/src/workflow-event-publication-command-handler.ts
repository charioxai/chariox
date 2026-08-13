import {
  createDefaultShellContext,
  executeWorkflowPublicationCommand,
  executeWorkflowEventPublicationCommand,
  type RuntimeSession,
  type ShellCommandResult,
} from "@chariox/kernel-client"

type WorkflowPublicationCommandDeps = {
  sessionState: () => RuntimeSession
  currentWorkspaceTarget: () => string
  selectedWorkflowId?: () => string | null
  sendWorkflowEventPublicationRequest: (
    request: Record<string, unknown>,
  ) => Promise<Record<string, unknown>>
  applySessionState: (session: RuntimeSession) => void
  appendNotice: (message: string) => void
  flashFooter: (message: string, tone: "info" | "error") => void
}

export async function handleWorkflowPublicationCommand(
  deps: WorkflowPublicationCommandDeps,
  args: string[],
): Promise<void> {
  await executeAndRenderPublicationCommand(deps, (context) =>
    executeWorkflowPublicationCommand(
      args,
      context,
      { client: { send: deps.sendWorkflowEventPublicationRequest } },
    ))
}

export async function handleWorkflowEventPublicationCommand(
  deps: WorkflowPublicationCommandDeps,
  args: string[],
): Promise<void> {
  await executeAndRenderPublicationCommand(deps, (context) =>
    executeWorkflowEventPublicationCommand(
      args,
      context,
      { send: deps.sendWorkflowEventPublicationRequest },
    ))
}

async function executeAndRenderPublicationCommand(
  deps: WorkflowPublicationCommandDeps,
  execute: (context: ReturnType<typeof createDefaultShellContext>) => Promise<ShellCommandResult>,
): Promise<void> {
  const session = deps.sessionState()
  const workspace = deps.currentWorkspaceTarget()
  const result = await execute(createDefaultShellContext({
      workspace,
      worktree: session.worktree_id ?? workspace,
      sessionId: session.id,
      agentId: session.focused_agent_id ?? undefined,
      workflowId: deps.selectedWorkflowId?.() ?? session.workflows?.[0]?.id,
    }))
  if (!result.ok) {
    deps.flashFooter(result.message ?? "workflow publication command failed", "error")
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
