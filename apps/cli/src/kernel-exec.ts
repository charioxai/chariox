import { closeRoomTerminalRequest, expectRoomTerminalResult, openRoomTerminalRequest, readRoomTerminalOutputRequest, roomTerminalRequest, type KernelTerminalTarget } from "@chariox/kernel-client/ipc-requests"
import type { Writable } from "node:stream"
import { CommandFailure } from "./agent-command-table.js"
import type { SendRequest } from "./kernel-files.js"

const COLS = 200
const ROWS = 50
// The terminal service answers a wait after at most four minutes; a longer wait is several requests.
const WAIT_CHUNK_MS = 240_000

export type KernelExecOptions = { command: readonly string[]; directory?: string | undefined; timeoutSeconds: number }

/**
 * One command on a kernel with no Room: open a terminal of the kernel's own running the argv,
 * wait for it to finish, collect its output, close it. It is the kernel terminal's own
 * operations, not a second exec path, so the output is the terminal's (stdout and stderr share
 * the one terminal stream) and the exit code is the program's.
 */
export async function runKernelExec(options: KernelExecOptions, send: SendRequest, stdout: Writable): Promise<number> {
  const target: KernelTerminalTarget = { kernelTerminal: true, ...(options.directory ? { directory: options.directory } : {}) }
  const { terminal } = expectRoomTerminalResult(await send(openRoomTerminalRequest(target, { cols: COLS, rows: ROWS }, { command: options.command })), "opened")
  const terminalId = terminal.terminal_id
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
    for (;;) {
      const { output } = expectRoomTerminalResult(await send(readRoomTerminalOutputRequest(target, terminalId, offset)), "output")
      if (output.gap) throw new CommandFailure("kernel_output_lost", "the command wrote more than the terminal keeps; run it in a terminal and read as it goes")
      const bytes = Buffer.from(output.data_base64, "base64")
      if (bytes.length) stdout.write(bytes.toString("utf8").replace(/\r\n/g, "\n"))
      offset = output.next_offset
      if (offset >= output.terminal.output_offset) break
    }
    const status = ended as { state: string; outcome?: { reason: string; exit_code?: number | null; signal?: string | null } }
    if (status.outcome?.reason === "program_exited") {
      if (typeof status.outcome.exit_code === "number") return status.outcome.exit_code
      if (status.outcome.signal) return 128
    }
    throw new CommandFailure("exec_failed", `the command did not exit normally (${(status.outcome as { reason?: string } | undefined)?.reason ?? status.state})`)
  } finally {
    await send(closeRoomTerminalRequest(target, terminalId)).catch(() => undefined)
  }
}
