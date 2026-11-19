/**
 * Printed after the provider's own terminal ends. The provider prints its own
 * resume command there, which bypasses Chariox (Claude, OpenCode) or names a
 * proxy that no longer exists (Codex); this is the command that works.
 */
export function formatNativeReopenHint(provider: "claude" | "codex" | "opencode", sessionId: string): string {
  return `\n[chariox] To reopen this Room: chariox ${provider} ${sessionId}\n`
}
