import type { SliceDisplayEndpoint, SliceRecord } from "@chariox/kernel-client/kernel-types"

export type SliceViewerAvailability =
  | { state: "check_required"; endpointKind: SliceDisplayEndpoint["kind"] }
  | { state: "available"; endpointKind: SliceDisplayEndpoint["kind"] }
  | { state: "unavailable"; message: string }
  | { state: "error"; message: string }

export type SliceViewerEndpointResult =
  | { endpoint: SliceDisplayEndpoint }
  | { error: unknown }

export function evaluateSliceViewerAvailability(
  slice: SliceRecord,
  endpointResult?: SliceViewerEndpointResult,
): SliceViewerAvailability {
  if (slice.display_mode !== "headed") {
    return {
      state: "unavailable",
      message: `slice ${slice.name} (id=${slice.id}) is headless and has no graphical display; bind or select a headed slice with /room bind <slice-ref>`,
    }
  }

  if (slice.status !== "running") {
    if (slice.status === "unhealthy") {
      const detail = safeSliceViewerDiagnostic(slice.last_error)
      return {
        state: "error",
        message: `slice ${slice.name} (id=${slice.id}) is unhealthy${detail ? `: ${detail}` : ""}; inspect /slice logs ${slice.id} and check the worker and relay`,
      }
    }
    const action = slice.status === "stopped"
      ? `start it with /slice start ${slice.id}`
      : `wait for the slice to finish ${slice.status}`
    const detail = safeSliceViewerDiagnostic(slice.last_error)
    return {
      state: "unavailable",
      message: `slice ${slice.name} (id=${slice.id}) is ${slice.status}${detail ? `: ${detail}` : ""}; ${action}`,
    }
  }

  const configuredEndpoint = slice.display_endpoint
  if (!configuredEndpoint) {
    return {
      state: "error",
      message: `kernel reports no display endpoint for headed slice ${slice.name} (id=${slice.id}); inspect /slice logs ${slice.id} and check the worker`,
    }
  }
  if (configuredEndpoint.slice_id !== slice.id) {
    return {
      state: "error",
      message: `kernel display metadata for slice ${slice.id} refers to ${configuredEndpoint.slice_id}; reload the slice record before opening its display`,
    }
  }

  if (!endpointResult) return { state: "check_required", endpointKind: configuredEndpoint.kind }
  if ("error" in endpointResult) {
    const detail = safeSliceViewerDiagnostic(endpointResult.error)
    if (isSliceViewerAdmissionDenied(detail)) {
      return {
        state: "error",
        message: `kernel denied display admission for slice ${slice.name} (id=${slice.id}): ${detail}; reconnect with the relay client identity bound to this Room`,
      }
    }
    const staleOrOffline = isSliceViewerFailureStaleOrOffline(endpointResult.error)
    return {
      state: "error",
      message: staleOrOffline
        ? `slice ${slice.name} (id=${slice.id}) display worker appears stale or offline: ${detail}; check the worker and relay, then retry the display command`
        : `kernel could not resolve the display endpoint for slice ${slice.name} (id=${slice.id}): ${detail}; check the worker and relay, then retry the display command`,
    }
  }

  const endpoint = endpointResult.endpoint
  if (endpoint.slice_id !== slice.id) {
    return {
      state: "error",
      message: `kernel returned a display endpoint for slice ${endpoint.slice_id} while checking ${slice.name} (id=${slice.id}); refresh the Room binding before opening its display`,
    }
  }
  if (endpoint.kind !== configuredEndpoint.kind) {
    return {
      state: "error",
      message: `kernel display endpoint kind changed for slice ${slice.name} (id=${slice.id}); reload the slice record and retry`,
    }
  }
  return { state: "available", endpointKind: endpoint.kind }
}

export function formatSliceViewerAvailability(availability: SliceViewerAvailability): string {
  if (availability.state === "available") {
    return `viewer=available display=${availability.endpointKind}`
  }
  if (availability.state === "check_required") {
    return `viewer=not checked display=${availability.endpointKind}; use /room view to open`
  }
  return `viewer=${availability.state} ${availability.message}`
}

export function validateRoomBoundSliceIdentity(
  sessionId: string,
  binding: {
    session_id: string
    slice_id: string
    owner_kernel_id: string
    worker_kernel_ref: string
  },
  slice: SliceRecord,
): string | null {
  if (binding.session_id !== sessionId) {
    return "Room Environment slice binding belongs to a different session"
  }
  if (binding.slice_id !== slice.id) {
    return `Room Environment slice identity mismatch: binding names ${binding.slice_id}, but the kernel returned ${slice.id}`
  }
  if (binding.owner_kernel_id !== slice.owner_kernel_id) {
    return `Room Environment slice ${slice.id} belongs to a different owner kernel; refresh the Room binding`
  }
  if (binding.worker_kernel_ref !== slice.worker_kernel_ref) {
    return `Room Environment slice ${slice.id} has a different worker identity; refresh the Room binding`
  }
  return null
}

export function isSliceViewerFailureStaleOrOffline(error: unknown): boolean {
  const message = safeSliceViewerDiagnostic(error)
  return /offline|stale|not connected|connection (?:closed|refused|reset)|timed? ?out|timeout|peer.{0,40}(?:missing|unavailable|not found)|worker.{0,40}unavailable/i.test(message)
}

function isSliceViewerAdmissionDenied(message: string): boolean {
  return /viewer key does not match the authenticated relay client|remote viewer admission requires a key-bound relay identity/i.test(message)
}

export function safeSliceViewerDiagnostic(error: unknown): string {
  const message = error instanceof Error ? error.message : typeof error === "string" ? error : ""
  return message
    .replace(/\b(?:https?|wss?):\/\/\S+/gi, "[endpoint]")
    .replace(/\b(token|secret|password|access_token|refresh_token|authorization)=\S+/gi, "$1=[redacted]")
    .replace(/\bBearer\s+\S+/gi, "Bearer [redacted]")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, 240)
}
