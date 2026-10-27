import assert from "node:assert/strict"
import test from "node:test"
import { PassThrough, Readable, Writable } from "node:stream"
import { EventEmitter } from "node:events"
import { runExec } from "./exec-command.js"
import { LocalIpcError } from "@chariox/kernel-client/ipc"
import { CommandFailure, exitCodeFor, parseArguments } from "./agent-command-table.js"

function sink(chunks: Buffer[]): Writable {
  return new Writable({ write(chunk, _encoding, done) { chunks.push(Buffer.from(chunk)); done() } })
}

const options = { session: "room", command: ["/bin/cat"] }
function result(value: object) { return { RoomTerminal: { result: value } } }
function capabilities() { return { RelayStatus: { status: { capabilities: ["room_exec_v1"] } } } }

test("exec parses its flags separately from literal argv", () => {
  assert.equal(parseArguments("exec", ["--session", "room", "--kernel", "worker"]).flags.get("--kernel"), "worker")
})

test("exec drains early binary output while its input remains open", async () => {
  const input = new PassThrough()
  const stdout: Buffer[] = [], stderr: Buffer[] = []
  let accepted = 0, reads = 0, closed = false
  const pending: (() => void)[] = []
  const send = async (body: Record<string, unknown>) => {
    if ("RelayStatus" in body) return capabilities()
    const operation = (body.RoomTerminal as { operation: Record<string, unknown> }).operation
    switch (operation.operation) {
      case "exec_open": return result({ result: "exec_opened", exec_id: "exec-1" })
      case "exec_write": {
        const bytes = Buffer.from(operation.data_base64 as string, "base64")
        assert.equal(operation.offset, accepted)
        assert.deepEqual(bytes, Buffer.from([0, 255, 13, 10]))
        accepted += bytes.length
        return result({ result: "exec_input", next_offset: accepted })
      }
      case "exec_stdin_close":
        assert.equal(operation.offset, 4); closed = true
        for (const resolve of pending) resolve()
        return result({ result: "exec_input", next_offset: 4 })
      case "exec_read": {
        reads++
        if (reads === 1) return result({ result: "exec_output", output: { chunks: [
          { cursor: 1, stream: "stdout", data_base64: "AP8NCg==" },
          { cursor: 2, stream: "stderr", data_base64: "ZXJy" },
        ], next_cursor: 2, outcome: null } })
        assert.equal(operation.after_cursor, 2)
        if (!closed) await new Promise<void>(resolve => pending.push(resolve))
        return result({ result: "exec_output", output: { chunks: [], next_cursor: 2, outcome: { kind: "exited", code: 23 } } })
      }
      case "exec_close": return result({ result: "exec_closed" })
      default: throw new Error("unexpected operation")
    }
  }
  const stdoutSink = sink(stdout)
  const code = runExec(options, send, { stdin: input, stdout: stdoutSink, stderr: sink(stderr) })
  while (stdout.length === 0) await new Promise(resolve => setImmediate(resolve))
  assert.equal(accepted, 0, "output arrives before any stdin")
  assert.deepEqual(Buffer.concat(stdout), Buffer.from([0, 255, 13, 10]))
  assert.equal(Buffer.concat(stderr).toString(), "err")
  input.end(Buffer.from([0, 255, 13, 10]))
  assert.equal(await code, 23)
})

test("exec exits and releases an idle stdin even when the program ignores input", async () => {
  const input = new PassThrough()
  const calls: string[] = []
  const send = async (body: Record<string, unknown>) => {
    if ("RelayStatus" in body) return capabilities()
    const op = (body.RoomTerminal as { operation: { operation: string } }).operation.operation
    calls.push(op)
    return result(op === "exec_open" ? { result: "exec_opened", exec_id: "exec-1" }
      : op === "exec_read" ? { result: "exec_output", output: { chunks: [], next_cursor: 0, outcome: { kind: "signaled", signal: 15 } } }
        : { result: "exec_closed" })
  }
  assert.equal(await runExec(options, send, { stdin: input, stdout: sink([]), stderr: sink([]) }), 143)
  assert.equal(input.destroyed, true)
  assert.deepEqual(calls, ["exec_open", "exec_read", "exec_close"])
})

