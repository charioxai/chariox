import assert from "node:assert/strict"
import test from "node:test"
import { PassThrough, Readable, Writable } from "node:stream"
import { EventEmitter } from "node:events"
import { runExec } from "./exec-command.js"
import { CommandFailure, parseArguments } from "./agent-command-table.js"

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

test("exec SIGTERM cancels and closes even while local stdout is blocked", async () => {
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
  signals.emit("SIGTERM")
  assert.equal(await code, 143)
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
