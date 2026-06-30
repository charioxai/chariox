const suppressedAgentPaneNoticePatterns = [
  /^A queued message from attachment `[^`]+` was added to agent `[^`]+` in session `[^`]+` as `[^`]+`\. Queue depth is now \d+\.$/i,
  /^Attachment `[^`]+` steered queued prompt `[^`]+` to agent `[^`]+`\.$/i,
  /^Attachment `[^`]+` cancelled queued prompt `[^`]+` for agent `[^`]+`\.$/i,
  /^Attachment `[^`]+` updated queued prompt `[^`]+` for agent `[^`]+`\.$/i,
]

export function runtimeNoticeShouldRenderInAgentPane(message: string): boolean {
  const trimmed = message.trim()
  return trimmed.length > 0
    && !suppressedAgentPaneNoticePatterns.some((pattern) => pattern.test(trimmed))
}