test("exec refuses an older kernel before creating a temporary Room", async () => {
  const calls: string[] = []
  await assert.rejects(runExec({ workspace: "/work", command: ["cat"] }, async body => {
    calls.push(Object.keys(body)[0]!)
    return { RelayStatus: { status: { capabilities: ["room_terminal_v1"] } } }
  }, { stdin: Readable.from([]), stdout: sink([]), stderr: sink([]) }), /update it/)
  assert.deepEqual(calls, ["RelayStatus"])
})

test("exec retries only a busy input offset while the output consumer continues", async () => {
  let attempted = false, busy = true, closed = false
  const send = async (body: Record<string, unknown>) => {
    if ("RelayStatus" in body) return capabilities()
    const operation = (body.RoomTerminal as { operation: Record<string, unknown> }).operation
    if (operation.operation === "exec_open") return result({ result: "exec_opened", exec_id: "exec-1" })
    if (operation.operation === "exec_write") {
      assert.equal(operation.offset, 0); attempted = true
      if (busy) { busy = false; throw new CommandFailure("exec_input_busy", "full") }
      return result({ result: "exec_input", next_offset: 1 })
    }
    if (operation.operation === "exec_stdin_close") { closed = true; return result({ result: "exec_input", next_offset: 1 }) }
    if (operation.operation === "exec_read") {
      while (!closed) await new Promise(resolve => setTimeout(resolve, 1))
      return result({ result: "exec_output", output: { chunks: [], next_cursor: 0, outcome: { kind: "exited", code: 0 } } })
    }
    return result({ result: "exec_closed" })
  }
  assert.equal(await runExec(options, send, { stdin: Readable.from([Buffer.from("a")]), stdout: sink([]), stderr: sink([]) }), 0)
  assert.equal(attempted, true)
})

for (const [signal, exit] of [["SIGTERM", 143], ["SIGHUP", 129]] as const) test(`exec ${signal} cancels and closes even while local stdout is blocked`, async () => {
  const input = new PassThrough(), signals = new EventEmitter() as NodeJS.Process
  const calls: string[] = []
  let writing: (() => void) | undefined
  const blocked = new Writable({ write(_chunk, _encoding, done) { writing = done } })
  const send = async (body: Record<string, unknown>) => {
    if ("RelayStatus" in body) return capabilities()
    const op = (body.RoomTerminal as { operation: { operation: string } }).operation.operation
    calls.push(op)
    return result(op === "exec_open" ? { result: "exec_opened", exec_id: "exec-1" }
      : op === "exec_read" ? { result: "exec_output", output: { chunks: [{ cursor: 1, stream: "stdout", data_base64: "YQ==" }], next_cursor: 1, outcome: null } }
        : { result: op === "exec_cancel" ? "exec_cancelled" : "exec_closed" })
  }
  const code = runExec(options, send, { stdin: input, stdout: blocked, stderr: sink([]), signals })
  while (!writing) await new Promise(resolve => setImmediate(resolve))
  signals.emit(signal)
  assert.equal(await code, exit)
  assert.deepEqual(calls, ["exec_open", "exec_read", "exec_cancel", "exec_close"])
  writing()
})

