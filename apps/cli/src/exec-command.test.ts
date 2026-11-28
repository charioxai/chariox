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
function result(value: object) { return { RunCommand: { result: value } } }

for (const stream of ["stdout", "stderr"] as const) test(`exec renews its lease while a finished page blocks on ${stream}`, async t => {
  t.mock.timers.enable({ apis: ["setTimeout", "Date"] })
  let lastRead = Date.now(), renewals = 0
  let release: (() => void) | undefined
  const bytes: Buffer[] = []
  const blocked = new Writable({ write(chunk, _encoding, done) {
    bytes.push(Buffer.from(chunk))
    if (bytes.length === 1) done()
    else release = done
  } })
  const page = { chunks: [
    { cursor: 1, stream, data_base64: "YQ==" },
    { cursor: 2, stream, data_base64: "AP8NCg==" },
  ], next_cursor: 2, outcome: { kind: "exited", code: 23 } }
  const send = async (body: Record<string, unknown>) => {
    const op = (body.RunCommand as { operation: Record<string, unknown> }).operation
    if (op.operation === "exec_open") return result({ result: "exec_opened", exec_id: "exec-1" })
    if (["exec_read", "exec_close"].includes(String(op.operation))) {
      if (Date.now() - lastRead >= 300_000) throw new CommandFailure("exec_not_found", "reader lease expired")
      if (op.operation === "exec_read") {
        lastRead = Date.now()
        if (op.wait_ms === 0) {
          renewals++
          assert.equal(op.after_cursor, 1, "only locally consumed bytes may be acknowledged")
          return result({ result: "exec_output", output: { ...page, chunks: page.chunks.slice(1) } })
        }
        assert.equal(op.after_cursor, 0)
        return result({ result: "exec_output", output: page })
      }
    }
    return result({ result: "exec_closed" })
  }
  const running = runExec(options, send, { stdin: Readable.from([]), stdout: stream === "stdout" ? blocked : sink([]), stderr: stream === "stderr" ? blocked : sink([]) })
  running.catch(() => undefined)
  while (!release) await new Promise(resolve => setImmediate(resolve))
  for (let i = 0; i < 6; i++) {
    t.mock.timers.tick(60_000)
    await new Promise(resolve => setImmediate(resolve))
  }
  release()
  assert.equal(await running, 23)
  assert.ok(renewals >= 5)
  assert.deepEqual(Buffer.concat(bytes), Buffer.from([97, 0, 255, 13, 10]))
  const settled = renewals
  t.mock.timers.tick(600_000)
  await new Promise(resolve => setImmediate(resolve))
  assert.equal(renewals, settled, "no renewals after exec_close")
})

test("exec parses its flags separately from literal argv", () => {
  assert.equal(parseArguments("exec", ["--session", "room", "--kernel", "worker"]).flags.get("--kernel"), "worker")
})

test("exec reports a lease renewal failure while local output remains blocked", async t => {
  t.mock.timers.enable({ apis: ["setTimeout", "Date"] })
  let release: (() => void) | undefined
  const blocked = new Writable({ write(_chunk, _encoding, done) { release = done } })
  const send = async (body: Record<string, unknown>) => {
    const op = (body.RunCommand as { operation: Record<string, unknown> }).operation
    if (op.operation === "exec_open") return result({ result: "exec_opened", exec_id: "exec-1" })
    if (op.operation === "exec_read") {
      if (op.wait_ms === 0) throw new CommandFailure("kernel_connection_lost", "lost during lease renewal")
      return result({ result: "exec_output", output: { chunks: [{ cursor: 1, stream: "stdout", data_base64: "YQ==" }], next_cursor: 1, outcome: null } })
    }
    return result({ result: "exec_input", next_offset: 0 })
  }
  const running = runExec(options, send, { stdin: Readable.from([]), stdout: blocked, stderr: sink([]) })
  const rejected = assert.rejects(running, (error: unknown) => {
    assert.ok(error instanceof CommandFailure)
    assert.equal(error.code, "kernel_connection_lost")
    assert.equal(error.details?.output_cursor, 0)
    return true
  })
  while (!release) await new Promise(resolve => setImmediate(resolve))
  t.mock.timers.tick(60_000)
  await rejected
  release()
})

