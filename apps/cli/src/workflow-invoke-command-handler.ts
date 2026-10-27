import type {
  RuntimeSession,
  WorkflowDefinition,
  WorkflowEndpointDefinition,
  WorkflowQueuedPrompt,
  WorkflowRun,
} from "./cli-types.js"

type FooterTone = "info" | "error"

export type WorkflowRunInvokePayload = {
  workflow: WorkflowDefinition
  endpoint: WorkflowEndpointDefinition
  session: RuntimeSession
} & ({ workflow_run: WorkflowRun } | { queued_prompt: WorkflowQueuedPrompt })

export type WorkflowInvokeCommandContext = {
  firstWorkflowArgIsExplicit: (workflowRef: string | undefined) => boolean
  workflowRefOrSelected: (workflowRef: string | null | undefined) => string | null
}

export type WorkflowInvokeCommandDeps = {
  invokeWorkflowEndpoint?: (
    workflowRef: string,
    endpointRef: string,
    prompt?: string | null,
    queueRef?: string | null,
  ) => Promise<WorkflowRunInvokePayload>
  applySessionState: (session: RuntimeSession) => void
  upsertWorkflowDefinition: (workflow: WorkflowDefinition) => void
  selectWorkflowCanvas: (workflowId: string | null) => void
  showWorkflowScreen: () => void
  flashFooter: (message: string, tone: FooterTone) => void
}

export async function handleWorkflowInvokeCommand(
  deps: WorkflowInvokeCommandDeps,
  context: WorkflowInvokeCommandContext,
  args: readonly string[],
): Promise<void> {
  const firstArg = args[1]
  const explicitWorkflowRef = context.firstWorkflowArgIsExplicit(firstArg) ? firstArg : null
  const workflowRef = context.workflowRefOrSelected(explicitWorkflowRef)
  const endpointRef = explicitWorkflowRef ? args[2] : firstArg
  const promptArgs = args.slice(explicitWorkflowRef ? 3 : 2)
  const queueFlagIndex = promptArgs.findIndex((arg) => arg === "--queue")
  const queueRef = queueFlagIndex >= 0 ? promptArgs[queueFlagIndex + 1] : null
  const prompt = promptArgs
    .filter((_, index) => queueFlagIndex < 0 || (index !== queueFlagIndex && index !== queueFlagIndex + 1))
    .join(" ")
    .trim()
  if (!workflowRef || !endpointRef) {
    deps.flashFooter("usage: /workflow run|start [workflow-ref] <endpoint-ref> [prompt]", "error")
    return
  }
  if (!deps.invokeWorkflowEndpoint) {
    deps.flashFooter("workflow runtime commands unavailable", "error")
    return
  }
  const payload = await deps.invokeWorkflowEndpoint(workflowRef, endpointRef, prompt || null, queueRef)
  deps.applySessionState(payload.session)
  deps.upsertWorkflowDefinition(payload.workflow)
  deps.selectWorkflowCanvas(payload.workflow.id)
  deps.showWorkflowScreen()
  if ("workflow_run" in payload) {
    deps.flashFooter(
      `started workflow run ${payload.workflow_run.id} [${String(payload.workflow_run.status).toLowerCase()}]`,
      "info",
    )
  } else {
    deps.flashFooter(
      `queued workflow prompt ${payload.queued_prompt.id}`,
      "info",
    )
  }
}
