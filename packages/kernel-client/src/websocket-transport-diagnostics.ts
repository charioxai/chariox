export function isWebSocketEndpoint(value: string) {
  return value.startsWith("ws://") || value.startsWith("wss://")
}

export function formatTransportError(error: unknown, endpoint: string): string {
  const message = extractTransportErrorMessage(error)
  if (message) {
    return message
  }
  return `websocket error at ${endpoint}`
}

export function extractTransportErrorMessage(error: unknown): string | null {
  if (error instanceof Error && error.message.trim()) {
    return error.message
  }
  if (typeof error === "string" && error.trim()) {
    return error
  }
  if (!error || typeof error !== "object") {
    return null
  }

  const fields = error as Record<string, unknown>
  const nested = extractTransportErrorMessage(fields.error)
  if (nested) {
    return nested
  }
  if (typeof fields.message === "string" && fields.message.trim()) {
    return fields.message
  }
  if (typeof fields.reason === "string" && fields.reason.trim()) {
    return fields.reason
  }
  if (typeof fields.code === "string" && fields.code.trim()) {
    return fields.code
  }
  if (typeof fields.type === "string" && fields.type.trim() && fields.type !== "error") {
    return `websocket ${fields.type}`
  }
  return null
}
