import type { RuntimeSession } from "./cli-types.js"

export type RootWorkflowShortcutKind = "loop" | "goal"

export type RootWorkflowShortcutDescriptor = {
  readonly kind: RootWorkflowShortcutKind
  readonly commandName: "/loop" | "/goal"
  readonly registryEntryName: string
  readonly endpointRef: string
  readonly entryNode: string
  readonly generatedNodes: readonly string[]
}

export const ROOT_WORKFLOW_SHORTCUTS: Record<RootWorkflowShortcutKind, RootWorkflowShortcutDescriptor> = {
  loop: {
    kind: "loop",
    commandName: "/loop",
    registryEntryName: "loop-until-done",
    endpointRef: "entry",
    entryNode: "worker",
    generatedNodes: ["checker"],
  },
  goal: {
    kind: "goal",
    commandName: "/goal",
    registryEntryName: "planner-worker-reviewer",
    endpointRef: "entry",
    entryNode: "planner",
    generatedNodes: ["worker", "reviewer"],
  },
}

export function focusedSessionAgent(session: RuntimeSession, focusedAgentId: string | null) {
  if (!focusedAgentId) {
    return null
  }
  return session.agents.find((agent) => agent.id === focusedAgentId || agent.agent_ref === focusedAgentId) ?? null
}

export function formatRootWorkflowRunSummary(input: {
  readonly descriptor: RootWorkflowShortcutDescriptor
  readonly focusedAgentId: string
  readonly workflowId: string | null
  readonly invocationKind: string
  readonly agentIdsByNode?: Record<string, string>
}): string {
  const generatedAgentIds = input.descriptor.generatedNodes
    .map((node) => input.agentIdsByNode?.[node])
    .filter((agentId): agentId is string => Boolean(agentId))
  const spawned = generatedAgentIds.length > 0
    ? `; spawned ${generatedAgentIds.join(", ")}`
    : ""
  return [
    `ran workflow ${input.descriptor.registryEntryName}`,
    input.workflowId ? ` as ${input.workflowId}` : "",
    `; reused ${input.focusedAgentId} as ${input.descriptor.entryNode}`,
    spawned,
    ` [${input.invocationKind}]`,
  ].join("")
}
