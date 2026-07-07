import type {
  DaemonHealthProjection,
} from "./kernel-types.js"

type CommandLaneKind = "session" | "agent" | "workflow" | "provider"

type CommandLaneKindSummary = {
  lanes: number
  queued: number
  saturatedLanes: CommandLaneIssue[]
}

type CommandLaneIssue = {
  kind: CommandLaneKind
  laneId: string
  queued: number
  limit: number
}

export function commandLaneHealthIssueCount(health: DaemonHealthProjection): number {
  return commandLaneHealthSummary(health).saturated
}

export function commandLaneHealthSummary(health: DaemonHealthProjection): {
  session: CommandLaneKindSummary
  agent: CommandLaneKindSummary
  workflow: CommandLaneKindSummary
  provider: CommandLaneKindSummary
  saturated: number
  saturatedLanes: CommandLaneIssue[]
} {
  const session = summarizeCommandLanes("session", health.session_command_lanes)
  const agent = summarizeCommandLanes("agent", health.agent_command_lanes)
  const workflow = summarizeCommandLanes("workflow", health.workflow_command_lanes)
  const provider = summarizeCommandLanes("provider", health.provider_runtime_lanes)
  const saturatedLanes = [
    ...session.saturatedLanes,
    ...agent.saturatedLanes,
    ...workflow.saturatedLanes,
    ...provider.saturatedLanes,
  ]
  return {
    session,
    agent,
    workflow,
    provider,
    saturated: saturatedLanes.length,
    saturatedLanes,
  }
}

export function commandLaneInspectionTargets(lanes: readonly CommandLaneIssue[]): string {
  const targets = lanes.slice(0, 4).map(commandLaneInspectionTarget)
  if (lanes.length > 4) {
    targets.push(`${lanes.length - 4} more lane${lanes.length - 4 === 1 ? "" : "s"}`)
  }
  return targets.length > 0 ? targets.join(", ") : "stuck sessions/agents"
}

function commandLaneInspectionTarget(lane: CommandLaneIssue): string {
  switch (lane.kind) {
    case "session":
      return `session ${lane.laneId}`
    case "agent":
      return `agent ${lane.laneId}`
    case "workflow":
      return `workflow ${lane.laneId}`
    case "provider":
      return `provider run ${lane.laneId}`
  }
}

function summarizeCommandLanes(
  kind: CommandLaneKind,
  lanes: readonly { lane_id: string; queue_limit: number; queued_commands: number }[],
): CommandLaneKindSummary {
  const saturatedLanes = lanes
    .filter((lane) => lane.queue_limit > 0 && lane.queued_commands >= lane.queue_limit)
    .map((lane) => ({
      kind,
      laneId: lane.lane_id,
      queued: lane.queued_commands,
      limit: lane.queue_limit,
    }))
  return {
    lanes: lanes.length,
    queued: lanes.reduce((sum, lane) => sum + lane.queued_commands, 0),
    saturatedLanes,
  }
}
