import type { KernelAccessRequester } from "@chariox/kernel-client/kernel-types"

/** Keep paths quoted and controls/bidi characters visible, including old kernels. */
export function requesterLabels(requester: KernelAccessRequester | null | undefined): string[] {
  if (!requester) return ["Requester identity unavailable"]
  const harness = requester.provider_harness === "codex" ? "Codex"
    : requester.provider_harness === "claude" ? "Claude Code"
    : requester.provider_harness === "opencode" ? "OpenCode" : "External program"
  const path = JSON.stringify(requester.executable).replace(/[\u007f-\u009f\u061c\u200e\u200f\u2028-\u202e\u2066-\u2069]/g,
    char => `\\u${char.charCodeAt(0).toString(16).padStart(4, "0")}`)
  return [`Requester: ${harness} · PID ${requester.pid}`, `Executable: ${path}`,
    `Process start: ${requester.process_start_id} · exec version ${requester.process_exec_version}`]
}
