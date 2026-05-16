import type {
  AgentInstance,
  TerminalOutputRecord,
} from "./cli-types.js"

type TerminalRecordAgentResolverOptions = {
  record: Pick<TerminalOutputRecord, "agent_id">
  streamingAgentId: string | null
  activePromptAgentId: string | null
  agents: AgentInstance[]
  focusedAgentId: string | null
}

export function resolveTerminalRecordAgentId(options: TerminalRecordAgentResolverOptions): string | null {
  if (options.record.agent_id) {
    return options.record.agent_id
  }
  const activeProcessingAgentId = options.agents.find((agent) => agent.is_processing || agent.state === "Working")?.id ?? null
  return options.streamingAgentId
    ?? options.activePromptAgentId
    ?? activeProcessingAgentId
    ?? options.focusedAgentId
}
