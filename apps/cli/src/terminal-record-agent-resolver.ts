import type {
  TerminalOutputRecord,
} from "./cli-types.js"

type TerminalRecordAgentResolverOptions = {
  record: Pick<TerminalOutputRecord, "agent_id">
}

export function resolveTerminalRecordAgentId(options: TerminalRecordAgentResolverOptions): string | null {
  if (options.record.agent_id) {
    return options.record.agent_id
  }
  return null
}
