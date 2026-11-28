import type { ProgramInput } from "@chariox/kernel-client/kernel-types"
import { fstatSync } from "node:fs"
import { Readable, type Writable } from "node:stream"
import { CommandFailure } from "./agent-command-table.js"
import { runExec } from "./exec-command.js"
import type { SendRequest } from "./kernel-files.js"

export const MAX_INLINE_STDIN_BYTES = 64 * 1024
export const EXEC_REQUEST_MARGIN_SECONDS = 30
export type KernelExecOptions = {
  command: readonly string[]; directory?: string | undefined; timeoutSeconds: number; stdin?: ProgramInput | undefined; signals?: NodeJS.Process
}

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

/** Keeps the kernel CLI's input rules while the shared command runner owns all execution. */
export function runKernelExec(options: KernelExecOptions, send: SendRequest, stdout: Writable): Promise<number> {
  const stdin = options.stdin?.kind === "inline"
    ? Readable.from([Buffer.from(options.stdin.data_base64, "base64")])
    : undefined
  return runExec({
    command: options.command, directory: options.directory, timeoutSeconds: options.timeoutSeconds,
    stdinFile: options.stdin?.kind === "file" ? options.stdin.path : undefined,
  }, send, { stdin, stdout, stderr: process.stderr, ...(options.signals ? { signals: options.signals } : {}) })
}
