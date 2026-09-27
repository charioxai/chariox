import type { RoomEnvironmentSliceBinding } from "./kernel-types.js"

export type ScopedSliceViewerTarget = {
  sessionId: string
  agentId: string
  sliceId: string
}

export function buildHostedCloudViewUrl(apiUrl: string, target: ScopedSliceViewerTarget): string {
  const url = new URL("/view", apiUrl)
  url.searchParams.set("view_target", `${target.sessionId}:${target.agentId}:${target.sliceId}`)
  return url.toString()
}

export function scopedSliceViewerTarget(options: {
  sessionId?: string | null
  attachmentId?: string | null
  agentId?: string | null
  sliceId: string
  binding: RoomEnvironmentSliceBinding | null
}): { target: ScopedSliceViewerTarget; error: null } | { target: null; error: string } {
  if (!options.sessionId || !options.attachmentId || !options.agentId) {
    return { target: null, error: "Selkies slice screen requires an active Room session, attachment, and focused agent" }
  }
  if (!options.binding) {
    return { target: null, error: "Room Environment has no bound slice to view" }
  }
  if (options.binding.session_id !== options.sessionId) {
    return { target: null, error: "Room Environment slice binding belongs to a different session" }
  }
  if (options.binding.slice_id !== options.sliceId) {
    return { target: null, error: `Room Environment is bound to slice ${options.binding.slice_id}, not ${options.sliceId}` }
  }
  return {
    target: { sessionId: options.sessionId, agentId: options.agentId, sliceId: options.sliceId },
    error: null,
  }
}
