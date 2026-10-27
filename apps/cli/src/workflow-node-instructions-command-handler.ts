import type {
  RuntimeSession,
  WorkflowDefinition,
  WorkflowNodeDefinition,
} from "./cli-types.js"
import { readFile } from "node:fs/promises"
import { resolve as resolvePath } from "node:path"

type FooterTone = "info" | "error"

export type WorkflowNodeInstructionsPayload = {
  node: WorkflowNodeDefinition
  workflow: WorkflowDefinition
  session: RuntimeSession
}

type WorkflowResolvePayload = {
  workflow: WorkflowDefinition
}

export type WorkflowNodeInstructionsCommandContext = {
  firstWorkflowArgIsExplicit: (workflowRef: string | undefined) => boolean
  workflowRefOrSelected: (workflowRef: string | null | undefined) => string | null
}

export type WorkflowNodeInstructionsCommandDeps = {
  currentWorkspaceTarget: () => string
  resolveWorkflow: (workflowRef: string) => Promise<WorkflowResolvePayload>
  upsertWorkflowDefinition: (workflow: WorkflowDefinition) => void
  updateWorkflowNodeInstructions?: (
    workflowRef: string,
    nodeId: string,
    instructions: string | null,
  ) => Promise<WorkflowNodeInstructionsPayload>
  openWorkflowNodeInstructionsEditor?: (workflowId: string, nodeId: string, draft: string) => void
  closeWorkflowNodeInstructionsEditor?: () => void
  getWorkflowNodeInstructionsDraft?: () => string
  getWorkflowNodeInstructionsContext?: () => { workflowId: string; nodeId: string } | null
  applySessionState: (session: RuntimeSession) => void
  selectWorkflowCanvas: (workflowId: string | null) => void
  flashFooter: (message: string, tone: FooterTone) => void
}

export async function handleWorkflowNodeInstructionsCommand(
  deps: WorkflowNodeInstructionsCommandDeps,
  context: WorkflowNodeInstructionsCommandContext,
  args: readonly string[],
): Promise<void> {
  const instructionsAction = args[2]
  if (!instructionsAction) {
    deps.flashFooter(
      "usage: /workflow node instructions show|set|save|close [workflow-ref] <node-id> [file]",
      "error",
    )
    return
  }
  if (instructionsAction === "close") {
    deps.closeWorkflowNodeInstructionsEditor?.()
    deps.flashFooter("closed node instructions editor", "info")
    return
  }
  if (instructionsAction === "save") {
    await saveOpenInstructionsEditor(deps)
    return
  }
  const explicitWorkflowRef = context.firstWorkflowArgIsExplicit(args[3]) ? args[3] : null
  const instructionsWorkflowRef = context.workflowRefOrSelected(explicitWorkflowRef)
  const nodeId = explicitWorkflowRef ? args[4] : args[3]
  const fileRef = explicitWorkflowRef ? args[5] : args[4]
  if (!instructionsWorkflowRef || !nodeId) {
    deps.flashFooter(
      "usage: /workflow node instructions show|set [workflow-ref] <node-id> [file]",
      "error",
    )
    return
  }
  const resolved = await deps.resolveWorkflow(instructionsWorkflowRef)
  deps.upsertWorkflowDefinition(resolved.workflow)
  const node = resolved.workflow.nodes?.find((entry) => entry.id === nodeId)
  if (!node) {
    deps.flashFooter(`workflow node ${nodeId} not found`, "error")
    return
  }
  if (instructionsAction === "show") {
    openInstructionsEditor(deps, resolved.workflow.id, node)
    deps.flashFooter(`opened node ${node.id} instructions in the I/O panel`, "info")
    return
  }
  if (instructionsAction !== "set") {
    deps.flashFooter(
      "usage: /workflow node instructions show|set|save|close [workflow-ref] <node-id> [file]",
      "error",
    )
    return
  }
  if (fileRef) {
    await updateInstructionsFromFile(deps, resolved.workflow.id, node.id, fileRef)
    return
  }
  openInstructionsEditor(deps, resolved.workflow.id, node)
  deps.flashFooter("editing node instructions in the I/O panel; submit text then /workflow node instructions save", "info")
}

async function saveOpenInstructionsEditor(
  deps: WorkflowNodeInstructionsCommandDeps,
): Promise<void> {
  const contextState = deps.getWorkflowNodeInstructionsContext?.()
  if (!contextState || !deps.updateWorkflowNodeInstructions || !deps.getWorkflowNodeInstructionsDraft) {
    deps.flashFooter("no workflow node instructions editor is open", "error")
    return
  }
  const payload = await deps.updateWorkflowNodeInstructions(
    contextState.workflowId,
    contextState.nodeId,
    deps.getWorkflowNodeInstructionsDraft(),
  )
  deps.applySessionState(payload.session)
  deps.upsertWorkflowDefinition(payload.workflow)
  deps.closeWorkflowNodeInstructionsEditor?.()
  deps.flashFooter(`saved node instructions for ${payload.node.id}`, "info")
}

async function updateInstructionsFromFile(
  deps: WorkflowNodeInstructionsCommandDeps,
  workflowId: string,
  nodeId: string,
  fileRef: string,
): Promise<void> {
  if (!deps.updateWorkflowNodeInstructions) {
    deps.flashFooter("workflow instructions unavailable", "error")
    return
  }
  const content = await readFile(resolvePath(deps.currentWorkspaceTarget(), fileRef), "utf8")
  const payload = await deps.updateWorkflowNodeInstructions(workflowId, nodeId, content)
  deps.applySessionState(payload.session)
  deps.upsertWorkflowDefinition(payload.workflow)
  deps.flashFooter(`updated node instructions for ${payload.node.id}`, "info")
}

function openInstructionsEditor(
  deps: WorkflowNodeInstructionsCommandDeps,
  workflowId: string,
  node: WorkflowNodeDefinition,
): void {
  deps.openWorkflowNodeInstructionsEditor?.(workflowId, node.id, node.instructions ?? "")
  deps.selectWorkflowCanvas(workflowId)
}