test("exec keeps resume details when cancellation loses its connection", async () => {
  const input = new PassThrough(), signals = new EventEmitter() as NodeJS.Process
  let writing: (() => void) | undefined
  const blocked = new Writable({ write(_chunk, _encoding, done) { writing = done } })
  const send = async (body: Record<string, unknown>) => {
    if ("RelayStatus" in body) return capabilities()
    const op = (body.RoomTerminal as { operation: { operation: string } }).operation.operation
    if (op === "exec_cancel") throw new CommandFailure("kernel_connection_lost", "lost cancellation")
    return result(op === "exec_open" ? { result: "exec_opened", exec_id: "exec-1" }
      : { result: "exec_output", output: { chunks: [{ cursor: 1, stream: "stdout", data_base64: "YQ==" }], next_cursor: 1, outcome: null } })
  }
  const code = runExec(options, send, { stdin: input, stdout: blocked, stderr: sink([]), signals })
  const rejected = assert.rejects(code, (error: unknown) => error instanceof CommandFailure
    && error.code === "kernel_connection_lost" && error.details?.exec_id === "exec-1"
    && error.details?.output_cursor === 0)
  while (!writing) await new Promise(resolve => setImmediate(resolve))
  signals.emit("SIGTERM")
  await rejected
  writing()
})

test("exec handles a pipe error arriving after interrupted stdout", async () => {
  const input = new PassThrough(), signals = new EventEmitter() as NodeJS.Process
  let writing: ((error: Error) => void) | undefined
  const blocked = new Writable({ write(_chunk, _encoding, done) { writing = done } })
  const send = async (body: Record<string, unknown>) => {
    if ("RelayStatus" in body) return capabilities()
    const op = (body.RoomTerminal as { operation: { operation: string } }).operation.operation
    return result(op === "exec_open" ? { result: "exec_opened", exec_id: "exec-1" }
      : op === "exec_read" ? { result: "exec_output", output: { chunks: [{ cursor: 1, stream: "stdout", data_base64: "YQ==" }], next_cursor: 1, outcome: null } }
        : { result: op === "exec_cancel" ? "exec_cancelled" : "exec_closed" })
  }
  const code = runExec(options, send, { stdin: input, stdout: blocked, stderr: sink([]), signals })
  while (!writing) await new Promise(resolve => setImmediate(resolve))
  signals.emit("SIGTERM")
  assert.equal(await code, 143)
  writing(Object.assign(new Error("broken pipe"), { code: "EPIPE" }))
  await new Promise(resolve => setImmediate(resolve))
})

for (const [operation, code, expected, exit] of [
  ["handle kernel response", "connection_closed", "kernel_connection_lost", 5],
  ["handle kernel response", "outcome_unknown", "kernel_connection_lost", 5],
  ["handle kernel response", "write_failed", "kernel_connection_lost", 5],
  ["connect kernel websocket", "connection_closed", "kernel_unreachable", 4],
] as const) test(`exec normalizes real LocalIpcError ${operation}/${code} with resume details`, async () => {
  await assert.rejects(runExec(options, async body => {
    if ("RelayStatus" in body) return capabilities()
    throw new LocalIpcError(operation, "transport lost", code)
  }, { stdin: Readable.from([]), stdout: sink([]), stderr: sink([]) }), (error: unknown) => {
    assert.ok(error instanceof CommandFailure)
    assert.equal(error.code, expected)
    assert.equal(exitCodeFor(error.code), exit)
    assert.equal(error.details?.session_id, "room")
    return true
  })
})

for (const code of ["terminal_refused", "kernel_access_ended", "kernel_access_denied", "exec_spawn_failed", "future_open_refusal"]) {
  test(`exec removes its temporary Room after definite ${code}`, async () => {
    const calls: string[] = []
    await assert.rejects(runExec({ workspace: "/work", command: ["cat"] }, async body => {
      calls.push(Object.keys(body)[0]!)
      if ("RelayStatus" in body) return capabilities()
      if ("CreateSession" in body) return { SessionCreated: { session: { id: "temp-room" } } }
      if ("EndSession" in body) return {}
      throw new LocalIpcError("handle kernel response", "open refused", code)
    }, { stdin: Readable.from([]), stdout: sink([]), stderr: sink([]) }), (error: unknown) => {
      assert.ok(error instanceof CommandFailure)
      assert.equal(error.code, code)
      assert.equal(error.details?.next_step, undefined)
      return true
    })
    assert.equal(calls.at(-1), "EndSession")
  })
}

