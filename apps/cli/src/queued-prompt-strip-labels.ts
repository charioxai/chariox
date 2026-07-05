import { queuedPromptMetaLabel as sharedQueuedPromptMetaLabel } from "@arroba/kernel-client/queued-prompt-controls"

export function queuedPromptTitleLabel(count: number, focused: boolean): string {
  const countLabel = `QUEUE • ${count} prompt${count === 1 ? "" : "s"}`
  return focused
    ? `${countLabel} • J/K select • S steer • C cancel`
    : countLabel
}

export function queuedPromptActionLabel(action: "steer" | "cancel", focusedPrimary: boolean): string {
  if (!focusedPrimary) {
    return action
  }
  return action === "steer" ? "S" : "C"
}

export function queuedPromptMetaLabel(item: { readonly status: string; readonly attachmentCount: number }): string {
  return sharedQueuedPromptMetaLabel(item)
}
