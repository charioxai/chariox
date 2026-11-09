import { userDomainRefusalReason } from "./user-domain-refusal.js"
/** The kernel wrote the whole sentence for this refusal; the user reads it as it is. */
const roomToolRefusedCode = "room_tool_refused"
export class LocalIpcError extends Error {
  get refusalReason() { return userDomainRefusalReason(this.code) }
  constructor(
    readonly operation: string,
    message: string,
    readonly code: string | null = null,
    readonly retryable = false,
  ) {
    super(code === roomToolRefusedCode
      ? message
      : `kernel transport \`${operation}\` failed: ${message}${userDomainRefusalReason(code) ? " (reason=" + userDomainRefusalReason(code) + ")" : ""}`)
    this.name = "LocalIpcError"
  }
}
