import { closeRoomTerminalRequest, expectRoomTerminalResult, openRoomTerminalRequest, readRoomTerminalOutputRequest, roomTerminalRequest, type KernelTerminalTarget } from "@chariox/kernel-client/ipc-requests"
import { StringDecoder } from "node:string_decoder"
import type { Writable } from "node:stream"
import { CommandFailure } from "./agent-command-table.js"
import type { SendRequest } from "./kernel-files.js"

const COLS = 200
const ROWS = 50
// The terminal service answers a wait after at most four minutes; a longer wait is several requests.
const WAIT_CHUNK_MS = 240_000

export type KernelExecOptions = { command: readonly string[]; directory?: string | undefined; timeoutSeconds: number; signals?: NodeJS.Process }

/**
 * One command on a kernel with no Room: open a terminal of the kernel's own running the argv,
 * wait for it to finish, collect its output, close it. It is the kernel terminal's own
 * operations, not a second exec path, so the output is the terminal's (stdout and stderr share
 * the one terminal stream) and the exit code is the program's.
 */
export async function runKernelExec(options: KernelExecOptions, send: SendRequest, stdout: Writable): Promise<number> {
  // The folder is chosen when the terminal opens; the kernel refuses it on every later operation.
  const opening: KernelTerminalTarget = { kernelTerminal: true, ...(options.directory ? { directory: options.directory } : {}) }
  const target: KernelTerminalTarget = { kernelTerminal: true }
  let terminalId: string | undefined
  let failed = true
  let signalExit: number | undefined
  let wakeInterrupted!: () => void
  const interrupted = new Promise<void>(resolve => { wakeInterrupted = resolve })
  // The kernel may have started the command by the time a signal arrives.
  // Keep waiting for the terminal ID so the finally block can close it.
  const interrupt = (signal: number) => () => {
    signalExit ??= 128 + signal
    wakeInterrupted()
  }
  const onInt = interrupt(2), onTerm = interrupt(15), onHup = interrupt(1)
  options.signals?.on("SIGINT", onInt); options.signals?.on("SIGTERM", onTerm); options.signals?.on("SIGHUP", onHup)
  const sendUntilInterrupted: SendRequest = body => Promise.race([send(body), interrupted])
  try {
    const { terminal } = expectRoomTerminalResult(await send(openRoomTerminalRequest(opening, { cols: COLS, rows: ROWS }, { command: options.command })), "opened")
    terminalId = terminal.terminal_id
    if (signalExit !== undefined) return signalExit
    const deadline = Date.now() + options.timeoutSeconds * 1000
    let ended: unknown
    for (;;) {
      const timeoutMs = Math.max(0, Math.min(WAIT_CHUNK_MS, deadline - Date.now()))
      const reply = await sendUntilInterrupted(roomTerminalRequest(target, {
        operation: "wait", terminal_id: terminalId, watches: [{ kind: "finished" }] as never, timeout_ms: timeoutMs,
      }))
      if (signalExit !== undefined) return signalExit
      const { waited } = expectRoomTerminalResult(reply, "waited")
      if (waited.fired) { ended = waited.terminal.status; break }
      if (Date.now() >= deadline) {
        throw new CommandFailure("kernel_request_timeout", `the command is still running after ${options.timeoutSeconds} s and was stopped; raise --timeout, or run it in a terminal (chariox terminal open ... --kernel)`)
      }
    }
    let offset = 0
    // A page is a byte range that may end inside a character: the decoder carries the cut bytes to the next page,
    // and a carriage return waits for the byte after it before CRLF becomes LF.
    const decoder = new StringDecoder("utf8")
    let carriedReturn = false
    const emit = (text: string) => {
      const joined = (carriedReturn ? "\r" : "") + text
      carriedReturn = joined.endsWith("\r")
      const body = carriedReturn ? joined.slice(0, -1) : joined
      if (body) stdout.write(body.replace(/\r\n/g, "\n"))
    }
    for (;;) {
      const reply = await sendUntilInterrupted(readRoomTerminalOutputRequest(target, terminalId, offset))
      if (signalExit !== undefined) return signalExit
      const { output } = expectRoomTerminalResult(reply, "output")
      if (output.gap) throw new CommandFailure("kernel_output_lost", "the command wrote more than the terminal keeps; run it in a terminal and read as it goes")
      const bytes = Buffer.from(output.data_base64, "base64")
      if (bytes.length) emit(decoder.write(bytes))
      offset = output.next_offset
      if (offset >= output.terminal.output_offset) break
    }
    emit(decoder.end() + (carriedReturn ? "\r" : ""))
    carriedReturn = false
    const status = ended as { state: string; outcome?: { reason: string; exit_code?: number | null; signal?: string | null } }
    if (status.outcome?.reason === "program_exited") {
      if (typeof status.outcome.exit_code === "number") { failed = false; return status.outcome.exit_code }
      const signal = Number(status.outcome.signal)
      if (Number.isInteger(signal) && signal > 0 && signal <= 127) { failed = false; return 128 + signal }
      throw new CommandFailure("exec_failed", "the kernel did not report the terminating signal number; update the kernel and retry")
    }
    throw new CommandFailure("exec_failed", `the command did not exit normally (${(status.outcome as { reason?: string } | undefined)?.reason ?? status.state})`)
  } finally {
    try {
      if (terminalId !== undefined) {
        // Name the terminal and how to close it if cleanup fails.
        await send(closeRoomTerminalRequest(target, terminalId)).catch((error: unknown) => {
          if (failed) process.stderr.write(`${JSON.stringify({ warning: { code: "exec_terminal_left_open", terminal_id: terminalId, message: String((error as Error)?.message ?? error), next_step: `chariox terminal close --terminal ${terminalId}` } })}\n`)
          else throw new CommandFailure("exec_terminal_left_open", `the command finished but its terminal could not be closed: ${String((error as Error)?.message ?? error)}`, { terminal_id: terminalId, next_step: `chariox terminal close --terminal ${terminalId}` })
        })
      }
    } finally {
      options.signals?.off("SIGINT", onInt); options.signals?.off("SIGTERM", onTerm); options.signals?.off("SIGHUP", onHup)
    }
  }
}