test("exec cleanup failure preserves a command exit and reports the retained Room", async () => {
  const warnings: Buffer[] = []
  const code = await runExec({ workspace: "/work", command: ["false"] }, async body => {
    if ("RelayStatus" in body) return capabilities()
    if ("CreateSession" in body) return { SessionCreated: { session: { id: "temp-room" } } }
    if ("EndSession" in body) throw new Error("cleanup lost")
    const op = (body.RoomTerminal as { operation: { operation: string } }).operation.operation
    return result(op === "exec_open" ? { result: "exec_opened", exec_id: "exec-1" }
      : op === "exec_read" ? { result: "exec_output", output: { chunks: [], next_cursor: 0, outcome: { kind: "exited", code: 23 } } }
        : { result: "exec_closed" })
  }, { stdin: Readable.from([]), stdout: sink([]), stderr: sink(warnings) })
  assert.equal(code, 23)
  assert.match(Buffer.concat(warnings).toString(), /temp-room/)
})

test("exec cleanup failure preserves the original open refusal", async () => {
  await assert.rejects(runExec({ workspace: "/work", command: ["cat"] }, async body => {
    if ("RelayStatus" in body) return capabilities()
    if ("CreateSession" in body) return { SessionCreated: { session: { id: "temp-room" } } }
    if ("EndSession" in body) throw new Error("cleanup lost")
    throw new LocalIpcError("handle kernel response", "open refused", "terminal_refused")
  }, { stdin: Readable.from([]), stdout: sink([]), stderr: sink([]) }), (error: unknown) => error instanceof CommandFailure && error.code === "terminal_refused")
})


test("exec preserves the interrupt exit when canceled pipes report incomplete output", async () => {
  const signals = new EventEmitter() as NodeJS.Process
  let reading = false
  let cancelled!: () => void
  const done = new Promise<void>(resolve => { cancelled = resolve })
  const code = runExec(options, async body => {
    if ("RelayStatus" in body) return capabilities()
    const operation = (body.RoomTerminal as { operation: { operation: string } }).operation.operation
    if (operation === "exec_open") return result({ result: "exec_opened", exec_id: "exec-1" })
    if (operation === "exec_cancel") { cancelled(); return result({ result: "exec_cancelled" }) }
    if (operation === "exec_read") {
      reading = true; await done
      return result({ result: "exec_output", output: { chunks: [], next_cursor: 0, outcome: { kind: "failed", message: "output stream incomplete" } } })
    }
    return result({ result: "exec_closed" })
  }, { stdin: new PassThrough(), stdout: sink([]), stderr: sink([]), signals })
  while (!reading) await new Promise(resolve => setImmediate(resolve))
  signals.emit("SIGINT")
  assert.equal(await code, 130)
})

for (const [code, probedLoss] of [
  ["kernel_request_timeout", false], ["kernel_connection_lost", false],
  ["kernel_outcome_unknown", false], ["kernel_access_ended", true],
] as const) test(`uncertain open retains its Room after ${code} (probe=${probedLoss})`, async () => {
  let ended = false
  await assert.rejects(runExec({ workspace: "/work", command: ["cat"] }, async body => {
    if ("RelayStatus" in body) return capabilities()
    if ("CreateSession" in body) return { SessionCreated: { session: { id: "temp-room" } } }
    if ("EndSession" in body) { ended = true; return {} }
    throw probedLoss ? new CommandFailure(code, "grant ended after an uncertain write")
      : new LocalIpcError("handle kernel response", "gateway outcome is unknown", code)
  }, { stdin: Readable.from([]), stdout: sink([]), stderr: sink([]) }), (error: unknown) => {
    assert.ok(error instanceof CommandFailure)
    assert.match(String(error.details?.next_step), /retained Room/)
    return true
  })
  assert.equal(ended, false)
})
