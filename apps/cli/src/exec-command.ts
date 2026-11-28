import timers from "node:timers/promises"
import type { Readable, Writable } from "node:stream"
import type { CommandTarget, ExecOutcome, RunCommandOperation, RunCommandResult } from "@chariox/kernel-client/kernel-types"
import { runCommandMinimumProtocolVersion, runCommandRequest } from "@chariox/kernel-client/ipc-requests"
import { KernelProtocolMinimumError, withProtocolMinimum } from "./protocol-minimum-diagnostic.js"
import { LocalIpcError } from "@chariox/kernel-client/ipc"
import { failureOf, lostInFlight } from "./kernel-command-failure.js"
import { CommandFailure } from "./agent-command-table.js"
import type { SendRequest } from "./kernel-files.js"

const CHUNK_BYTES = 256 * 1024

export type ExecOptions = { session?: string | undefined; workspace?: string | undefined; hostAgent?: string | undefined; timeoutSeconds?: number | undefined; directory?: string | undefined; stdinFile?: string | undefined; command: readonly string[] }
export type ExecIo = { stdin: Readable | undefined; stdout: Writable; stderr: Writable; signals?: NodeJS.Process }

type Reply = { RunCommand: { result: RunCommandResult } }

function codeOf(error: unknown): string | undefined {
  return error && typeof error === "object" && "code" in error ? String(error.code) : undefined
}

function write(stream: Writable, bytes: Buffer, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    const abort = () => reject(new CommandFailure("exec_interrupted", "command interrupted"))
    if (signal.aborted) return abort()
    // Keep this listener until the pending write settles, including after an
    // interrupt: a pipe may report EPIPE after the command already returned.
    const failed = (error: Error) => { signal.removeEventListener("abort", abort); reject(error) }
    stream.once("error", failed)
    signal.addEventListener("abort", abort, { once: true })
    try {
      stream.write(bytes, error => {
        signal.removeEventListener("abort", abort)
        if (error) reject(error)
        else { stream.off("error", failed); resolve() }
      })
    } catch (error) {
      stream.off("error", failed)
      signal.removeEventListener("abort", abort)
      reject(error)
    }
  })
}

