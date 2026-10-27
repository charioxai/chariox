import type { WorkflowDefinition } from "./cli-types.js"
import type { WorkflowInspectorMode } from "./workflow-inspector-projection.js"

type FooterTone = "info" | "error"

export type WorkflowTerminalCommandDeps = {
  sessionWorkflows: () => readonly WorkflowDefinition[]
  workflowRefOrSelected: (workflowRef: string | null | undefined) => string | null
  resolveWorkflow: (workflowRef: string) => Promise<{ workflow: WorkflowDefinition }>
  upsertWorkflowDefinition: (workflow: WorkflowDefinition) => void
  selectWorkflowCanvas: (workflowId: string | null) => void
  showWorkflowScreen: () => void
  openWorkflowTerminalPanel?: (workflowId: string, mode?: WorkflowInspectorMode) => void
  flashFooter: (message: string, tone: FooterTone) => void
}

export async function handleWorkflowTerminalCommand(
  deps: WorkflowTerminalCommandDeps,
  args: readonly string[],
): Promise<void> {
  const paneMode = args[0] === "pane" ? parsePaneMode(args[1]) : "logs"
  if (!paneMode) {
    deps.flashFooter("usage: /workflow pane logs|trace|edit [workflow-ref]", "error")
    return
  }
  const workflowArgIndex = args[0] === "pane" ? 2 : 1
  const workflowRef = deps.workflowRefOrSelected(args[workflowArgIndex]) ?? deps.sessionWorkflows()[0]?.id ?? null
  if (!workflowRef) {
    deps.flashFooter(args[0] === "pane" ? "usage: /workflow pane logs|trace|edit [workflow-ref]" : "usage: /workflow terminal [workflow-ref]", "error")
    return
  }
  const payload = await deps.resolveWorkflow(workflowRef)
  deps.upsertWorkflowDefinition(payload.workflow)
  deps.selectWorkflowCanvas(payload.workflow.id)
  deps.showWorkflowScreen()
  deps.openWorkflowTerminalPanel?.(payload.workflow.id, paneMode)
  deps.flashFooter(`opened workflow ${paneMode} pane for ${payload.workflow.id}`, "info")
}

function parsePaneMode(value: string | undefined): WorkflowInspectorMode | null {
  if (value === "logs" || value === "trace" || value === "edit") {
    return value
  }
  if (value === "traces") {
    return "trace"
  }
  return null
}
