import { closeRoomTerminalRequest, expectRoomTerminalResult, listRoomTerminalsRequest, openRoomTerminalRequest, readRoomTerminalOutputRequest, roomTerminalRequest, type KernelTerminalTarget } from "@chariox/kernel-client/ipc-requests"
import type { ProgramInput } from "@chariox/kernel-client/kernel-types"
import { StringDecoder } from "node:string_decoder"
import type { Readable, Writable } from "node:stream"
import { CommandFailure } from "./agent-command-table.js"
import type { SendRequest } from "./kernel-files.js"

const COLS = 200
const ROWS = 50
// The terminal service answers a wait after at most four minutes; a longer wait is several requests.
const WAIT_CHUNK_MS = 240_000

// One request carries at most this much input; the kernel enforces the same limit.
export const MAX_INLINE_STDIN_BYTES = 64 * 1024

export type KernelExecOptions = { command: readonly string[]; directory?: string | undefined; timeoutSeconds: number; stdin?: ProgramInput | undefined; signals?: NodeJS.Process }

/**
 * The input of the program: a file already on the kernel when `--stdin-file` names one, else what
 * this process's own stdin carries up to its end (nothing for a terminal or an empty input).
 */
export async function readExecInput(input: Readable & { isTTY?: boolean }, stdinFile: string | undefined, timeoutSeconds: number): Promise<ProgramInput | undefined> {
  if (stdinFile !== undefined) return { kind: "file", path: stdinFile }
  if (input.isTTY) return undefined
  const chunks: Buffer[] = []
  let size = 0
  const timer = setTimeout(() => input.destroy(new CommandFailure("exec_stdin_not_ended", `stdin did not end within ${timeoutSeconds} s; end it (for example </dev/null), pipe the input in, or pass --stdin-file`)), timeoutSeconds * 1000)
  try {
    for await (const chunk of input) {
      chunks.push(chunk as Buffer)
      size += (chunk as Buffer).length
      if (size > MAX_INLINE_STDIN_BYTES) throw new CommandFailure("exec_stdin_too_large", `stdin is larger than ${MAX_INLINE_STDIN_BYTES} bytes (64 KiB), the most one exec carries; upload the file with chariox files put --kernel NAME and run it with --stdin-file KERNEL_PATH`)
    }
  } catch (error) {
    if (error instanceof CommandFailure) throw error
    throw new CommandFailure("exec_stdin_unreadable", `stdin could not be read: ${String((error as Error)?.message ?? error)}`)
  } finally {
    clearTimeout(timer)
  }
  return size === 0 ? undefined : { kind: "inline", data_base64: Buffer.concat(chunks).toString("base64") }
}

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
  const { terminal } = expectRoomTerminalResult(await send(openRoomTerminalRequest(opening, { cols: COLS, rows: ROWS }, { command: options.command, ...(options.stdin ? { stdin: options.stdin } : {}) })), "opened")
  const terminalId = terminal.terminal_id
  let failed = true
  // An interrupt ends the command with the terminal: Ctrl-C must not leave a shell running on the kernel.
  const interrupt = (signal: number) => () => { void send(closeRoomTerminalRequest(target, terminalId)).finally(() => process.exit(128 + signal)) }
  const onInt = interrupt(2), onTerm = interrupt(15), onHup = interrupt(1)
  options.signals?.on("SIGINT", onInt); options.signals?.on("SIGTERM", onTerm); options.signals?.on("SIGHUP", onHup)
  const deadline = Date.now() + options.timeoutSeconds * 1000
  try {
    let ended: unknown
    for (;;) {
      const timeoutMs = Math.max(0, Math.min(WAIT_CHUNK_MS, deadline - Date.now()))
      const { waited } = expectRoomTerminalResult(await send(roomTerminalRequest(target, {
        operation: "wait", terminal_id: terminalId, watches: [{ kind: "finished" }] as never, timeout_ms: timeoutMs,
      })), "waited")
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
      const { output } = expectRoomTerminalResult(await send(readRoomTerminalOutputRequest(target, terminalId, offset)), "output")
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
    options.signals?.off("SIGINT", onInt); options.signals?.off("SIGTERM", onTerm); options.signals?.off("SIGHUP", onHup)
    // The command never outlives this call. The kernel removes a one-command terminal itself once
    // its output was read to the end, so a close that finds nothing is the expected answer: it
    // counts as closed when the kernel's list no longer holds the terminal. A terminal that is
    // still listed and cannot be closed is named, with the way to close it.
    await send(closeRoomTerminalRequest(target, terminalId)).catch(async (error: unknown) => {
      if (await isGone(send, target, terminalId)) return
      if (failed) process.stderr.write(`${JSON.stringify({ warning: { code: "exec_terminal_left_open", terminal_id: terminalId, message: String((error as Error)?.message ?? error), next_step: `chariox terminal close --terminal ${terminalId}` } })}\n`)
      else throw new CommandFailure("exec_terminal_left_open", `the command finished but its terminal could not be closed: ${String((error as Error)?.message ?? error)}`, { terminal_id: terminalId, next_step: `chariox terminal close --terminal ${terminalId}` })
    })
  }
}

async function isGone(send: SendRequest, target: KernelTerminalTarget, terminalId: string): Promise<boolean> {
  try {
    const { terminals } = expectRoomTerminalResult(await send(listRoomTerminalsRequest(target)), "listed")
    return !terminals.some(terminal => terminal.terminal_id === terminalId)
  } catch {
    return false
  }
}
