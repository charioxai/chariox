import {
  expectRoomTerminalResult,
  readRoomTerminalOutputRequest,
  resizeRoomTerminalRequest,
  roomTerminalInputRequest,
  roomTerminalSnapshotRequest,
  RoomTerminalRefusedError,
  type RoomTerminalTarget,
} from "@chariox/kernel-client/ipc-requests"
import type { RoomTerminalSummary } from "@chariox/kernel-client/kernel-types"

/** Ctrl-] ends an attach, as in telnet; the terminal keeps running. */
export const TERMINAL_DETACH_BYTE = 0x1d
// The same key when the outer terminal reports keys with the kitty protocol.
const TERMINAL_DETACH_KITTY = "\x1b[93;5u"
const OUTPUT_POLL_MS = 4000
const RETRY_DELAYS_MS = [200, 500, 1000, 2000]
const INTERRUPTED = Symbol("interrupted")

export type TerminalSend = (request: unknown) => Promise<unknown>

export type TerminalAttachInput = {
  isTTY?: boolean
  setRawMode?(raw: boolean): unknown
  resume(): unknown
  pause(): unknown
  on(event: "data", listener: (chunk: Buffer) => void): unknown
  off(event: "data", listener: (chunk: Buffer) => void): unknown
}

export type TerminalAttachOutput = {
  columns?: number
  rows?: number
  write(data: string | Uint8Array): unknown
  on(event: "resize", listener: () => void): unknown
  off(event: "resize", listener: () => void): unknown
}

export type TerminalAttachEnd =
  | { readonly reason: "detached" }
  | { readonly reason: "ended"; readonly description: string }

// What a full-screen program leaves switched on in the outer terminal.
const MODE_RESET = "\x1b[0m\x1b[?25h\x1b[?1l\x1b>\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?2004l\x1b[?2026l\x1b[<u\x1b[?7h"