/** One command, concurrent bounded pipes, and no replay of an uncertain start. */
export async function runExec(options: ExecOptions, send: SendRequest, io: ExecIo): Promise<number> {
  if ((options.session && options.workspace) || options.command.length === 0 || (options.hostAgent && !options.session)
    || ((options.directory !== undefined || options.stdinFile !== undefined) && Boolean(options.session || options.workspace))) {
    throw new CommandFailure("usage", "Usage: chariox exec [--kernel NAME] [--session ROOM [--host-agent AGENT] | --workspace PATH] [--timeout SECONDS] -- argv...")
  }
  const target: CommandTarget = options.session
    ? options.hostAgent
      ? { kind: "agent_slice", session_id: options.session, agent_id: options.hostAgent }
      : { kind: "room_workspace", session_id: options.session }
    : { kind: "kernel" }
  const request = async (operation: RunCommandOperation) => {
    const body = runCommandRequest(target, operation)
    try {
      const response = await withProtocolMinimum(() => send(body), {
        capability: "Command execution", requestVariant: "RunCommand",
        minimumProtocolVersion: runCommandMinimumProtocolVersion,
      }) as Reply
      return response.RunCommand.result
    } catch (error) {
      if (error instanceof KernelProtocolMinimumError) throw new CommandFailure("kernel_incompatible", error.message)
      throw error
    }
  }
  let execId: string | undefined
  let offset = 0
  let cursor = 0
  let completed = false
  let stopped = false
  let interrupted: number | undefined
  const stopInput = new AbortController()
  const stopOutput = new AbortController()
  let cancellation: Promise<unknown> | undefined
  const interrupt = (signal: number) => {
    interrupted = signal
    stopped = true
    stopInput.abort()
    stopOutput.abort()
    if (execId && !cancellation) {
      cancellation = request({ operation: "exec_cancel", exec_id: execId })
      // The read loop reports a refusal or a lost connection with resume details.
      cancellation.catch(() => undefined)
    }
  }
  const onInt = () => interrupt(2)
  const onTerm = () => interrupt(15)
  const onHup = () => interrupt(1)
  const signals = io.signals
  signals?.on("SIGINT", onInt)
  signals?.on("SIGTERM", onTerm)
  signals?.on("SIGHUP", onHup)
  try {
    const directory = options.directory ?? options.workspace
    const opened = await request({
      operation: "exec_open", command: options.command,
      ...(directory !== undefined ? { directory } : {}),
      ...(options.stdinFile !== undefined ? { stdin_file: options.stdinFile } : {}),
      timeout_ms: Math.ceil((options.timeoutSeconds ?? 30) * 1000),
    })
    if (opened.result !== "exec_opened" || !opened.exec_id) throw new Error("kernel returned no exec ID")
    execId = opened.exec_id
    if (stopped) interrupt(interrupted ?? 15)
    const inputRequest = async (operation: RunCommandOperation) => {
      for (;;) {
        if (stopped) return
        try { return await request(operation) } catch (error) {
          if (codeOf(error) === "exec_stdin_closed") return
          if (codeOf(error) !== "exec_input_busy") throw error
          await timers.setTimeout(10, undefined, { signal: stopInput.signal }).catch(() => undefined)
        }
      }
    }
    // Destroying stdin is local cleanup: it releases an idle terminal read when
    // a command exits without consuming input or the user interrupts it.
    const feeding = (async () => {
      try {
        for await (const input of io.stdin ?? []) {
          const bytes = Buffer.isBuffer(input) ? input : Buffer.from(input)
          for (let start = 0; start < bytes.length && !stopped; start += CHUNK_BYTES) {
            const chunk = bytes.subarray(start, start + CHUNK_BYTES)
            const response = await inputRequest({ operation: "exec_write", exec_id: execId, offset, data_base64: chunk.toString("base64") })
            if (!response) return
            if (response.result !== "exec_input" || response.next_offset !== offset + chunk.length) throw new Error("kernel acknowledged an unexpected stdin offset")
            offset = response.next_offset
          }
          if (stopped) return
        }
        await inputRequest({ operation: "exec_stdin_close", exec_id: execId, offset })
      } catch (error) {
        if (!stopped) throw error
      }
    })()
    const reading = (async () => {
      for (;;) {
        const response = await request({ operation: "exec_read", exec_id: execId, after_cursor: cursor, wait_ms: 1000 })
        if (response.result !== "exec_output") throw new Error("kernel returned no exec output")
        const output = response.output
        const consumed = new AbortController()
        const renewing = (async () => {
          while (!consumed.signal.aborted) {
            await timers.setTimeout(60_000, undefined, { signal: consumed.signal }).catch(() => undefined)
            if (consumed.signal.aborted) return
            // A local consumer may stall beyond the reader lease. Acknowledge
            // only bytes already written, leaving this page available to resume.
            await request({ operation: "exec_read", exec_id: execId, after_cursor: cursor, wait_ms: 0 })
          }
        })()
        try {
          await Promise.race([renewing, (async () => {
            for (const chunk of output.chunks) {
              if (chunk.cursor !== cursor + 1) throw new Error("kernel returned unordered exec output")
              await write(chunk.stream === "stdout" ? io.stdout : io.stderr, Buffer.from(chunk.data_base64, "base64"), stopOutput.signal)
              cursor = chunk.cursor
            }
          })()])
        } finally {
          consumed.abort()
          // Settle an in-flight renewal before the next read acknowledges more.
          await renewing
        }
        if (output.next_cursor !== cursor) throw new Error("kernel returned an unexpected exec cursor")
        if (output.outcome) return output.outcome
      }
    })()
    // Either pipe can fail first; observe both promises without leaving an
    // unhandled rejection while the other is backpressured.
    const inputFailure = feeding.then(() => new Promise<never>(() => {}))
    let outcome: ExecOutcome
    try { outcome = await Promise.race([reading, inputFailure]) } finally {
      stopped = true
      stopInput.abort()
      stopOutput.abort()
      io.stdin?.destroy()
      feeding.catch(() => undefined)
      reading.catch(() => undefined)
    }
    if (cancellation) await cancellation
    await request({ operation: "exec_close", exec_id: execId })
    completed = true
    if (interrupted !== undefined) return 128 + interrupted
    if (outcome.kind === "failed") throw new Error(outcome.message)
    if (outcome.kind === "memory_limit") throw new CommandFailure("memory_limit", `command stopped at its memory limit (${outcome.limit_bytes} bytes)`)
    if (outcome.kind === "timed_out") {
      const elapsed = `command didn't finish in ${outcome.timeout_ms / 1000} s`
      if (outcome.exit.kind === "not_started") throw new CommandFailure("exec_timed_out", `${elapsed} and did not start before the deadline`, { outcome })
      const status = outcome.exit.kind === "exited" ? `exit code ${outcome.exit.code}` : `signal ${outcome.exit.signal}`
      throw new CommandFailure("exec_timed_out", `${elapsed} and was stopped (${status})`, { outcome })
    }
    return outcome.kind === "exited" ? outcome.code : 128 + outcome.signal
  } catch (error) {
    // A probed grant-ending loss can follow admission; it is not a server refusal.
    const uncertainOpen = ["kernel_request_timeout", "kernel_connection_lost", "kernel_outcome_unknown"].includes(codeOf(error) ?? "")
    const definiteRefusal = !uncertainOpen && (error instanceof LocalIpcError
      ? error.operation === "handle kernel response" && !lostInFlight(error)
      : /^(exec_|kernel_incompatible$)/.test(codeOf(error) ?? ""))
    if (!execId && definiteRefusal) completed = true
    if (execId && codeOf(error) === "exec_interrupted" && interrupted !== undefined) {
      try {
        if (cancellation) await cancellation
        await request({ operation: "exec_close", exec_id: execId })
        completed = true
        return 128 + interrupted
      } catch (cancelError) { error = cancelError }
    }
    // A broken local pipe is a definite reason to stop the remote producer.
    if (execId && /^E[A-Z]+$/.test(codeOf(error) ?? "")) {
      try { await request({ operation: "exec_close", exec_id: execId }); completed = true } catch { /* Resume details below retain uncertain cleanup. */ }
    }
    const failure = failureOf(error)
    throw new CommandFailure(failure.code, failure.message, {
      ...failure.details, target, exec_id: execId ?? null, stdin_offset: offset, output_cursor: cursor,
      ...(!completed ? { next_step: execId
        ? "resume this command with RunCommand exec_read, or exec_close to stop it; do not rerun an uncertain command"
        : "list commands on this target with RunCommand exec_list before retrying" } : {}),
    })
  } finally {
    stopped = true
    stopInput.abort()
    signals?.off("SIGINT", onInt)
    signals?.off("SIGTERM", onTerm)
    signals?.off("SIGHUP", onHup)
  }
}
