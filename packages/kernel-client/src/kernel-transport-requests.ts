export type KernelTransportRequest =
  | {
    type: "request"
    request_id: string
    command_id: string
    request: unknown
    kernel?: string
  }
  | {
    type: "subscribe"
    request_id: string
    session_id: string
    attachment_id: string
    subscription_scope: string | null
    resume_from_event_id: number | null
    heartbeat_interval_ms?: number
    kernel?: string
  }
  | {
    type: "unsubscribe"
    request_id: string
  }

export type KernelTransportControlRequest =
  | {
    type: "subscribe"
    session_id: string
    attachment_id: string
    subscription_scope?: string | null
    resume_from_event_id?: number | null
    heartbeat_interval_ms?: number
  }
  | { type: "unsubscribe" }

export type KernelRequestRoute = { kernel?: string | undefined; commandId?: string | undefined }

export function normalizeWebSocketRequest(requestId: string, request: unknown, route: KernelRequestRoute = {}): KernelTransportRequest {
  const transportRequest = extractTransportRequest(request)
  if (transportRequest?.type === "subscribe") {
    return {
      type: "subscribe",
      request_id: requestId,
      session_id: transportRequest.session_id,
      attachment_id: transportRequest.attachment_id,
      subscription_scope: transportRequest.subscription_scope ?? null,
      resume_from_event_id: transportRequest.resume_from_event_id ?? null,
      ...(transportRequest.heartbeat_interval_ms === undefined
        ? {}
        : { heartbeat_interval_ms: transportRequest.heartbeat_interval_ms }),
      ...(route.kernel === undefined ? {} : { kernel: route.kernel }),
    }
  }
  if (transportRequest?.type === "unsubscribe") {
    return {
      type: "unsubscribe",
      request_id: requestId,
    }
  }
  return {
    type: "request",
    request_id: requestId,
    command_id: route.commandId ?? requestId,
    request,
    ...(route.kernel === undefined ? {} : { kernel: route.kernel }),
  }
}

export function extractTransportRequest(request: unknown): KernelTransportControlRequest | null {
  if (!request || typeof request !== "object") {
    return null
  }
  const value = (request as { __kernel_transport?: unknown }).__kernel_transport
  if (!value || typeof value !== "object") {
    return null
  }
  const transport = value as Record<string, unknown>
  if (
    transport.type === "subscribe"
    && typeof transport.session_id === "string"
    && typeof transport.attachment_id === "string"
  ) {
    return {
      type: "subscribe",
      session_id: transport.session_id,
      attachment_id: transport.attachment_id,
      subscription_scope: typeof transport.subscription_scope === "string" ? transport.subscription_scope : null,
      resume_from_event_id:
        typeof transport.resume_from_event_id === "number" ? transport.resume_from_event_id : null,
      ...(typeof transport.heartbeat_interval_ms === "number"
        ? { heartbeat_interval_ms: transport.heartbeat_interval_ms }
        : {}),
    }
  }
  if (transport.type === "unsubscribe") {
    return { type: "unsubscribe" }
  }
  return null
}
