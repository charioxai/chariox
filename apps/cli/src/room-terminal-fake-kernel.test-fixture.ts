import { EventEmitter } from "node:events"
import { LocalIpcError } from "@chariox/kernel-client/ipc"
import type { RoomTerminalSummary } from "@chariox/kernel-client/kernel-types"

type Operation = { operation: string; [key: string]: unknown }

/** A kernel that answers `RoomTerminal` requests for one scripted terminal. */
export class FakeTerminalKernel {
  readonly requests: Operation[] = []
  readonly typed: Buffer[] = []
  readonly stalled: Buffer[] = []
  output = Buffer.alloc(0)
  retainedFrom = 0
  ended: string | null = null
  failNext = 0
  refuse: string | null = null
  /** Input requests are received but never answered. */
  stallInput = false
  /** Snapshot requests wait until `releaseSnapshots` runs. */
  holdSnapshots = false
  private heldSnapshots: (() => void)[] = []
  screen = "PROMPT$ "
  size = { cols: 80, rows: 24 }
  private waiters: (() => void)[] = []

  summary(): RoomTerminalSummary {
    return {
      terminal_id: "term-1", session_id: "room", title: "shell", program: "/bin/bash", working_directory: "/w",
      cols: this.size.cols, rows: this.size.rows,
      status: this.ended
        ? { state: "ended", outcome: { reason: "program_exited", exit_code: 0, signal: null }, description: this.ended }
        : { state: "running" },
      driver: { state: "free" }, opened_by: { kind: "user" }, user_interacted: true, created_at_ms: 1,
      output_offset: this.retainedFrom + this.output.length,
    }
  }

  emit(text: string | Buffer) {
    this.output = Buffer.concat([this.output, Buffer.from(text)])
    this.waiters.splice(0).forEach(wake => wake())
  }

  releaseSnapshots() {
    this.holdSnapshots = false
    this.heldSnapshots.splice(0).forEach(release => release())
  }

  end(description: string) {
    this.ended = description
    this.waiters.splice(0).forEach(wake => wake())
  }

  readonly send = async (request: unknown): Promise<unknown> => {
    const { operation, session_id } = (request as { RoomTerminal: { operation: Operation; session_id: string } }).RoomTerminal
    void session_id
    this.requests.push(operation)
    if (this.failNext > 0) {
      this.failNext--
      throw new LocalIpcError("write kernel request", "connection lost", "connection_closed", true)
    }
    if (this.refuse) return { Error: { message: this.refuse } }
    const wrap = (result: unknown) => ({ RoomTerminal: { result } })
    switch (operation.operation) {
      case "list": return wrap({ result: "listed", terminals: [this.summary()] })
      case "open": return wrap({ result: "opened", terminal: this.summary() })
      case "close": return wrap({ result: "closed" })
      case "resize":
        this.size = { cols: operation.cols as number, rows: operation.rows as number }
        return wrap({ result: "resized", terminal: this.summary() })
      case "snapshot":
        if (this.holdSnapshots) await new Promise<void>(resolve => this.heldSnapshots.push(resolve))
        return wrap({ result: "snapshot", snapshot: {
          terminal: this.summary(), cursor_row: 0, cursor_col: 0, screen_text: this.screen, scrollback_text: "",
          screen_ansi_base64: Buffer.from(this.screen).toString("base64"), output_offset: this.retainedFrom + this.output.length,
        } })
      case "input":
        if (this.stallInput) {
          this.stalled.push(Buffer.from(operation.data_base64 as string, "base64"))
          return new Promise<never>(() => {})
        }
        this.typed.push(Buffer.from(operation.data_base64 as string, "base64"))
        return wrap({ result: "input", input: { accepted: true, driver: { state: "free" } } })
      case "read_output": {
        const after = operation.after_offset as number
        const gap = () => wrap({ result: "output", output: { terminal: this.summary(), data_base64: "", next_offset: this.retainedFrom + this.output.length, gap: true } })
        if (after < this.retainedFrom) return gap()
        const available = () => this.retainedFrom + this.output.length > after || this.ended !== null
        if (!available()) {
          await new Promise<void>(resolve => {
            const timer = setTimeout(resolve, Math.min(Number(operation.wait_ms ?? 0), 50))
            this.waiters.push(() => { clearTimeout(timer); resolve() })
          })
        }
        if (after < this.retainedFrom) return gap()
        const data = this.output.subarray(after - this.retainedFrom)
        return wrap({ result: "output", output: {
          terminal: this.summary(), data_base64: data.toString("base64"),
          next_offset: this.retainedFrom + this.output.length, gap: false,
        } })
      }
      default: throw new Error(`unexpected operation ${operation.operation}`)
    }
  }
}

export class FakeTerminalIo {
  readonly written: string[] = []
  rawModes: boolean[] = []
  paused = false
  columns = 100
  rows = 40
  isTTY = true
  private readonly events = new EventEmitter()

  setRawMode(raw: boolean) { this.rawModes.push(raw) }
  resume() { this.paused = false }
  pause() { this.paused = true }
  on(event: string, listener: (...args: never[]) => void) { this.events.on(event, listener as never) }
  off(event: string, listener: (...args: never[]) => void) { this.events.off(event, listener as never) }
  write(data: string | Uint8Array) { this.written.push(typeof data === "string" ? data : Buffer.from(data).toString("latin1")) }
  type(data: string | Buffer) { this.events.emit("data", Buffer.from(data)) }
  resizeTo(columns: number, rows: number) { this.columns = columns; this.rows = rows; this.events.emit("resize") }
  get text() { return this.written.join("") }
}

export const until = async (condition: () => boolean, what: string) => {
  for (let i = 0; i < 200; i++) {
    if (condition()) return
    await new Promise(resolve => setTimeout(resolve, 10))
  }
  throw new Error(`timed out waiting for ${what}`)
}
