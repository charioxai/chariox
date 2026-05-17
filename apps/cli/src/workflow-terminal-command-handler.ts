import type { WorkflowDefinition } from "./cli-types.js"

type FooterTone = "info" | "error"

export type WorkflowTerminalCommandDeps = {
  sessionWorkflows: () => readonly WorkflowDefinition[]
  workflowRefOrSelected: (workflowRef: string | null | undefined) => string | null
  resolveWorkflow: (workflowRef: string) => Promise<{ workflow: WorkflowDefinition }>
  upsertWorkflowDefinition: (workflow: WorkflowDefinition) => void
  selectWorkflowCanvas: (workflowId: string | null) => void
  showWorkflowScreen: () => void
  openWorkflowTerminalPanel?: (workflowId: string) => void
  flashFooter: (message: string, tone: FooterTone) => void
}

export async function handleWorkflowTerminalCommand(
  deps: WorkflowTerminalCommandDeps,
  args: readonly string[],
): Promise<void> {
  const workflowRef = deps.workflowRefOrSelected(args[1]) ?? deps.sessionWorkflows()[0]?.id ?? null
  if (!workflowRef) {
    deps.flashFooter("usage: /workflow terminal [workflow-ref]", "error")
    return
  }
  const payload = await deps.resolveWorkflow(workflowRef)
  deps.upsertWorkflowDefinition(payload.workflow)
  deps.selectWorkflowCanvas(payload.workflow.id)
  deps.showWorkflowScreen()
  deps.openWorkflowTerminalPanel?.(payload.workflow.id)
  deps.flashFooter(`opened workflow terminal for ${payload.workflow.id}`, "info")
}
