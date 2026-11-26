import { closeRoomTerminalRequest, expectRoomTerminalResult, listRoomTerminalsRequest, openRoomTerminalRequest, readRoomTerminalOutputRequest, roomTerminalRequest, type KernelTerminalTarget } from "@chariox/kernel-client/ipc-requests"
import type { ProgramInput } from "@chariox/kernel-client/kernel-types"
import { fstatSync } from "node:fs"
import { StringDecoder } from "node:string_decoder"
import { Readable, type Writable } from "node:stream"
import { CommandFailure } from "./agent-command-table.js"
import type { SendRequest } from "./kernel-files.js"

const COLS = 200
const ROWS = 50
// The terminal service answers a wait after at most four minutes; a longer wait is several requests.
const WAIT_CHUNK_MS = 240_000

// One request carries at most this much input; the kernel enforces the same limit.
export const MAX_INLINE_STDIN_BYTES = 64 * 1024

export type KernelExecOptions = {
  command: readonly string[]; directory?: string | undefined; timeoutSeconds: number; stdin?: ProgramInput | undefined; signals?: NodeJS.Process
  /** The kernel the command runs on, named in what is said about a dropped link. */
  kernel?: string | undefined
  /** The wait between two tries while the link is down. */
  retryStepMs?: number
  /** Where the one plain line about a dropped link goes (stderr by default). */
  notice?: (line: string) => void
}

// A dropped link between this computer and the kernel does not end the command: the kernel keeps its terminal until it is
// closed. While the link is down the same request is tried again every this long, until the exec's own --timeout runs out.
export const EXEC_LINK_RETRY_MS = 2_000
// The gateway's own answers for a request that did not get through; the command is not known to have stopped.
const LINK_DROP_CODES = new Set(["kernel_connection_lost", "kernel_request_timeout", "kernel_offline", "kernel_unreachable"])
const linkDropped = (error: unknown): boolean => LINK_DROP_CODES.has((error as { code?: unknown } | null)?.code as string)

// How long an idle stdin (a pipe nobody writes to, as in an agent's tool shell) is given to show data or its end.
// A producer that has sent nothing by then is not waited for: pass --stdin to wait for it.
const STDIN_PROBE_MS = 250

export type ExecInputOptions = {
  /** `--stdin-file`: a file already on the kernel. */
  file?: string | undefined
  /** `--stdin`: read this process's stdin to its end, however long it takes (up to the exec timeout). */
  wait?: boolean
  /** Stdin is a regular file (`< script.sh`): it is finite, so it is read whole. */
  regularFile?: boolean
  timeoutSeconds: number
  probeMs?: number
  /** Watches an idle stdin while the command runs, so data that arrives too late to be sent is reported, not lost silently. */
  late?: LateStdin
}

/** Whether data arrived on an idle stdin after the command had started; `stop` ends the watch. */
export type LateStdin = { arrived: boolean; stop: () => void }
export const lateStdin = (): LateStdin => ({ arrived: false, stop: () => {} })

/** What to tell the caller when input showed up after the command had started: one line, after the command's own output. */
export const LATE_STDIN_NOTICE = "stdin data arrived after the command started and was not sent; pass --stdin to wait for it"

/**
 * The input of the program: a file already on the kernel when `--stdin-file` names one, else what this
 * process's own stdin carries. Stdin is only read when it is a regular file, when `--stdin` asks for it,
 * or when a pipe shows data or its end at once; an idle open stdin is never waited on, so a shell whose
 * stdin stays open (an agent's tool shell, cron) runs the command at once. A terminal is not read.
 */