test("exec settles a pending lease renewal before acknowledging the next page", async t => {
  t.mock.timers.enable({ apis: ["setTimeout"] })
  let releaseWrite: (() => void) | undefined, releaseRenewal: (() => void) | undefined
  let reads = 0
  const blocked = new Writable({ write(_chunk, _encoding, done) { releaseWrite = done } })
  const send = async (body: Record<string, unknown>) => {
    const op = (body.RunCommand as { operation: Record<string, unknown> }).operation
    if (op.operation === "exec_open") return result({ result: "exec_opened", exec_id: "exec-1" })
    if (op.operation === "exec_read") {
      if (op.wait_ms === 0) {
        assert.equal(op.after_cursor, 0)
        await new Promise<void>(resolve => { releaseRenewal = resolve })
        return result({ result: "exec_output" })
      }
      reads++
      assert.equal(op.after_cursor, reads - 1)
      return result({ result: "exec_output", output: reads === 1
        ? { chunks: [{ cursor: 1, stream: "stdout", data_base64: "YQ==" }], next_cursor: 1, outcome: null }
        : { chunks: [], next_cursor: 1, outcome: { kind: "exited", code: 23 } } })
    }
    return result({ result: "exec_closed" })
  }
  const running = runExec(options, send, { stdin: Readable.from([]), stdout: blocked, stderr: sink([]) })
  while (!releaseWrite) await new Promise(resolve => setImmediate(resolve))
  t.mock.timers.tick(60_000)
  while (!releaseRenewal) await new Promise(resolve => setImmediate(resolve))
  releaseWrite()
  await new Promise(resolve => setImmediate(resolve))
  assert.equal(reads, 1)
  releaseRenewal()
  assert.equal(await running, 23)
  assert.equal(reads, 2)
})

test("exec drains early binary output while its input remains open", async () => {
  const input = new PassThrough()
  const stdout: Buffer[] = [], stderr: Buffer[] = []
  let accepted = 0, reads = 0, closed = false
  const pending: (() => void)[] = []
  const send = async (body: Record<string, unknown>) => {
    const operation = (body.RunCommand as { operation: Record<string, unknown> }).operation
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
    const op = (body.RunCommand as { operation: { operation: string } }).operation.operation
    calls.push(op)
    return result(op === "exec_open" ? { result: "exec_opened", exec_id: "exec-1" }
      : op === "exec_read" ? { result: "exec_output", output: { chunks: [], next_cursor: 0, outcome: { kind: "signaled", signal: 15 } } }
        : { result: "exec_closed" })
  }
  assert.equal(await runExec(options, send, { stdin: input, stdout: sink([]), stderr: sink([]) }), 143)
  assert.equal(input.destroyed, true)
  assert.deepEqual(calls, ["exec_open", "exec_read", "exec_close"])
})

test("exec retries only a busy input offset while the output consumer continues", async () => {
  let attempted = false, busy = true, closed = false
  const send = async (body: Record<string, unknown>) => {
    const operation = (body.RunCommand as { operation: Record<string, unknown> }).operation
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
    const op = (body.RunCommand as { operation: { operation: string } }).operation.operation
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
    const op = (body.RunCommand as { operation: { operation: string } }).operation.operation
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
    const op = (body.RunCommand as { operation: { operation: string } }).operation.operation
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
    throw new LocalIpcError(operation, "transport lost", code)
  }, { stdin: Readable.from([]), stdout: sink([]), stderr: sink([]) }), (error: unknown) => {
    assert.ok(error instanceof CommandFailure)
    assert.equal(error.code, expected)
    assert.equal(exitCodeFor(error.code), exit)
    assert.deepEqual(error.details?.target, { kind: "room_workspace", session_id: "room" })
    return true
  })
})

for (const code of ["kernel_access_ended", "exec_access_denied", "exec_spawn_failed", "future_open_refusal"]) {
  test(`exec preserves a definite ${code} refusal without allocating a Room`, async () => {
    const calls: string[] = []
    await assert.rejects(runExec({ workspace: "/work", command: ["cat"] }, async body => {
      calls.push(Object.keys(body)[0]!)
      throw new LocalIpcError("handle kernel response", "open refused", code)
    }, { stdin: undefined, stdout: sink([]), stderr: sink([]) }), (error: unknown) => {
      assert.ok(error instanceof CommandFailure)
      assert.equal(error.code, code)
      assert.equal(error.details?.next_step, undefined)
      return true
    })
    assert.deepEqual(calls, ["RunCommand"])
  })
}

