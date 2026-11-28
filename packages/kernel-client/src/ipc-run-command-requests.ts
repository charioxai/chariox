import type { CommandTarget, RunCommandOperation, RunCommandRequest } from "./kernel-types-run-command.js"

export const runCommandMinimumProtocolVersion = 1017

export function runCommandRequest(target: CommandTarget, operation: RunCommandOperation, actingAgent?: string): { RunCommand: RunCommandRequest } {
  return { RunCommand: { target, operation, ...(actingAgent ? { acting_agent: actingAgent } : {}) } }
}
