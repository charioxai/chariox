import type { ShellContext } from "@arroba/kernel-client/shell-core"
import { executeShellLine } from "@arroba/kernel-client/shell-script"

import type {
  RuntimeSession,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import {
  appendWorkspaceShellEntry,
  workspaceShellCommandText,
  type WorkspaceShellEntry,
} from "./workspace-shell.js"

export type WorkspaceShellSubmitResult = {
  ok: boolean
  output: string
  context: ShellContext
}

export function deriveWorkspaceShellContextForSession(
  previous: ShellContext,
  session: RuntimeSession,
  attachmentId: string | null | undefined,
): ShellContext {
  return {
    ...previous,
    workspace: session.workspace_id || previous.workspace,
    worktree: session.worktree_id || previous.worktree,
    sessionId: session.id,
    attachmentId: attachmentId ?? previous.attachmentId,
    agentId: session.focused_agent_id ?? session.agents[0]?.id ?? previous.agentId,
  }
}

type WorkspaceShellSubmitDeps = {
  client: LocalIpcClient
  executeShellLine?: typeof executeShellLine
  workspaceShellContext: () => ShellContext
  setWorkspaceShellContext: (context: ShellContext) => void
  nextEntryId: () => number
  setWorkspaceShellEntries: (updater: (entries: WorkspaceShellEntry[]) => WorkspaceShellEntry[]) => void
  sessionState: () => RuntimeSession
  refreshSessionState: (sessionId: string) => Promise<RuntimeSession>
  applySessionState: (session: RuntimeSession) => void
  selectedWorkflowId: () => string | null
  setSelectedWorkflowId: (workflowId: string) => void
  setSelectedWorkflowNodeId: (nodeId: string | null) => void
  rebuildTranscript: () => void
  flashFooter: (message: string, tone: "info" | "error") => void
  onSessionRefreshError?: (sessionId: string, error: unknown) => void
}

export function createWorkspaceShellSubmitController(
  deps: WorkspaceShellSubmitDeps,
): {
  submit: (rawPrompt: string) => Promise<WorkspaceShellSubmitResult>
} {
  return {
    submit(rawPrompt) {
      return submitWorkspaceShellCommand(rawPrompt, deps)
    },
  }
}

export async function submitWorkspaceShellCommand(
  rawPrompt: string,
  deps: WorkspaceShellSubmitDeps,
): Promise<WorkspaceShellSubmitResult> {
  const command = workspaceShellCommandText(rawPrompt)
  if (!command) {
    deps.flashFooter("usage: @ <arroba-shell command>", "error")
    return { ok: false, output: "usage: @ <arroba-shell command>", context: deps.workspaceShellContext() }
  }
  const context = deps.workspaceShellContext()
  const output: string[] = []
  const runShellLine = deps.executeShellLine ?? executeShellLine
  const result = await runShellLine(command, context, { client: deps.client }, (text) => output.push(text))
  const rendered = output.join("").trimEnd()
  const nextContext = result.context
  deps.setWorkspaceShellContext(nextContext)
  deps.setWorkspaceShellEntries((entries) => appendWorkspaceShellEntry(entries, {
    id: deps.nextEntryId(),
    command,
    output: rendered,
    ok: result.ok,
  }))

  const nextSessionId = nextContext.sessionId
  if (nextSessionId && nextSessionId === deps.sessionState().id) {
    try {
      deps.applySessionState(await deps.refreshSessionState(nextSessionId))
    } catch (error) {
      deps.onSessionRefreshError?.(nextSessionId, error)
    }
  }

  const nextWorkflowId = nextContext.workflowId ?? null
  if (result.ok && nextWorkflowId) {
    const workflowExists = (deps.sessionState().workflows ?? []).some((workflow) => workflow.id === nextWorkflowId)
    if (workflowExists && deps.selectedWorkflowId() !== nextWorkflowId) {
      deps.setSelectedWorkflowId(nextWorkflowId)
      deps.setSelectedWorkflowNodeId(null)
    }
  }

  deps.rebuildTranscript()
  deps.flashFooter(result.ok ? "shell command completed" : (rendered || "shell command failed"), result.ok ? "info" : "error")
  return { ok: result.ok, output: rendered, context: nextContext }
}
