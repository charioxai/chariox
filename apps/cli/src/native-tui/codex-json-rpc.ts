export type CodexJsonRpcMessage = {
  id?: unknown
  method?: string
  params?: Record<string, unknown>
  result?: Record<string, unknown>
  error?: unknown
}

export function parseCodexJsonRpcMessage(raw: { toString(): string }): CodexJsonRpcMessage | null {
  try {
    return JSON.parse(raw.toString()) as CodexJsonRpcMessage
  } catch {
    return null
  }
}

export function isCodexKernelInitialize(message: CodexJsonRpcMessage): boolean {
  const clientInfo = message.params?.clientInfo
  if (!clientInfo || typeof clientInfo !== "object") return false
  const name = (clientInfo as Record<string, unknown>).name
  return typeof name === "string" && name.includes("chariox")
}

export function extractCodexThreadId(message: CodexJsonRpcMessage): string | null {
  const thread = message.result?.thread
  if (thread && typeof thread === "object" && "id" in thread && typeof thread.id === "string") {
    return thread.id
  }
  const id = message.result?.id
  return typeof id === "string" ? id : null
}
