export class LocalIpcError extends Error {
  constructor(
    readonly operation: string,
    message: string,
    readonly code: string | null = null,
    readonly retryable = false,
  ) {
    super(`kernel transport \`${operation}\` failed: ${message}`)
    this.name = "LocalIpcError"
  }
}
