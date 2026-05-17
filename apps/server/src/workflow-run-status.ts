export function isTerminalWorkflowRunStatus(status: string) {
  return ["completed", "failed", "stopped"].includes(status.toLowerCase())
}
