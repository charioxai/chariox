import type {
  RoomTerminalOperation,
  RoomTerminalRequest,
  RoomTerminalResponse,
  RoomTerminalResult,
} from "./kernel-types-room-terminal.js"

// MP-08 / MP-10 / MP-11: the Room terminal service is provisional protocol 569.
export const roomTerminalMinimumProtocolVersion = 569

/** Which terminal a request is about: the Room, and the machine that runs it. */
export type RoomTerminalTarget = {
  readonly sessionId: string
  /** Speak for this Room agent instead of the signed-in user. */
  readonly actingAgent?: string
  /** Run on the machine where this Room agent runs; clients echo `host_agent` back. */
  readonly hostAgent?: string
}

export function roomTerminalRequest(target: RoomTerminalTarget, operation: RoomTerminalOperation) {
  const request: RoomTerminalRequest = {
    session_id: target.sessionId,
    ...(target.actingAgent ? { acting_agent: target.actingAgent } : {}),
    ...(target.hostAgent ? { host_agent: target.hostAgent } : {}),
    operation,
  }
  return { RoomTerminal: request } as const
}

export function openRoomTerminalRequest(
  target: RoomTerminalTarget,
  size: { readonly cols: number; readonly rows: number },
  options: { readonly title?: string; readonly command?: readonly string[] } = {},
) {
  return roomTerminalRequest(target, {
    operation: "open",
    cols: size.cols,
    rows: size.rows,
    ...(options.title ? { title: options.title } : {}),
    ...(options.command ? { command: options.command } : {}),
  })
}

export function listRoomTerminalsRequest(target: RoomTerminalTarget) {
  return roomTerminalRequest(target, { operation: "list" })
}

export function roomTerminalSnapshotRequest(target: RoomTerminalTarget, terminalId: string) {
  return roomTerminalRequest(target, { operation: "snapshot", terminal_id: terminalId })
}

function base64Of(data: Uint8Array): string {
  let binary = ""
  for (let offset = 0; offset < data.length; offset += 0x8000) {
    binary += String.fromCharCode(...data.subarray(offset, offset + 0x8000))
  }
  return btoa(binary)
}

export function roomTerminalInputRequest(target: RoomTerminalTarget, terminalId: string, data: Uint8Array) {
  return roomTerminalRequest(target, {
    operation: "input",
    terminal_id: terminalId,
    data_base64: base64Of(data),
  })
}

export function resizeRoomTerminalRequest(
  target: RoomTerminalTarget,
  terminalId: string,
  size: { readonly cols: number; readonly rows: number },
) {
  return roomTerminalRequest(target, { operation: "resize", terminal_id: terminalId, cols: size.cols, rows: size.rows })
}

export function readRoomTerminalOutputRequest(
  target: RoomTerminalTarget,
  terminalId: string,
  afterOffset: number,
  waitMs = 0,
) {
  return roomTerminalRequest(target, {
    operation: "read_output",
    terminal_id: terminalId,
    after_offset: afterOffset,
    ...(waitMs > 0 ? { wait_ms: waitMs } : {}),
  })
}

export function closeRoomTerminalRequest(target: RoomTerminalTarget, terminalId: string) {
  return roomTerminalRequest(target, { operation: "close", terminal_id: terminalId })
}

/** The kernel answered and said no (not a lost connection): retrying will not help. */
export class RoomTerminalRefusedError extends Error {}

/** The one result of a reply to `roomTerminalRequest`, or the kernel's reason it refused. */
export function roomTerminalResult(response: unknown): RoomTerminalResult {
  const reply = response as Partial<RoomTerminalResponse> & { Error?: { message?: string } }
  if (reply.RoomTerminal?.result) return reply.RoomTerminal.result
  throw new RoomTerminalRefusedError(reply.Error?.message ?? "The kernel returned no terminal result.")
}

/** Pick the result variant a request expects; anything else is a protocol error. */
export function expectRoomTerminalResult<K extends RoomTerminalResult["result"]>(
  response: unknown,
  kind: K,
): Extract<RoomTerminalResult, { result: K }> {
  const result = roomTerminalResult(response)
  if (result.result !== kind) throw new Error(`Expected a ${kind} terminal result, got ${result.result}.`)
  return result as Extract<RoomTerminalResult, { result: K }>
}