export async function readExecInput(input: Readable & { isTTY?: boolean }, options: ExecInputOptions): Promise<ProgramInput | undefined> {
  if (options.file !== undefined) return { kind: "file", path: options.file }
  if (input.isTTY && !options.wait) return undefined
  if (!options.wait && !options.regularFile && !(await showsDataOrEnd(input, options.probeMs ?? STDIN_PROBE_MS))) {
    // An idle stdin is not waited on, but it is watched without keeping this process alive: a producer that
    // writes while the command runs is told it was not heard.
    if (options.late) watchLate(input, options.late)
    else input.destroy()
    return undefined
  }
  const chunks: Buffer[] = []
  let size = 0
  const timer = setTimeout(() => input.destroy(new CommandFailure("exec_stdin_not_ended", `stdin did not end within ${options.timeoutSeconds} s; end it (for example </dev/null) or pipe the input in`)), options.timeoutSeconds * 1000)
  try {
    for await (const chunk of input) {
      chunks.push(chunk as Buffer)
      size += (chunk as Buffer).length
      if (size > MAX_INLINE_STDIN_BYTES) throw new CommandFailure("exec_stdin_too_large", `stdin is larger than ${MAX_INLINE_STDIN_BYTES} bytes (64 KiB), the most one exec carries; upload the file with chariox files put --kernel NAME and run it with --stdin-file KERNEL_PATH`)
    }
  } catch (error) {
    if (error instanceof CommandFailure) throw error
    throw unreadable(error)
  } finally {
    clearTimeout(timer)
  }
  return size === 0 ? undefined : { kind: "inline", data_base64: Buffer.concat(chunks).toString("base64") }
}

/**
 * This process's stdin. Some runtimes refuse to open it at all (a directory redirected into stdin throws on first
 * access): that becomes a stream whose first read fails, so it is refused as exec_stdin_unreadable like any other
 * unreadable input, and `--stdin-file` (which never reads it) is unaffected.
 */
export function ownStdin(): Readable & { isTTY?: boolean } {
  try { return process.stdin } catch (error) { return new Readable({ read() { this.destroy(error as Error) } }) }
}

/** This process's stdin is a regular file (`< script.sh`), which is finite: no probe is needed. */
export function stdinIsRegularFile(): boolean {
  try { return fstatSync(0).isFile() } catch { return false }
}

const unreadable = (error: unknown) => new CommandFailure("exec_stdin_unreadable", `stdin could not be read: ${String((error as Error)?.message ?? error)}`)

/**
 * Watch an idle stdin for data while the command runs; the handle is unreferenced so the watch never holds the
 * process open. An error on the input (it was closed, or cannot be read) ends the watch: it is not the command's
 * failure and must not stop the CLI before the terminal is closed. Both listeners go when the watch ends.
 */
function watchLate(input: Readable, late: LateStdin): void {
  const onData = () => { late.arrived = true; late.stop() }
  const onError = () => late.stop()
  late.stop = () => { input.off("data", onData); input.off("error", onError); input.destroy() }
  try {
    input.on("error", onError)
    input.on("data", onData)
    ;(input as Readable & { unref?: () => void }).unref?.()
  } catch { late.stop() }
}

/**
 * Whether the stream has data or has ended within `ms`; an idle one is left to the caller, which watches or closes it.
 * An error before either rejects with exec_stdin_unreadable (for example a directory redirected into stdin).
 */
