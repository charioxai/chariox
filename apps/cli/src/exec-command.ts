import timers from "node:timers/promises"
import type { Readable, Writable } from "node:stream"
import type { ExecOutcome, ExecOutput } from "@chariox/kernel-client/kernel-types"
import { LocalIpcError } from "@chariox/kernel-client/ipc"
import { failureOf, lostInFlight } from "./kernel-command-failure.js"
import { CommandFailure } from "./agent-command-table.js"
import type { SendRequest } from "./kernel-files.js"

const CHUNK_BYTES = 256 * 1024

export type ExecOptions = { session?: string | undefined; workspace?: string | undefined; hostAgent?: string | undefined; command: readonly string[] }
export type ExecIo = { stdin: Readable; stdout: Writable; stderr: Writable; signals?: NodeJS.Process }

type Reply = { RoomTerminal: { result: { result: string; exec_id?: string; next_offset?: number; output?: ExecOutput } } }

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
  if (Boolean(options.session) === Boolean(options.workspace) || options.command.length === 0 || (options.hostAgent && !options.session)) {
    throw new CommandFailure("usage", "Usage: chariox exec [--kernel NAME] (--session ROOM | --workspace PATH) [--host-agent AGENT] -- argv...")
  }
  const status = await send({ RelayStatus: null }) as { RelayStatus: { status: { capabilities?: string[] } } }
  if (!status.RelayStatus.status.capabilities?.includes("room_exec_v1")) {
    throw new CommandFailure("kernel_incompatible", "this kernel does not support binary Room exec; update it")
  }
  let session = options.session
  if (!session) {
    const made = await send({ CreateSession: { workspace_id: options.workspace, worktree_id: options.workspace, alias: null, slice_ref: null, no_subscribe: true } }) as { SessionCreated: { session: { id: string } } }
    session = made.SessionCreated.session.id
  }
  const request = async (operation: Record<string, unknown>) => {
    const response = await send({ RoomTerminal: { session_id: session, ...(options.hostAgent ? { host_agent: options.hostAgent } : {}), operation } }) as Reply
    return response.RoomTerminal.result
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
    // The kernel's capability check covers remote kernels; local old kernels
    // reject this new operation before starting anything.
    const opened = await request({ operation: "exec_open", command: options.command })
    if (opened.result !== "exec_opened" || !opened.exec_id) throw new Error("kernel returned no exec ID")
    execId = opened.exec_id
    if (stopped) interrupt(interrupted ?? 15)
    const inputRequest = async (operation: Record<string, unknown>) => {
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
        for await (const input of io.stdin) {
          const bytes = Buffer.isBuffer(input) ? input : Buffer.from(input)
          for (let start = 0; start < bytes.length && !stopped; start += CHUNK_BYTES) {
            const chunk = bytes.subarray(start, start + CHUNK_BYTES)
            const response = await inputRequest({ operation: "exec_write", exec_id: execId, offset, data_base64: chunk.toString("base64") })
            if (!response) return
            if (response.next_offset !== offset + chunk.length) throw new Error("kernel acknowledged an unexpected stdin offset")
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
        const output = response.output
        if (response.result !== "exec_output" || !output) throw new Error("kernel returned no exec output")
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
      io.stdin.destroy()
      feeding.catch(() => undefined)
      reading.catch(() => undefined)
    }
    if (cancellation) await cancellation
    await request({ operation: "exec_close", exec_id: execId })
    completed = true
    if (interrupted !== undefined) return 128 + interrupted
    if (outcome.kind === "failed") throw new Error(outcome.message)
    return outcome.kind === "exited" ? outcome.code : 128 + outcome.signal
  } catch (error) {
    // A probed grant-ending loss can follow admission; it is not a server refusal.
    const uncertainOpen = ["kernel_request_timeout", "kernel_connection_lost", "kernel_outcome_unknown"].includes(codeOf(error) ?? "")
    const definiteRefusal = !uncertainOpen && (error instanceof LocalIpcError
      ? error.operation === "handle kernel response" && !lostInFlight(error)
      : /^(exec_|terminal_)/.test(codeOf(error) ?? ""))
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
      session_id: session, exec_id: execId ?? null, stdin_offset: offset, output_cursor: cursor,
      ...(!completed ? { next_step: execId
        ? "resume this exec with RoomTerminal exec_read, or exec_close to cancel; do not rerun an uncertain command"
        : "EndSession on the retained Room cancels any uncollected command; do not rerun an uncertain start" } : {}),
    })
  } finally {
    stopped = true
    stopInput.abort()
    signals?.off("SIGINT", onInt)
    signals?.off("SIGTERM", onTerm)
    signals?.off("SIGHUP", onHup)
    if (!options.session && completed) {
      try { await send({ EndSession: { session_id: session } }) } catch (error) {
        // Room cleanup must never replace the command's exit or its real error.
        const warning = { code: "exec_room_cleanup_failed", session_id: session,
          message: failureOf(error).message, next_step: "EndSession on this Room to remove it" }
        await write(io.stderr, Buffer.from(`${JSON.stringify({ warning })}\n`), new AbortController().signal).catch(() => undefined)
      }
    }
  }
}
