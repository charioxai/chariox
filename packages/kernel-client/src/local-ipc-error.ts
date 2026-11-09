import { userDomainRefusalReason } from "./user-domain-refusal.js"
/** The kernel wrote the whole sentence for these failures; the user reads it as it is. */
const roomToolCodes = new Set(["room_tool_refused", "room_tool_retry"])
export class LocalIpcError extends Error {
  get refusalReason() { return userDomainRefusalReason(this.code) }
  constructor(
    readonly operation: string,
    message: string,
    readonly code: string | null = null,
    readonly retryable = false,
  ) {
    super(code !== null && roomToolCodes.has(code)
      ? message
      : `kernel transport \`${operation}\` failed: ${message}${userDomainRefusalReason(code) ? " (reason=" + userDomainRefusalReason(code) + ")" : ""}`)
    this.name = "LocalIpcError"
  }
}
