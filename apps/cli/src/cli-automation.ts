import { createServer, type Server as NetServer, type Socket as NetSocket } from "node:net"
import { unlink } from "node:fs/promises"

export type CliAutomationRequest = {
  id?: unknown
  action?: unknown
  attachments?: unknown
  command?: unknown
  prompt?: unknown
  screen?: unknown
  choiceIndex?: unknown
  delta?: unknown
  interactionId?: unknown
  reply?: unknown
  editing?: unknown
  daemonDisconnected?: unknown
  sessionId?: unknown
  statusLine?: unknown
  intervalMs?: unknown
  timeoutMs?: unknown
  selectedWorkflowAlias?: unknown
  shellEntryCount?: unknown
  workflowAlias?: unknown
  entryId?: unknown
  turnId?: unknown
  agentId?: unknown
  collapsed?: unknown
  promptId?: unknown
  queuedPromptAction?: unknown
  externalSessionId?: unknown
  externalSessionIndex?: unknown
  machineRef?: unknown
  kernelRef?: unknown
  focus?: unknown
  providerId?: unknown
  modelId?: unknown
  effort?: unknown
}

export type CliAutomationResponse = {
  id: string | number | null
  ok: boolean
  data?: unknown
  error?: string
}

export type CliAutomationSnapshot = {
  [key: string]: unknown
  screen?: unknown
  daemonDisconnected?: unknown
  statusLine?: unknown
  session?: Record<string, unknown> & { id?: unknown }
  selectedWorkflow?: (Record<string, unknown> & { alias?: unknown }) | null
  workflows?: Array<Record<string, unknown> & { alias?: unknown }>
  interactions?: Array<Record<string, unknown> & { id?: unknown; agentId?: unknown }>
  shell?: Record<string, unknown> & { entries?: unknown[] }
  transcript?: Record<string, unknown> & { entries?: unknown[] }
  agentPanes?: Record<string, Array<Record<string, unknown>>>
  queuedPromptStrips?: Record<string, {
    selectedIndex?: unknown
    items?: Array<Record<string, unknown>>
  }>
}

export type CliAutomationServer = NetServer

type StartCliAutomationServerOptions = {
  socketPath: string
  handleRequest: (request: CliAutomationRequest) => Promise<unknown> | unknown
  formatError: (error: unknown) => string
  onListening?: (socketPath: string) => void
}

export function automationSnapshotMatches(
  snapshot: CliAutomationSnapshot,
  request: CliAutomationRequest,
): boolean {
  if (typeof request.screen === "string" && snapshot.screen !== request.screen) {
    return false
  }
  if (typeof request.daemonDisconnected === "boolean" && snapshot.daemonDisconnected !== request.daemonDisconnected) {
    return false
  }
  if (typeof request.sessionId === "string" && snapshot.session?.id !== request.sessionId) {
    return false
  }
  if (typeof request.statusLine === "string" && snapshot.statusLine !== request.statusLine) {
    return false
  }
  if (typeof request.selectedWorkflowAlias === "string" && snapshot.selectedWorkflow?.alias !== request.selectedWorkflowAlias) {
    return false
  }
  if (typeof request.workflowAlias === "string" && !(snapshot.workflows ?? []).some((workflow) => workflow.alias === request.workflowAlias)) {
    return false
  }
  if (typeof request.shellEntryCount === "number" && (snapshot.shell?.entries?.length ?? 0) < request.shellEntryCount) {
    return false
  }
  return true
}

export async function startCliAutomationServer({
  socketPath,
  handleRequest,
  formatError,
  onListening,
}: StartCliAutomationServerOptions): Promise<CliAutomationServer> {
  await unlink(socketPath).catch((error: NodeJS.ErrnoException) => {
    if (error.code !== "ENOENT") {
      throw error
    }
  })
  const server = createServer((socket) => {
    socket.setEncoding("utf8")
    let buffer = ""
    socket.on("data", (chunk) => {
      buffer += chunk
      while (buffer.includes("\n")) {
        const newlineIndex = buffer.indexOf("\n")
        const line = buffer.slice(0, newlineIndex).trim()
        buffer = buffer.slice(newlineIndex + 1)
        if (!line) {
          continue
        }
        dispatchAutomationLine(socket, line, handleRequest, formatError)
      }
    })
  })
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject)
    server.listen(socketPath, () => {
      server.off("error", reject)
      resolve()
    })
  })
  onListening?.(socketPath)
  return server
}

export function stopCliAutomationServer(server: CliAutomationServer, socketPath: string): void {
  server.close()
  void unlink(socketPath).catch(() => {})
}

function dispatchAutomationLine(
  socket: NetSocket,
  line: string,
  handleRequest: (request: CliAutomationRequest) => Promise<unknown> | unknown,
  formatError: (error: unknown) => string,
): void {
  let request: CliAutomationRequest
  try {
    request = JSON.parse(line) as CliAutomationRequest
  } catch (error) {
    sendAutomationResponse(socket, {
      id: null,
      ok: false,
      error: `invalid JSON automation request: ${formatError(error)}`,
    })
    return
  }
  const id = typeof request.id === "string" || typeof request.id === "number" ? request.id : null
  void Promise.resolve(handleRequest(request))
    .then((data) => sendAutomationResponse(socket, { id, ok: true, data }))
    .catch((error) => sendAutomationResponse(socket, { id, ok: false, error: formatError(error) }))
}

function sendAutomationResponse(socket: NetSocket, response: CliAutomationResponse): void {
  socket.write(`${JSON.stringify(response)}\n`)
}