function showsDataOrEnd(input: Readable, ms: number): Promise<boolean> {
  return new Promise((resolve, reject) => {
    const done = () => {
      clearTimeout(timer)
      input.off("readable", onReadable)
      input.off("error", onError)
    }
    const onReadable = () => { done(); resolve(true) }
    const onError = (error: unknown) => { done(); input.destroy(); reject(unreadable(error)) }
    const timer = setTimeout(() => { done(); resolve(false) }, ms)
    // Some runtimes refuse to open an unreadable stdin the moment it is listened to (bun, for a directory).
    try {
      input.on("error", onError)
      input.on("readable", onReadable)
    } catch (error) { onError(error) }
  })
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
  const retryStepMs = options.retryStepMs ?? EXEC_LINK_RETRY_MS
  const tell = options.notice ?? ((line: string) => { process.stderr.write(`${line}\n`) })
  let deadline = Number.POSITIVE_INFINITY
  // The link stayed down past --timeout: the command is not this call's to stop, so the terminal is not closed.
  let leaveRunning = false
  // Every request after the terminal is open: a link that drops is waited for, never answered by closing the terminal
  // (that would end the command whose result is wanted). wait and read_output are by terminal and offset, so a retry is safe.
  const sendThroughDrops: SendRequest = async body => {
    let told = false
    for (;;) {
      try {
        return await sendUntilInterrupted(body)
      } catch (error) {
        if (!linkDropped(error)) throw error
        if (signalExit !== undefined) return undefined
        if (Date.now() >= deadline) { leaveRunning = true; throw linkLost(options, terminalId!, options.timeoutSeconds) }
        if (!told) { told = true; tell(`link to kernel ${options.kernel ?? "the kernel"} lost, waiting for it; the command keeps running as terminal ${terminalId}`) }
        let timer: ReturnType<typeof setTimeout> | undefined
        await Promise.race([new Promise<void>(resolve => { timer = setTimeout(resolve, retryStepMs) }), interrupted])
        clearTimeout(timer)
        if (signalExit !== undefined) return undefined
      }
    }
  }
  try {
    const { terminal } = expectRoomTerminalResult(await send(openRoomTerminalRequest(opening, { cols: COLS, rows: ROWS }, { command: options.command, ...(options.stdin ? { stdin: options.stdin } : {}) })), "opened")
    terminalId = terminal.terminal_id
    if (signalExit !== undefined) return signalExit
    deadline = Date.now() + options.timeoutSeconds * 1000
    let ended: unknown
    for (;;) {
      const timeoutMs = Math.max(0, Math.min(WAIT_CHUNK_MS, deadline - Date.now()))
      const reply = await sendThroughDrops(roomTerminalRequest(target, {
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
      const reply = await sendThroughDrops(readRoomTerminalOutputRequest(target, terminalId, offset))
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
      if (terminalId !== undefined && !leaveRunning) {
        const openTerminal = terminalId
        // The kernel removes a one-command terminal itself once its output was read to the end, so a close that finds
        // nothing is the expected answer: it counts as closed when the kernel's list no longer holds the terminal. A
        // terminal that is still listed and cannot be closed is named, with the way to close it.
        await send(closeRoomTerminalRequest(target, terminalId)).catch(async (error: unknown) => {
          if (await isGone(send, target, openTerminal)) return
          if (failed) process.stderr.write(`${JSON.stringify({ warning: { code: "exec_terminal_left_open", terminal_id: terminalId, message: String((error as Error)?.message ?? error), next_step: `chariox terminal close --terminal ${terminalId}` } })}\n`)
          else throw new CommandFailure("exec_terminal_left_open", `the command finished but its terminal could not be closed: ${String((error as Error)?.message ?? error)}`, { terminal_id: terminalId, next_step: `chariox terminal close --terminal ${terminalId}` })
        })
      }
    } finally {
      options.signals?.off("SIGINT", onInt); options.signals?.off("SIGTERM", onTerm); options.signals?.off("SIGHUP", onHup)
    }
  }
}

/** The link stayed down until the exec's own --timeout ran out: say so, and where the command is, without touching it. */
function linkLost(options: KernelExecOptions, terminalId: string, timeoutSeconds: number): CommandFailure {
  const kernel = options.kernel ? ` --kernel ${options.kernel}` : ""
  const named = options.kernel ? `kernel ${options.kernel}` : "the kernel"
  return new CommandFailure("exec_link_lost",
    `the link to ${named} was down until the --timeout of ${timeoutSeconds} s ran out. The command was not stopped: it is still running, or has finished, on ${named} as terminal ${terminalId}. ` +
    `Collect it with chariox terminal wait${kernel} --terminal ${terminalId} and chariox terminal read${kernel} --terminal ${terminalId}, or stop it with chariox terminal close${kernel} --terminal ${terminalId}. ` +
    `A finished command whose output nobody read is dropped by the kernel 60 s after it ends.`,
    { terminal_id: terminalId, kernel: options.kernel ?? null, next_step: `chariox terminal wait${kernel} --terminal ${terminalId}` })
}

async function isGone(send: SendRequest, target: KernelTerminalTarget, terminalId: string): Promise<boolean> {
  try {
    const { terminals } = expectRoomTerminalResult(await send(listRoomTerminalsRequest(target)), "listed")
    return !terminals.some(terminal => terminal.terminal_id === terminalId)
  } catch {
    return false
  }
}
