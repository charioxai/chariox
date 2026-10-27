import type {
  RuntimeSession,
  WorkflowDefinition,
  WorkflowEndpointDefinition,
} from "./cli-types.js"
import { parseWorkflowEndpointMaxInstances } from "./workflow-endpoint-pool-projection.js"

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
  setWorkflowEndpointMaxInstances?: (
    workflowRef: string,
    endpointRef: string,
    maxInstances: number,
  ) => Promise<WorkflowEndpointPayload>
  removeWorkflowEndpoint?: (
    workflowRef: string,
    endpointRef: string,
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
  if (action === "bind" || action === "rebind") {
    const explicitWorkflowRef = args.length >= 5 ? args[2] : null
    const workflowRef = context.workflowRefOrSelected(explicitWorkflowRef)
    const endpointRef = explicitWorkflowRef ? args[3] : args[2]
    const entryNodeId = explicitWorkflowRef ? args[4] : args[3]
    if (!workflowRef || !endpointRef || !entryNodeId) {
      deps.flashFooter(
        `usage: /workflow endpoint ${action} [workflow-ref] <endpoint-ref> <entry-node-id>`,
        "error",
      )
      return
    }
    const payload = await deps.bindWorkflowEndpoint(workflowRef, endpointRef, entryNodeId)
    deps.applySessionState(payload.session)
    deps.selectWorkflowCanvas(payload.workflow.id)
    deps.flashFooter(
      `workflow endpoint ${payload.endpoint.id} ${action === "rebind" ? "rebound" : "bound"} to node ${payload.endpoint.entry_node_id}`,
      "info",
    )
    return
  }
  if (action === "max-instances") {
    const explicitWorkflowRef = args.length >= 5 ? args[2] : null
    const workflowRef = context.workflowRefOrSelected(explicitWorkflowRef)
    const endpointRef = explicitWorkflowRef ? args[3] : args[2]
    const value = (explicitWorkflowRef ? args[4] : args[3])?.trim()
    if (!workflowRef || !endpointRef || !value) {
      deps.flashFooter(
        "usage: /workflow endpoint max-instances [workflow-ref] <endpoint-ref> <count>",
        "error",
      )
      return
    }
    if (!deps.setWorkflowEndpointMaxInstances) {
      deps.flashFooter("workflow endpoint capacity commands unavailable", "error")
      return
    }
    const maxInstances = parseWorkflowEndpointMaxInstances(value)
    if (maxInstances === null) {
      deps.flashFooter(
        "usage: /workflow endpoint max-instances [workflow-ref] <endpoint-ref> <count 1-32>",
        "error",
      )
      return
    }
    const payload = await deps.setWorkflowEndpointMaxInstances(workflowRef, endpointRef, maxInstances)
    deps.applySessionState(payload.session)
    deps.selectWorkflowCanvas(payload.workflow.id)
    deps.flashFooter(
      `workflow endpoint ${payload.endpoint.id} max-instances set to ${payload.endpoint.max_instances ?? maxInstances}`,
      "info",
    )
    return
  }
  if (action === "remove" || action === "delete") {
    const explicitWorkflowRef = context.firstWorkflowArgIsExplicit(args[2]) ? args[2] : null
    const workflowRef = context.workflowRefOrSelected(explicitWorkflowRef)
    const endpointRef = explicitWorkflowRef ? args[3] : args[2]
    if (!workflowRef || !endpointRef) {
      deps.flashFooter(
        `usage: /workflow endpoint ${action} [workflow-ref] <endpoint-ref>`,
        "error",
      )
      return
    }
    if (!deps.removeWorkflowEndpoint) {
      deps.flashFooter("workflow endpoint removal unavailable", "error")
      return
    }
    const payload = await deps.removeWorkflowEndpoint(workflowRef, endpointRef)
    deps.applySessionState(payload.session)
    deps.selectWorkflowCanvas(payload.workflow.id)
    deps.flashFooter(`removed workflow endpoint ${payload.endpoint.id}`, "info")
    return
  }
  deps.flashFooter(
    "usage: /workflow endpoint new [workflow-ref] <entry-node-id> [alias] | alias [workflow-ref] <endpoint-ref> <alias> | bind|rebind [workflow-ref] <endpoint-ref> <entry-node-id> | max-instances [workflow-ref] <endpoint-ref> <count 1-32> | remove [workflow-ref] <endpoint-ref>",
    "error",
  )
}
