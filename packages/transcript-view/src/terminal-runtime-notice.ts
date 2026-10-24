export type TerminalRuntimeNoticeEffect = "failure" | "info"

const terminalFailureNoticePatterns = [
  /^Provider launch `[^`]+` failed before it became ready:/i,
  /^Prompt dispatch failed after acknowledgement:/i,
  /^Remote prompt dispatch failed after acknowledgement:/i,
  /^Provider prompt dispatch failed:/i,
  /^Provider run `[^`]+` for `[^`]+` ended unexpectedly[.:]/i,
  /^Provider exited before the prompt was delivered:/i,
  /^Workflow run `[^`]+` failed after provider turn failure:/i,
  /\b(?:access|refresh|oauth) token\b.*\b(?:could not be refreshed|failed to refresh|expired|invalid|revoked)\b/i,
  /\b(?:authentication|authorization)\b.*\b(?:failed|invalid|expired|revoked|unauthorized)\b/i,
  /^(?:unauthorized|invalid api key|api key (?:is )?invalid)\b/i,
]

const terminalPaneSuppressedNoticePatterns = [
  /^A queued message from attachment `[^`]+` was added to agent `[^`]+` in session `[^`]+` as `[^`]+`\. Queue depth is now \d+\.$/i,
]

// MP-08 / MP-09 / MP-10 / MP-11 G17: kernel lifecycle notices are public status.
const terminalTaskStatusNoticePatterns = [
  /^Long wait: task .+ has made no progress for 15 minutes$/i,
  /^Blocked: /i,
]

const terminalWorkflowCorrectiveOutcomePatterns = [
  /^Workflow run `[^`]+` produced no structured output on attempt \d+\/\d+; a corrective turn was scheduled\.$/i,
  /^Workflow run `[^`]+` final output failed validation on attempt \d+\/\d+; a corrective turn was scheduled:/i,
]

export function terminalRuntimeNoticeEffect(message: string): TerminalRuntimeNoticeEffect {
  return terminalFailureNoticePatterns.some((pattern) => pattern.test(message.trim()))
    || terminalRuntimeNoticeJsonError(message)
    ? "failure"
    : "info"
}

export function terminalRuntimeNoticeShouldRenderInAgentPane(message: string): boolean {
  const trimmed = message.trim()
  return trimmed.length > 0
    && (terminalRuntimeNoticeEffect(trimmed) === "failure"
      || terminalWorkflowCorrectiveOutcomePatterns.some((pattern) => pattern.test(trimmed))
      || terminalRuntimeNoticeIsTaskStatus(trimmed))
    && !terminalPaneSuppressedNoticePatterns.some((pattern) => pattern.test(trimmed))
}

export function terminalRuntimeNoticeIsTaskStatus(message: string): boolean {
  return terminalTaskStatusNoticePatterns.some((pattern) => pattern.test(message.trim()))
}

export function terminalRuntimeNoticeEntryRole(message: string): "assistant" | "error" | "status" {
  if (terminalRuntimeNoticeEffect(message) === "failure") {
    return "error"
  }
  return terminalWorkflowCorrectiveOutcomePatterns.some((pattern) => pattern.test(message.trim()))
    ? "assistant"
    : "status"
}

export function terminalRuntimeNoticeEntryKey(agentId: string, providerRunId: string | null, message: string): string {
  return `runtime-notice:${providerRunId ?? "session"}:${agentId}:${terminalRuntimeNoticeMessageHash(message)}`
}

export function terminalRuntimeNoticeMessageHash(message: string): string {
  let hash = 0
  for (const char of message) {
    hash = (hash * 31 + char.charCodeAt(0)) >>> 0
  }
  return hash.toString(16)
}

function terminalRuntimeNoticeJsonError(message: string): boolean {
  let parsed: unknown
  try {
    parsed = JSON.parse(message)
  } catch {
    return false
  }
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    return false
  }
  const record = parsed as Record<string, unknown>
  if (record.type === "error") {
    return true
  }
  const status = record.status
  if (typeof status === "number" && status >= 400) {
    return true
  }
  const error = record.error
  return Boolean(error && typeof error === "object" && !Array.isArray(error))
}
