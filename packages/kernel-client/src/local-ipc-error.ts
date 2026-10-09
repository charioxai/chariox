import { userDomainRefusalReason } from "./user-domain-refusal.js"
export class LocalIpcError extends Error {
  get refusalReason() { return userDomainRefusalReason(this.code) }
  constructor(
    readonly operation: string,
    message: string,
    readonly code: string | null = null,
    readonly retryable = false,
  ) {
    super(`kernel transport \`${operation}\` failed: ${message}${userDomainRefusalReason(code) ? " (reason=" + userDomainRefusalReason(code) + ")" : ""}`)
    this.name = "LocalIpcError"
  }
}
