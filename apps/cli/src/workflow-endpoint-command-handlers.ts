import type {
  RuntimeSession,
  WorkflowDefinition,
  WorkflowEndpointDefinition,
} from "./cli-types.js"

type FooterTone = "info" | "error"

export type WorkflowEndpointPayload = {
  endpoint: WorkflowEndpointDefinition
  workflow: WorkflowDefinition
  session: RuntimeSession
}

export type WorkflowEndpointCommandContext = {
  firstWorkflowArgIsExplicit: (workflowRef: string | undefined) => boolean
  workflowRefOrSelected: (workflowRef: string | null | undefined) => string | null
}

export type WorkflowEndpointCommandDeps = {
  createWorkflowEndpoint: (
    workflowRef: string,
    entryNodeId: string,
    alias?: string | null,
  ) => Promise<WorkflowEndpointPayload>
  assignWorkflowEndpointAlias: (
    workflowRef: string,
    endpointRef: string,
    alias: string,
  ) => Promise<WorkflowEndpointPayload>
  bindWorkflowEndpoint: (
    workflowRef: string,
    endpointRef: string,
    entryNodeId: string,
  ) => Promise<WorkflowEndpointPayload>
  applySessionState: (session: RuntimeSession) => void
  selectWorkflowCanvas: (workflowId: string | null) => void
  flashFooter: (message: string, tone: FooterTone) => void
}

export async function handleWorkflowEndpointCommand(
  deps: WorkflowEndpointCommandDeps,
  context: WorkflowEndpointCommandContext,
  args: readonly string[],
): Promise<void> {
  const action = args[1]
  if (action === "new") {
    const explicitWorkflowRef = context.firstWorkflowArgIsExplicit(args[2]) ? args[2] : null
    const workflowRef = context.workflowRefOrSelected(explicitWorkflowRef)
    const entryNodeId = explicitWorkflowRef ? args[3] : args[2]
    const alias = (explicitWorkflowRef ? args[4] : args[3]) ?? null
    if (!workflowRef || !entryNodeId) {
      deps.flashFooter(
        "usage: /workflow endpoint new [workflow-ref] <entry-node-id> [alias]",
        "error",
      )
      return
    }
    const payload = await deps.createWorkflowEndpoint(workflowRef, entryNodeId, alias)
    deps.applySessionState(payload.session)
    deps.selectWorkflowCanvas(payload.workflow.id)
    deps.flashFooter(`created workflow endpoint ${payload.endpoint.id}`, "info")
    return
  }
  if (action === "alias") {
    const explicitWorkflowRef = args.length >= 5 ? args[2] : null
    const workflowRef = context.workflowRefOrSelected(explicitWorkflowRef)
    const endpointRef = explicitWorkflowRef ? args[3] : args[2]
    const alias = explicitWorkflowRef ? args[4] : args[3]
    if (!workflowRef || !endpointRef || !alias) {
      deps.flashFooter(
        "usage: /workflow endpoint alias [workflow-ref] <endpoint-ref> <alias>",
        "error",
      )
      return
    }
    const payload = await deps.assignWorkflowEndpointAlias(workflowRef, endpointRef, alias)
    deps.applySessionState(payload.session)
    deps.selectWorkflowCanvas(payload.workflow.id)
    deps.flashFooter(
      `workflow endpoint ${payload.endpoint.id} aliased as ${payload.endpoint.alias}`,
      "info",
    )
    return
  }
  if (action === "bind") {
    const explicitWorkflowRef = args.length >= 5 ? args[2] : null
    const workflowRef = context.workflowRefOrSelected(explicitWorkflowRef)
    const endpointRef = explicitWorkflowRef ? args[3] : args[2]
    const entryNodeId = explicitWorkflowRef ? args[4] : args[3]
    if (!workflowRef || !endpointRef || !entryNodeId) {
      deps.flashFooter(
        "usage: /workflow endpoint bind [workflow-ref] <endpoint-ref> <entry-node-id>",
        "error",
      )
      return
    }
    const payload = await deps.bindWorkflowEndpoint(workflowRef, endpointRef, entryNodeId)
    deps.applySessionState(payload.session)
    deps.selectWorkflowCanvas(payload.workflow.id)
    deps.flashFooter(
      `workflow endpoint ${payload.endpoint.id} bound to node ${payload.endpoint.entry_node_id}`,
      "info",
    )
    return
  }
  deps.flashFooter(
    "usage: /workflow endpoint new [workflow-ref] <entry-node-id> [alias] | alias [workflow-ref] <endpoint-ref> <alias> | bind [workflow-ref] <endpoint-ref> <entry-node-id>",
    "error",
  )
}