const HOSTED_SCREEN_SWITCH = /\x1b\[\?(?:1049|1047|47)([hl])/g
const HOSTED_SCREEN_SWITCH_PREFIXES = ["\x1b[?1049", "\x1b[?1047", "\x1b[?47"].flatMap(
  whole => Array.from({ length: whole.length }, (_, length) => whole.slice(0, length + 1)),
)
const CLEAR_SCREEN = "\x1b[0m\x1b[H\x1b[2J"

/**
 * The attach owns the outer terminal's alternate screen, so the hosted program must not switch it:
 * entering its own alternate screen becomes a clear, and leaving it asks for a redraw from the kernel,
 * which holds the normal screen it left behind.
 */
class HostedScreenFilter {
  private carry = ""

  /** Output that stayed quiet after a held-back start of a sequence: it was plain text after all. */
  flush(): Buffer {
    const held = Buffer.from(this.carry, "latin1")
    this.carry = ""
    return held
  }

  push(chunk: Uint8Array): { out: Buffer; left: boolean } {
    let text = this.carry + Buffer.from(chunk).toString("latin1")
    this.carry = ""
    // Keep back a possible start of a switch sequence that the next chunk completes.
    for (let length = Math.min(text.length, 8); length > 0; length--) {
      if (HOSTED_SCREEN_SWITCH_PREFIXES.includes(text.slice(-length))) {
        this.carry = text.slice(-length)
        text = text.slice(0, -length)
        break
      }
    }
    let left = false
    const out = text.replace(HOSTED_SCREEN_SWITCH, (_match, direction: string) => {
      if (direction === "l") left = true
      return direction === "h" ? CLEAR_SCREEN : ""
    })
    return { out: Buffer.from(out, "latin1"), left }
  }
}

/**
 * Hand the outer terminal to a Room terminal until Ctrl-] or until the terminal ends.
 * The terminal keeps running either way. A lost connection keeps the last screen and
 * resumes from the same output offset; only a refusal by the kernel ends the attach.
 */
export async function attachRoomTerminal(options: {
  send: TerminalSend
  target: RoomTerminalTarget
  terminal: RoomTerminalSummary
  input: TerminalAttachInput
  output: TerminalAttachOutput
  signal?: AbortSignal
}): Promise<TerminalAttachEnd> {
  const { send, target, input, output } = options
  const terminalId = options.terminal.terminal_id
  const hostedTarget: RoomTerminalTarget = options.terminal.host_agent && !target.hostAgent
    ? { ...target, hostAgent: options.terminal.host_agent }
    : target
  const hosted = new HostedScreenFilter()
  let redrawNeeded = false
  const size = () => ({ cols: output.columns || 80, rows: output.rows || 24 })
  const state: { end: TerminalAttachEnd | null } = { end: null }
  // Set once the outer terminal is restored: a reply that arrives later writes nothing to it.
  let over = false
  let wake: () => void = () => {}
  let interrupt: () => void = () => {}
  // Resolves when the attach ends, so a request that never answers cannot hold it.
  const interrupted = new Promise<typeof INTERRUPTED>(resolve => { interrupt = () => resolve(INTERRUPTED) })
  const finish = (result: TerminalAttachEnd) => {
    state.end ??= result
    wake()
    interrupt()
  }
  const draw = (bytes: Uint8Array) => {
    const { out, left } = hosted.push(bytes)
    redrawNeeded ||= left
    if (out.length > 0) output.write(out)
  }
  let titled = false
  const title = (text: string) => {
    titled = true
    output.write(`\x1b]2;${text}\x07`)
  }

  async function resync(): Promise<number> {
    const { snapshot } = expectRoomTerminalResult(await send(roomTerminalSnapshotRequest(hostedTarget, terminalId)), "snapshot")
    if (state.end || over) return snapshot.output_offset
    redrawNeeded = false
    draw(Buffer.concat([Buffer.from(CLEAR_SCREEN), Buffer.from(snapshot.screen_ansi_base64, "base64")]))
    return snapshot.output_offset
  }

  // Retry a lost connection forever (the last screen stays); a refusal ends the attach.
  async function untilDelivered<T>(call: () => Promise<T>): Promise<T | undefined> {
    for (let attempt = 0; !state.end; attempt++) {
      try {
        const result = await Promise.race([call(), interrupted])
        if (result === INTERRUPTED) return undefined
        if (attempt > 0) title("chariox terminal")
        return result
      } catch (error) {
        if (error instanceof RoomTerminalRefusedError) {
          finish({ reason: "ended", description: error.message })
          return undefined
        }
        title("chariox terminal — reconnecting…")
        await new Promise<void>(resolve => {
          wake = resolve
          setTimeout(resolve, RETRY_DELAYS_MS[Math.min(attempt, RETRY_DELAYS_MS.length - 1)])
        })
      }
    }
    return undefined
  }

  // Input keeps its order; a chunk lost with the connection is dropped, never replayed
  // into a program that may already have received it. After the attach is over, chunks that
  // have not been sent yet are dropped and a late reply writes nothing to the outer terminal.
  let sending: Promise<unknown> = Promise.resolve()
  const sendKeys = (bytes: Buffer) => {
    sending = sending.then(async () => {
      if (over) return
      try {
        const reply = expectRoomTerminalResult(await send(roomTerminalInputRequest(hostedTarget, terminalId, bytes)), "input")
        if (!reply.input.accepted && !over) title(`chariox terminal — ${reply.input.refusal ?? "input refused"}`)
      } catch (error) {
        if (error instanceof RoomTerminalRefusedError) finish({ reason: "ended", description: error.message })
        else if (!over) title("chariox terminal — input lost while reconnecting")
      }
    })
  }

  // Ctrl-] works from the first moment; other keys wait until the screen is in sync.
  let synced = false
  const onData = (chunk: Buffer) => {
    if (state.end) return
    const text = chunk.toString("latin1")
    const detachAt = Math.max(chunk.indexOf(TERMINAL_DETACH_BYTE), text.indexOf(TERMINAL_DETACH_KITTY))
    if (detachAt < 0) return synced ? sendKeys(chunk) : undefined
    if (detachAt > 0 && synced) sendKeys(chunk.subarray(0, detachAt))
    finish({ reason: "detached" })
  }
  const onResize = () => {
    void untilDelivered(() => send(resizeRoomTerminalRequest(hostedTarget, terminalId, size())))
  }
  const onAbort = () => finish({ reason: "detached" })

  const raw = Boolean(input.isTTY)
  try {
    if (raw) input.setRawMode?.(true)
    input.resume()
    options.signal?.addEventListener("abort", onAbort)
    input.on("data", onData)
    output.write("\x1b[?1049h")
    if (options.signal?.aborted) return { reason: "detached" }
    await untilDelivered(() => send(resizeRoomTerminalRequest(hostedTarget, terminalId, size())))
    let offset = await untilDelivered(resync)
    if (offset === undefined) return state.end ?? { reason: "detached" }
    synced = true
    output.on("resize", onResize)
    while (!state.end) {
      const reply = await untilDelivered(async () => expectRoomTerminalResult(
        await send(readRoomTerminalOutputRequest(hostedTarget, terminalId, offset!, OUTPUT_POLL_MS)), "output"))
      if (!reply || state.end) break
      const { output: batch } = reply
      if (batch.gap) {
        offset = await untilDelivered(resync)
        continue
      }
      if (batch.data_base64) draw(Buffer.from(batch.data_base64, "base64"))
      else {
        const held = hosted.flush()
        if (held.length > 0) output.write(held)
      }
      offset = batch.next_offset
      if (redrawNeeded) offset = await untilDelivered(resync) ?? offset
      if (batch.terminal.status.state === "ended" && !batch.data_base64) {
        finish({ reason: "ended", description: batch.terminal.status.description })
      }
    }
  } finally {
    input.off("data", onData)
    output.off("resize", onResize)
    options.signal?.removeEventListener("abort", onAbort)
    // Not awaiting `sending`: an unanswered input request must not keep the user in the attach.
    over = true
    output.write(MODE_RESET)
    output.write("\x1b[?1049l")
    if (titled) output.write("\x1b]2;\x07")
    if (raw) input.setRawMode?.(false)
    input.pause()
  }
  return state.end ?? { reason: "detached" }
}
