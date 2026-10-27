import {
  closeRoomTerminalRequest, expectRoomTerminalResult, listRoomTerminalsRequest, openRoomTerminalRequest,
  readRoomTerminalOutputRequest, roomTerminalInputRequest, roomTerminalRequest, roomTerminalSnapshotRequest,
  type RoomTerminalTarget,
} from "@chariox/kernel-client/ipc-requests"
import { CommandFailure } from "./agent-command-table.js"
import type { SendRequest } from "./kernel-files.js"

export const TERMINAL_VERBS = ["open", "list", "send", "read", "wait", "close"] as const
export type TerminalVerb = typeof TERMINAL_VERBS[number]

const DEFAULT_COLS = 120
const DEFAULT_ROWS = 30
// The terminal service answers a wait after at most four minutes; a longer wait is several requests.
const WAIT_CHUNK_MS = 240_000
const ENTER = "\r"
// CSI, OSC and the other two-byte escape sequences a shell prints for colours, titles and the cursor.
const ESCAPES = /\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07\x1b]*(?:\x07|\x1b\\)|[@-Z\\-_])/g

type Flag = string | true | undefined
export type TerminalFlags = {
  cols?: Flag; rows?: Flag; title?: Flag; session?: Flag; terminal?: Flag; since?: Flag
  waitMs?: Flag; for?: Flag; timeout?: Flag; hostAgent?: Flag; noEnter: boolean
}

function required(value: Flag, flag: string): string {
  if (typeof value !== "string") throw new CommandFailure("usage", `${flag} is required for this command`)
  return value
}

function wholeNumber(value: Flag, flag: string, fallback: number): number {
  if (value === undefined) return fallback
  const number = Number(value)
  if (typeof value !== "string" || !Number.isSafeInteger(number) || number < 0) throw new CommandFailure("usage", `${flag} needs a whole number from 0`)
  return number
}

/** The wait a command asks for, in milliseconds: a request to the machine must outlast it. */
export function waitMsOf(verb: TerminalVerb, flags: TerminalFlags): number {
  if (verb === "wait") return wholeNumber(flags.timeout, "--timeout", 60) * 1000
  if (verb === "read" || verb === "send") return wholeNumber(flags.waitMs, "--wait-ms", 0)
  return 0
}

function watchOf(spec: string): { kind: string; text?: string } {
  if (spec === "finished" || spec === "waiting_for_input") return { kind: spec }
  const text = spec.startsWith("output_contains=") ? spec.slice("output_contains=".length) : ""
  if (!text) throw new CommandFailure("usage", "--for is finished, waiting_for_input or output_contains=TEXT")
  return { kind: "output_contains", text }
}

// A page can end inside an escape sequence; the cut tail is left for the next read, which then starts at its ESC.
const CUT_ESCAPE = /\x1b(?:\[[0-?]*[ -/]*|\][^\x07\x1b]*\x1b?)?$/

function plainPage(output: { data_base64: string; next_offset: number }): { text: string; next_offset: number } {
  const bytes = Buffer.from(output.data_base64, "base64")
  let end = bytes.length
  let lead = end - 1
  while (lead >= Math.max(0, end - 3) && (bytes[lead]! & 0xc0) === 0x80) lead -= 1
  if (lead >= 0) {
    const byte = bytes[lead]!
    const width = byte >= 0xc2 && byte <= 0xdf ? 2 : byte >= 0xe0 && byte <= 0xef ? 3 : byte >= 0xf0 && byte <= 0xf4 ? 4 : 1
    if (end - lead < width) end = lead
  }
  // PTY pages are byte ranges. Re-read an incomplete character rather than
  // replacing it and advancing past bytes the next page needs.
  const raw = bytes.subarray(0, end).toString("utf8")
  const cut = CUT_ESCAPE.exec(raw)
  return {
    text: (cut ? raw.slice(0, cut.index) : raw).replace(ESCAPES, ""),
    next_offset: output.next_offset - (bytes.length - end) - (cut ? Buffer.byteLength(cut[0]) : 0),
  }
}

export async function runTerminalVerb(verb: TerminalVerb, positionals: readonly string[], flags: TerminalFlags, send: SendRequest): Promise<unknown> {
  const target: RoomTerminalTarget = {
    sessionId: required(flags.session, "--session"),
    ...(typeof flags.hostAgent === "string" ? { hostAgent: flags.hostAgent } : {}),
  }
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
    const text = (positionals[0] ?? "") + (flags.noEnter ? "" : ENTER)
    const keys = roomTerminalInputRequest(target, terminalId, Buffer.from(text, "utf8"))
    const waitMs = waitMsOf(verb, flags)
    // Output read from the offset before the input shows what this input did, not what came before.
    // With --since the offset is known and typing and reading travel together: one round trip.
    const offset = flags.since === undefined
      ? expectRoomTerminalResult(await send(roomTerminalSnapshotRequest(target, terminalId)), "snapshot").snapshot.output_offset
      : wholeNumber(flags.since, "--since", 0)
    if (waitMs === 0) {
      const { input } = expectRoomTerminalResult(await send(keys), "input")
      if (!input.accepted) throw new CommandFailure("terminal_input_refused", input.refusal ?? "the terminal did not accept the input")
      return { accepted: true, after_offset: offset, driver: input.driver }
    }
    const typing = send(keys)
    const reading = send(readRoomTerminalOutputRequest(target, terminalId, offset, waitMs))
    reading.catch(() => undefined)
    const { input } = expectRoomTerminalResult(await typing, "input")
    if (!input.accepted) throw new CommandFailure("terminal_input_refused", input.refusal ?? "the terminal did not accept the input")
    const { output } = expectRoomTerminalResult(await reading, "output")
    return { accepted: true, after_offset: offset, driver: input.driver, ...plainPage(output), gap: output.gap, terminal: output.terminal }
  }
  const since = wholeNumber(flags.since, "--since", 0)
  if (verb === "read") {
    const { output } = expectRoomTerminalResult(await send(readRoomTerminalOutputRequest(target, terminalId, since, waitMsOf(verb, flags))), "output")
    return { ...plainPage(output), gap: output.gap, terminal: output.terminal }
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
