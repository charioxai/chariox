import type {
  QueuedWorkflowLaunch,
  RuntimeSession,
  WorkflowDefinition,
  WorkflowEndpointDefinition,
  WorkflowRun,
} from "./cli-types.js"

type FooterTone = "info" | "error"

export type WorkflowRunInvokePayload = {
  workflow: WorkflowDefinition
  endpoint: WorkflowEndpointDefinition
  session: RuntimeSession
} & ({ workflow_run: WorkflowRun } | { queued_launch: QueuedWorkflowLaunch })

export type WorkflowInvokeCommandContext = {
  firstWorkflowArgIsExplicit: (workflowRef: string | undefined) => boolean
  workflowRefOrSelected: (workflowRef: string | null | undefined) => string | null
}

export type WorkflowInvokeCommandDeps = {
  invokeWorkflowEndpoint?: (
    workflowRef: string,
    endpointRef: string,
    prompt?: string | null,
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
  const prompt = args.slice(explicitWorkflowRef ? 3 : 2).join(" ").trim()
  if (!workflowRef || !endpointRef) {
    deps.flashFooter("usage: /workflow run|start [workflow-ref] <endpoint-ref> [prompt]", "error")
    return
  }
  if (!deps.invokeWorkflowEndpoint) {
    deps.flashFooter("workflow runtime commands unavailable", "error")
    return
  }
  const payload = await deps.invokeWorkflowEndpoint(workflowRef, endpointRef, prompt || null)
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
      `queued workflow launch ${payload.queued_launch.id}; active workflow run in session`,
      "info",
    )
  }
}
