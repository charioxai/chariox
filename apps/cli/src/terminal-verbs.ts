import {
  closeRoomTerminalRequest, expectRoomTerminalResult, listRoomTerminalsRequest, openRoomTerminalRequest,
  readRoomTerminalOutputRequest, roomTerminalInputRequest, roomTerminalRequest, roomTerminalSnapshotRequest,
  type RoomTerminalTarget,
} from "@chariox/kernel-client/ipc-requests"
import { CommandFailure } from "./agent-command-table.js"
import type { SendRequest } from "./machine-files.js"

export const TERMINAL_VERBS = ["open", "list", "send", "read", "wait", "close"] as const
export type TerminalVerb = typeof TERMINAL_VERBS[number]

const DEFAULT_COLS = 120
const DEFAULT_ROWS = 30
// The terminal service answers a wait after at most four minutes; a longer wait is several requests.
const WAIT_CHUNK_MS = 240_000
const ENTER = "\r"
// CSI, OSC and the other two-byte escape sequences a shell prints for colours, titles and the cursor.
const ESCAPES = /\u001b(?:\[[0-?]*[ -/]*[@-~]|\][^\u0007\u001b]*(?:\u0007|\u001b\\)|[@-Z\\-_])/g

export type TerminalFlags = { cols?: string | true; rows?: string | true; title?: string | true; session?: string | true; terminal?: string | true; since?: string | true; waitMs?: string | true; for?: string | true; timeout?: string | true; noEnter: boolean }

function required(value: string | true | undefined, flag: string): string {
  if (typeof value !== "string") throw new CommandFailure("usage", `${flag} is required for this command`)
  return value
}

function wholeNumber(value: string | true | undefined, flag: string, fallback: number): number {
  if (value === undefined) return fallback
  const number = Number(value)
  if (typeof value !== "string" || !Number.isSafeInteger(number) || number < 0) throw new CommandFailure("usage", `${flag} needs a whole number from 0`)
  return number
}

/** The wait a command asks for, in milliseconds: a request to the machine must outlast it. */
export function waitMsOf(verb: TerminalVerb, flags: TerminalFlags): number {
  if (verb === "wait") return wholeNumber(flags.timeout, "--timeout", 60) * 1000
  if (verb === "read") return wholeNumber(flags.waitMs, "--wait-ms", 0)
  return 0
}

function watchOf(spec: string): { kind: string; text?: string } {
  if (spec === "finished" || spec === "waiting_for_input") return { kind: spec }
  const text = spec.startsWith("output_contains=") ? spec.slice("output_contains=".length) : ""
  if (!text) throw new CommandFailure("usage", "--for is finished, waiting_for_input or output_contains=TEXT")
  return { kind: "output_contains", text }
}

function plainText(base64: string): string {
  return Buffer.from(base64, "base64").toString("utf8").replace(ESCAPES, "")
}

export async function runTerminalVerb(verb: TerminalVerb, positionals: readonly string[], flags: TerminalFlags, send: SendRequest): Promise<unknown> {
  const target: RoomTerminalTarget = { sessionId: required(flags.session, "--session") }
  if (verb === "open") {
    const command = positionals.length ? positionals : undefined
    const reply = await send(openRoomTerminalRequest(target,
      { cols: wholeNumber(flags.cols, "--cols", DEFAULT_COLS), rows: wholeNumber(flags.rows, "--rows", DEFAULT_ROWS) },
      { ...(typeof flags.title === "string" ? { title: flags.title } : {}), ...(command ? { command } : {}) }))
    return { terminal: expectRoomTerminalResult(reply, "opened").terminal }
  }
  if (verb === "list") {
    return { terminals: expectRoomTerminalResult(await send(listRoomTerminalsRequest(target)), "listed").terminals }
  }
  const terminalId = required(flags.terminal, "--terminal")
  if (verb === "close") {
    expectRoomTerminalResult(await send(closeRoomTerminalRequest(target, terminalId)), "closed")
    return { closed: terminalId }
  }
  if (verb === "send") {
    // Output read from `after_offset` shows what this input did, not what came before.
    const { snapshot } = expectRoomTerminalResult(await send(roomTerminalSnapshotRequest(target, terminalId)), "snapshot")
    const text = (positionals[0] ?? "") + (flags.noEnter ? "" : ENTER)
    const { input } = expectRoomTerminalResult(await send(roomTerminalInputRequest(target, terminalId, Buffer.from(text, "utf8"))), "input")
    if (!input.accepted) throw new CommandFailure("terminal_input_refused", input.refusal ?? "the terminal did not accept the input")
    return { accepted: true, after_offset: snapshot.output_offset, driver: input.driver }
  }
  const since = wholeNumber(flags.since, "--since", 0)
  if (verb === "read") {
    const { output } = expectRoomTerminalResult(await send(readRoomTerminalOutputRequest(target, terminalId, since, waitMsOf(verb, flags))), "output")
    return { text: plainText(output.data_base64), next_offset: output.next_offset, gap: output.gap, terminal: output.terminal }
  }
  const watches = [watchOf(required(flags.for, "--for"))]
  const deadline = Date.now() + waitMsOf(verb, flags)
  let after = flags.since === undefined ? undefined : since
  for (;;) {
    const timeoutMs = Math.max(0, Math.min(WAIT_CHUNK_MS, deadline - Date.now()))
    const { waited } = expectRoomTerminalResult(await send(roomTerminalRequest(target, {
      operation: "wait", terminal_id: terminalId, watches: watches as never, ...(after === undefined ? {} : { after_offset: after }), timeout_ms: timeoutMs,
    })), "waited")
    if (waited.fired || Date.now() >= deadline) {
      return { fired: waited.fired, screen_text: waited.screen_text, output_offset: waited.output_offset, terminal: waited.terminal }
    }
    after = waited.output_offset
  }
}