test("exec opens the kernel target in the workspace directory and closes empty stdin", async () => {
  let eof = false
  const outcome = { kind: "timed_out", timeout_ms: 1000, exit: { kind: "signaled", signal: 9 } }
  await assert.rejects(runExec({ workspace: "/work", command: ["cat"], timeoutSeconds: 1 }, async body => {
    const request = body.RunCommand as import("@chariox/kernel-client/kernel-types").RunCommandRequest
    assert.deepEqual(request.target, { kind: "kernel" })
    if (request.operation.operation === "exec_open") {
      assert.deepEqual(request.operation, { operation: "exec_open", command: ["cat"], directory: "/work", timeout_ms: 1000 })
      return result({ result: "exec_opened", exec_id: "exec-1" })
    }
    if (request.operation.operation === "exec_stdin_close") {
      eof = true
      assert.equal(request.operation.offset, 0)
      return result({ result: "exec_input", next_offset: 0 })
    }
    if (request.operation.operation === "exec_read") {
      await new Promise(resolve => setImmediate(resolve))
      assert.equal(eof, true)
      return result({ result: "exec_output", output: { chunks: [], next_cursor: 0, outcome } })
    }
    return result({ result: "exec_closed" })
  }, { stdin: undefined, stdout: sink([]), stderr: sink([]) }), (error: unknown) => {
    assert.ok(error instanceof CommandFailure)
    assert.equal(error.code, "exec_timed_out")
    assert.deepEqual(error.details?.outcome, outcome)
    assert.match(error.message, /didn't finish in 1 s and was stopped \(signal 9\)/)
    return true
  })
})

test("exec preserves the interrupt exit when canceled pipes report incomplete output", async () => {
  const signals = new EventEmitter() as NodeJS.Process
  let reading = false
  let cancelled!: () => void
  const done = new Promise<void>(resolve => { cancelled = resolve })
  const code = runExec(options, async body => {
    const operation = (body.RunCommand as { operation: { operation: string } }).operation.operation
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
] as const) test(`uncertain open identifies its target after ${code} (probe=${probedLoss})`, async () => {
  await assert.rejects(runExec({ workspace: "/work", command: ["cat"] }, async body => {
    assert.deepEqual(Object.keys(body), ["RunCommand"])
    throw probedLoss ? new CommandFailure(code, "grant ended after an uncertain write")
      : new LocalIpcError("handle kernel response", "gateway outcome is unknown", code)
  }, { stdin: Readable.from([]), stdout: sink([]), stderr: sink([]) }), (error: unknown) => {
    assert.ok(error instanceof CommandFailure)
    assert.match(String(error.details?.next_step), /exec_list/)
    return true
  })
})


test("exec reports a typed memory stop after collecting the command", async () => {
  const calls: string[] = []
  const send = async (body: Record<string, unknown>) => {
    const op = (body.RunCommand as { operation: { operation: string } }).operation.operation
    calls.push(op)
    return result(op === "exec_open" ? { result: "exec_opened", exec_id: "exec-1" }
      : op === "exec_read" ? { result: "exec_output", output: { chunks: [], next_cursor: 0, outcome: { kind: "memory_limit", limit_bytes: 100663296 } } }
        : { result: "exec_closed" })
  }
  await assert.rejects(runExec(options, send, { stdin: new PassThrough(), stdout: sink([]), stderr: sink([]) }),
    (error: unknown) => error instanceof CommandFailure && error.code === "memory_limit" && error.message.includes("100663296 bytes"))
  assert.deepEqual(calls, ["exec_open", "exec_read", "exec_close"])
})

test("exec recovers an admitted cursor after a brief disconnect without replaying bytes or open", async () => {
  const bytes: Buffer[] = [], cursors: number[] = []
  let opens = 0
  const send = async (body: Record<string, unknown>) => {
    const op = (body.RunCommand as { operation: Record<string, unknown> }).operation
    if (op.operation === "exec_open") { opens++; return result({ result: "exec_opened", exec_id: "recover" }) }
    if (op.operation === "exec_stdin_close") return result({ result: "exec_input", next_offset: 0 })
    if (op.operation === "exec_close") return result({ result: "exec_closed" })
    assert.equal(op.operation, "exec_read")
    cursors.push(Number(op.after_cursor))
    if (cursors.length === 2) throw new CommandFailure("kernel_connection_lost", "brief disconnect")
    return result({ result: "exec_output", output: {
      chunks: [{ cursor: cursors.length === 1 ? 1 : 2, stream: "stdout", data_base64: (cursors.length === 1 ? Buffer.from([0, 255]) : Buffer.from([13, 10])).toString("base64") }],
      next_cursor: cursors.length === 1 ? 1 : 2,
      outcome: cursors.length === 1 ? null : { kind: "exited", code: 17 },
    } })
  }
  assert.equal(await runExec({ ...options, timeoutSeconds: 10 }, send, { stdin: Readable.from([]), stdout: sink(bytes), stderr: sink([]) }), 17)
  assert.equal(opens, 1)
  assert.deepEqual(cursors, [0, 1, 1])
  assert.deepEqual(Buffer.concat(bytes), Buffer.from([0, 255, 13, 10]))
})

test("exec never retries an uncertain open to recover output", async () => {
  let requests = 0
  await assert.rejects(runExec({ ...options, timeoutSeconds: 1 }, async () => {
    requests++
    throw new CommandFailure("kernel_outcome_unknown", "unknown admission")
  }, { stdin: undefined, stdout: sink([]), stderr: sink([]) }), (failure: unknown) => failure instanceof CommandFailure && failure.details?.exec_id === null)
  assert.equal(requests, 1)
})

test("exec stops output recovery at the remaining command deadline", async () => {
  let reads = 0
  const began = Date.now()
  await assert.rejects(runExec({ ...options, timeoutSeconds: 0.03 }, async body => {
    const op = (body.RunCommand as { operation: Record<string, unknown> }).operation
    if (op.operation === "exec_open") return result({ result: "exec_opened", exec_id: "expires" })
    if (op.operation === "exec_stdin_close") return result({ result: "exec_input", next_offset: 0 })
    reads++
    throw new CommandFailure("kernel_connection_lost", "connection remains down")
  }, { stdin: undefined, stdout: sink([]), stderr: sink([]) }))
  assert.equal(reads, 1)
  assert.ok(Date.now() - began < 1000, "the retry cadence does not extend the original deadline")
})

test("exec stops output recovery at its last confirmed reader lease", async t => {
  t.mock.timers.enable({ apis: ["setTimeout", "Date"] })
  let reads = 0
  await assert.rejects(runExec({ ...options, timeoutSeconds: 600 }, async body => {
    const op = (body.RunCommand as { operation: Record<string, unknown> }).operation
    if (op.operation === "exec_open") return result({ result: "exec_opened", exec_id: "lease" })
    if (op.operation === "exec_stdin_close") return result({ result: "exec_input", next_offset: 0 })
    reads++
    t.mock.timers.tick(300_001)
    throw new CommandFailure("kernel_connection_lost", "last response was beyond the lease")
  }, { stdin: undefined, stdout: sink([]), stderr: sink([]) }))
  assert.equal(reads, 1, "a long command timeout does not extend the last confirmed lease")
})

test("exec bounds a stalled reconnect read by the original deadline", async () => {
  let reads = 0
  const began = Date.now()
  await assert.rejects(runExec({ ...options, timeoutSeconds: 2.05 }, async body => {
    const op = (body.RunCommand as { operation: Record<string, unknown> }).operation
    if (op.operation === "exec_open") return result({ result: "exec_opened", exec_id: "stalled" })
    if (op.operation === "exec_stdin_close") return result({ result: "exec_input", next_offset: 0 })
    reads++
    if (reads === 1) throw new CommandFailure("kernel_connection_lost", "brief loss")
    return new Promise<never>(() => {})
  }, { stdin: undefined, stdout: sink([]), stderr: sink([]) }))
  assert.equal(reads, 2)
  assert.ok(Date.now() - began < 3000, "a reconnect never gets a fresh full timeout")
})

test("exec can interrupt a pending output reconnect without replaying admission", async () => {
  const signals = new EventEmitter() as NodeJS.Process
  const ops: string[] = []
  const running = runExec({ ...options, timeoutSeconds: 10 }, async body => {
    const op = (body.RunCommand as { operation: Record<string, unknown> }).operation
    ops.push(String(op.operation))
    if (op.operation === "exec_open") return result({ result: "exec_opened", exec_id: "interrupted-recovery" })
    if (op.operation === "exec_read") {
      setImmediate(() => signals.emit("SIGINT"))
      throw new CommandFailure("kernel_connection_lost", "brief loss")
    }
    return result({ result: op.operation === "exec_cancel" ? "exec_cancelled" : "exec_closed" })
  }, { stdin: undefined, stdout: sink([]), stderr: sink([]), signals })
  assert.equal(await running, 130)
  assert.equal(ops.filter(op => op === "exec_open").length, 1)
  assert.ok(ops.includes("exec_cancel") && ops.includes("exec_close"))
  assert.equal(signals.listenerCount("SIGINT"), 0)
})
