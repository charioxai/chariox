import type { AgentPromptState, WorkspaceLiveSyncStatus } from "./kernel-types.js"

export function badge(parts: Array<{ label: string; tone: "idle" | "working" | "disconnected" | "error" }>) {
  return {
    label: parts.map((part) => part.label).join(" "),
    tone: parts.some((part) => part.tone === "working")
      ? "working"
      : parts[0]?.tone ?? "idle",
    parts,
  }
}

export function malformedRuntimeValue<T>(value: string): T {
  return value as unknown as T
}

export function workspaceLiveSyncStatus(
  footerState: WorkspaceLiveSyncStatus["footer_state"],
): WorkspaceLiveSyncStatus {
  return {
    session_id: "session-1",
    mode: footerState === "off" ? "unrestricted" : "managed",
    footer_state: footerState,
    sync_groups: [],
    targets: [],
    conflicts: [],
    ignore: {
      rules: [],
      force_excludes: [],
    },
  }
}
