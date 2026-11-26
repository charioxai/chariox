import assert from "node:assert/strict"
import { EventEmitter } from "node:events"
import test from "node:test"
import { Writable } from "node:stream"
import { CommandFailure } from "./agent-command-table.js"
import { Readable } from "node:stream"
import { LocalIpcError } from "@chariox/kernel-client/ipc"
import { MAX_INLINE_STDIN_BYTES, lateStdin, ownStdin, readExecInput, runKernelExec } from "./kernel-exec.js"

const ended = (code: number) => ({ state: "ended", outcome: { reason: "program_exited", exit_code: code, signal: null } })
const terminal = (status: unknown, offset: number) => ({ terminal_id: "t1", scope: "kernel", output_offset: offset, status })

function kernel(text: string, code: number) {
  const sent: any[] = []
  const data = Buffer.from(text).toString("base64")
  const send = async (body: any) => {
    sent.push(body)
    // The kernel's own validation: a folder is chosen when a terminal opens, never afterwards.
    if (body.KernelTerminal?.directory !== undefined && body.KernelTerminal.operation.operation !== "open") {
      throw Object.assign(new Error("a folder is chosen when a terminal is opened, not afterwards"), { code: "terminal_refused" })
    }
    const operation = body.KernelTerminal?.operation?.operation
    if (operation === "open") return { RoomTerminal: { result: { result: "opened", terminal: terminal({ state: "running" }, 0) } } }
    if (operation === "wait") return { RoomTerminal: { result: { result: "waited", waited: { fired: { kind: "finished" }, terminal: terminal(ended(code), text.length), screen_text: "", output_offset: text.length } } } }
    if (operation === "read_output") return { RoomTerminal: { result: { result: "output", output: { terminal: terminal(ended(code), text.length), data_base64: data, next_offset: text.length, gap: false } } } }
    if (operation === "close") return { RoomTerminal: { result: { result: "closed" } } }
    throw new Error(`unexpected ${JSON.stringify(body)}`)
  }
  return { sent, send }
}

function sink() {
  let text = ""
  return { stream: new Writable({ write(chunk, _encoding, done) { text += chunk.toString(); done() } }), text: () => text }
}

test("a one-shot command opens a kernel terminal, waits, prints its output, returns its exit code and closes it", async () => {
  const { sent, send } = kernel(" 21:00 up 3 days\r\n", 3)
  const out = sink()
  const code = await runKernelExec({ command: ["uptime"], timeoutSeconds: 30 }, send, out.stream)
  assert.equal(code, 3)
  assert.equal(out.text(), " 21:00 up 3 days\n")
  assert.deepEqual(sent.map(body => Object.keys(body)[0]), ["KernelTerminal", "KernelTerminal", "KernelTerminal", "KernelTerminal"])
  assert.deepEqual(sent.map(body => body.KernelTerminal.operation.operation), ["open", "wait", "read_output", "close"])
  assert.deepEqual(sent[0].KernelTerminal.operation.command, ["uptime"])
})

for (const [signal, exitCode] of [["SIGINT", 130], ["SIGTERM", 143], ["SIGHUP", 129]] as const) {
  test(`kernel exec closes a terminal interrupted by ${signal} while open is pending`, async () => {
    const { sent, send } = kernel("", 0)
    const signals = new EventEmitter() as NodeJS.Process
    const code = await runKernelExec({ command: ["sleep", "60"], timeoutSeconds: 30, signals }, async (body) => {
      const reply = await send(body)
      if (sent.length === 1) {
        signals.emit(signal)
        signals.emit("SIGINT")
      }
      return reply
    }, sink().stream)
    assert.equal(code, exitCode)
    assert.deepEqual(sent.map(body => body.KernelTerminal.operation.operation), ["open", "close"])
    assert.equal(sent[1].KernelTerminal.operation.terminal_id, "t1")
    for (const name of ["SIGINT", "SIGTERM", "SIGHUP"]) assert.equal(signals.listenerCount(name), 0)
  })
}

test("kernel exec closes a known terminal without waiting for the interrupted wait request", async () => {
  const { sent, send } = kernel("", 0)
  const signals = new EventEmitter() as NodeJS.Process
  const code = await runKernelExec({ command: ["sleep", "60"], timeoutSeconds: 30, signals }, async body => {
    const reply = await send(body)
    if (sent.at(-1).KernelTerminal.operation.operation === "wait") {
      signals.emit("SIGTERM")
      return new Promise<never>(() => {})
    }
    return reply
  }, sink().stream)
  assert.equal(code, 143)
  assert.deepEqual(sent.map(body => body.KernelTerminal.operation.operation), ["open", "wait", "close"])
  assert.equal(sent[2].KernelTerminal.operation.terminal_id, "t1")
})

test("kernel exec detaches signal listeners when open fails", async () => {
  const signals = new EventEmitter() as NodeJS.Process
  let requests = 0
  await assert.rejects(runKernelExec({ command: ["sleep", "60"], timeoutSeconds: 30, signals }, async () => {
    requests++
    throw new Error("open refused")
  }, sink().stream), /open refused/)
  assert.equal(requests, 1)
  for (const name of ["SIGINT", "SIGTERM", "SIGHUP"]) assert.equal(signals.listenerCount(name), 0)
})

test("a command killed by a signal is not reported as success", async () => {
  const { send } = kernel("", 0)
  const killed = async (body: any) => {
    const reply: any = await send(body)
    const waited = reply.RoomTerminal?.result?.waited
    if (waited) waited.terminal.status = { state: "ended", outcome: { reason: "memory_limit", limit_bytes: 1 } }
    return reply
  }
  await assert.rejects(runKernelExec({ command: ["x"], timeoutSeconds: 30 }, killed, sink().stream), (error: unknown) => error instanceof CommandFailure && error.code === "exec_failed")
})

for (const [signal, expected] of [["2", 130], ["9", 137], ["15", 143]] as const) {
  test(`a command terminated by signal ${signal} returns ${expected}`, async () => {
    const { sent, send } = kernel("", 0)
    const signalled = async (body: any) => {
      const reply: any = await send(body)
      const waited = reply.RoomTerminal?.result?.waited
      if (waited) waited.terminal.status = { state: "ended", outcome: { reason: "program_exited", exit_code: null, signal } }
      return reply
    }
    assert.equal(await runKernelExec({ command: ["sh"], timeoutSeconds: 30 }, signalled, sink().stream), expected)
    assert.equal(sent.at(-1).KernelTerminal.operation.operation, "close")
  })
}

test("--directory goes with open only; wait, read and close carry none, and the terminal is closed", async () => {
  const { sent, send } = kernel("/tmp\r\n", 0)
  const out = sink()
  assert.equal(await runKernelExec({ command: ["pwd"], directory: "/tmp", timeoutSeconds: 30 }, send, out.stream), 0)
  assert.equal(out.text(), "/tmp\n")
  assert.deepEqual(sent.map(body => body.KernelTerminal.directory), ["/tmp", undefined, undefined, undefined])
  assert.equal(sent.at(-1).KernelTerminal.operation.operation, "close")
})

test("a failed command run never leaves its terminal behind: it is closed, or named with the way to close it", async () => {
  const { sent, send } = kernel("x", 0)
  const failing = async (body: any) => {
    if (body.KernelTerminal.operation.operation === "read_output") throw new Error("link lost")
    return send(body)
  }
  await assert.rejects(runKernelExec({ command: ["x"], timeoutSeconds: 30 }, failing, sink().stream), /link lost/)
  assert.equal(sent.at(-1).KernelTerminal.operation.operation, "close", "closed after the failure")
  const stuck = async (body: any) => {
    if (body.KernelTerminal.operation.operation === "close") throw new Error("kernel gone")
    return send(body)
  }
  await assert.rejects(runKernelExec({ command: ["x"], timeoutSeconds: 30 }, stuck, sink().stream), (error: unknown) => error instanceof CommandFailure && error.code === "exec_terminal_left_open" && /terminal close --terminal t1/.test(JSON.stringify(error)))
})

test("a character split between two output pages is whole in the output, and a split CRLF is one newline", async () => {
  const bytes = Buffer.from("a\u00e9b\r\nc", "utf8")
  const cut = 2 // inside the two-byte e-acute
  const pages = [bytes.subarray(0, cut), bytes.subarray(cut, 5), bytes.subarray(5)]
  const edges = [cut, 5, bytes.length]
  let page = 0
  const send = async (body: any) => {
    const operation = body.KernelTerminal.operation.operation
    if (operation === "open") return { RoomTerminal: { result: { result: "opened", terminal: terminal({ state: "running" }, 0) } } }
    if (operation === "wait") return { RoomTerminal: { result: { result: "waited", waited: { fired: { kind: "finished" }, terminal: terminal(ended(0), bytes.length), screen_text: "", output_offset: bytes.length } } } }
    if (operation === "read_output") {
      const index = page++
      return { RoomTerminal: { result: { result: "output", output: { terminal: terminal(ended(0), bytes.length), data_base64: pages[index]!.toString("base64"), next_offset: edges[index], gap: false } } } }
    }
    return { RoomTerminal: { result: { result: "closed" } } }
  }
  const out = sink()
  await runKernelExec({ command: ["x"], timeoutSeconds: 30 }, send, out.stream)
  assert.equal(out.text(), "a\u00e9b\nc")
})

test("a program's stdin goes to the kernel with the open request", async () => {
  const { sent, send } = kernel("ok\r\n", 0)
  const stdin = { kind: "inline", data_base64: Buffer.from("echo hi\n").toString("base64") } as const
  await runKernelExec({ command: ["bash", "-s"], timeoutSeconds: 30, stdin }, send, sink().stream)
  assert.deepEqual(sent[0].KernelTerminal.operation.stdin, stdin)
})

const decoded = (input: Awaited<ReturnType<typeof readExecInput>>) => input && input.kind === "inline" ? Buffer.from(input.data_base64, "base64") : input
const idle = () => new Readable({ read() {} })

test("input piped to exec that has data at once is read to its end and sent whole", async () => {
  const script = Buffer.alloc(MAX_INLINE_STDIN_BYTES, "a")
  const read = await readExecInput(Readable.from([script.subarray(0, 100), script.subarray(100)]), { timeoutSeconds: 5 })
  assert.deepEqual(decoded(read), script)
})

test("an input that has already ended, or a terminal, sends none", async () => {
  assert.equal(await readExecInput(Readable.from([]), { timeoutSeconds: 5 }), undefined)
  assert.equal(await readExecInput(Object.assign(Readable.from([Buffer.from("typed")]), { isTTY: true }), { timeoutSeconds: 5 }), undefined)
})

test("an open stdin that stays idle is not waited on: no input, and it is closed so it cannot hold the process open", async () => {
  const never = idle()
  const started = Date.now()
  assert.equal(await readExecInput(never, { timeoutSeconds: 30, probeMs: 30 }), undefined)
  assert.ok(Date.now() - started < 2000, "the run must not wait for an idle stdin")
  assert.equal(never.destroyed, true)
})

test("a producer slower than the probe is not waited for unless --stdin asks for it, and the late data is reported", async () => {
  const slow = () => { const stream = idle(); setTimeout(() => { stream.push(Buffer.from("late\n")); stream.push(null) }, 120); return stream }
  const late = lateStdin()
  assert.equal(await readExecInput(slow(), { timeoutSeconds: 5, probeMs: 20, late }), undefined)
  assert.equal(late.arrived, false, "nothing has arrived yet")
  await new Promise(done => setTimeout(done, 250))
  assert.equal(late.arrived, true, "data that arrived while the command ran is reported")
  assert.equal(decoded(await readExecInput(slow(), { timeoutSeconds: 5, wait: true }))?.toString(), "late\n")
})

test("an idle stdin that stays open reports nothing, and stopping the watch closes it", async () => {
  const never = idle()
  const late = lateStdin()
  assert.equal(await readExecInput(never, { timeoutSeconds: 5, probeMs: 20, late }), undefined)
  await new Promise(done => setTimeout(done, 80))
  assert.equal(late.arrived, false)
  late.stop()
  assert.equal(never.destroyed, true)
})

test("a stdin that ends without data after the probe reports nothing", async () => {
  const quiet = idle()
  const late = lateStdin()
  await readExecInput(quiet, { timeoutSeconds: 5, probeMs: 20, late })
  quiet.push(null)
  await new Promise(done => setTimeout(done, 50))
  assert.equal(late.arrived, false)
  late.stop()
})

test("an unreadable stdin that fails before its first chunk is refused as exec_stdin_unreadable, not left unhandled", async () => {
  const broken = new Readable({ read() { this.destroy(Object.assign(new Error("EISDIR: illegal operation on a directory, read"), { code: "EISDIR" })) } })
  await assert.rejects(readExecInput(broken, { timeoutSeconds: 5, probeMs: 500, late: lateStdin() }),
    (error: unknown) => error instanceof CommandFailure && error.code === "exec_stdin_unreadable" && /EISDIR/.test(error.message))
  assert.equal(broken.listenerCount("error"), 0, "the probe removes its listeners")
  assert.equal(broken.listenerCount("readable"), 0)
})

test("an error after an idle probe ends the late watch quietly and removes its listeners", async () => {
  const input = idle()
  const late = lateStdin()
  assert.equal(await readExecInput(input, { timeoutSeconds: 5, probeMs: 20, late }), undefined)
  assert.equal(input.listenerCount("data"), 1)
  input.destroy(new Error("read EIO"))
  await new Promise(done => setTimeout(done, 20))
  assert.equal(late.arrived, false, "an input error is not data")
  assert.equal(input.listenerCount("error"), 0)
  assert.equal(input.listenerCount("data"), 0)
  late.stop()
})

test("a stream that throws the moment it is listened to (bun, a directory on stdin) is refused as exec_stdin_unreadable", async () => {
  const refusing = Object.assign(idle(), { on() { throw new Error("EISDIR: illegal operation on a directory, fstat") } })
  await assert.rejects(readExecInput(refusing, { timeoutSeconds: 5, probeMs: 500, late: lateStdin() }),
    (error: unknown) => error instanceof CommandFailure && error.code === "exec_stdin_unreadable" && /EISDIR/.test(error.message))
})

test("a runtime that cannot open stdin at all (a directory redirected into it) is refused as exec_stdin_unreadable", async () => {
  const real = Object.getOwnPropertyDescriptor(process, "stdin")!
  Object.defineProperty(process, "stdin", { configurable: true, get() { throw Object.assign(new Error("EISDIR: illegal operation on a directory, fstat"), { code: "EISDIR" }) } })
  try {
    const input = ownStdin()
    await assert.rejects(readExecInput(input, { timeoutSeconds: 5, probeMs: 500 }), (error: unknown) => error instanceof CommandFailure && error.code === "exec_stdin_unreadable" && /EISDIR/.test(error.message))
    assert.deepEqual(await readExecInput(ownStdin(), { file: "/home/u/s.sh", timeoutSeconds: 5 }), { kind: "file", path: "/home/u/s.sh" }, "--stdin-file never reads it")
  } finally { Object.defineProperty(process, "stdin", real) }
})

test("a regular file on stdin is read whole with no probe", async () => {
  const late = idle()
  setTimeout(() => { late.push(Buffer.from("from a file\n")); late.push(null) }, 80)
  assert.equal(decoded(await readExecInput(late, { timeoutSeconds: 5, regularFile: true, probeMs: 1 }))?.toString(), "from a file\n")
})

test("a file on the kernel is passed by reference and the local input is not read", async () => {
  const never = idle()
  assert.deepEqual(await readExecInput(never, { file: "/home/u/script.sh", timeoutSeconds: 5 }), { kind: "file", path: "/home/u/script.sh" })
  never.destroy()
})

test("--stdin on a terminal reads it to its end", async () => {
  const typed = Object.assign(Readable.from([Buffer.from("typed")]), { isTTY: true })
  assert.equal(decoded(await readExecInput(typed, { timeoutSeconds: 5, wait: true }))?.toString(), "typed")
})

test("input over 64 KiB is refused naming the limit and the file way", async () => {
  await assert.rejects(readExecInput(Readable.from([Buffer.alloc(MAX_INLINE_STDIN_BYTES + 1)]), { timeoutSeconds: 5 }),
    (error: unknown) => error instanceof CommandFailure && error.code === "exec_stdin_too_large" && /64 KiB/.test(error.message) && /files put/.test(error.message) && /--stdin-file/.test(error.message))
})

test("--stdin on input that never ends is refused after the timeout and says how to end it", async () => {
  await assert.rejects(readExecInput(idle(), { timeoutSeconds: 0.05, wait: true }),
    (error: unknown) => error instanceof CommandFailure && error.code === "exec_stdin_not_ended" && /<\/dev\/null/.test(error.message))
})

test("a terminal the kernel already removed after the last read counts as closed, not as an error", async () => {
  const { send } = kernel("done\r\n", 0)
  const removed = async (body: any) => {
    const operation = body.KernelTerminal.operation.operation
    if (operation === "close") throw Object.assign(new Error("this kernel has no terminal `t1`"), { code: "terminal_refused" })
    if (operation === "list") return { RoomTerminal: { result: { result: "listed", terminals: [] } } }
    return send(body)
  }
  const out = sink()
  assert.equal(await runKernelExec({ command: ["x"], timeoutSeconds: 30 }, removed, out.stream), 0)
  assert.equal(out.text(), "done\n")
})

test("a close that fails while the terminal is still listed is named with the way to close it", async () => {
  const { send } = kernel("done\r\n", 0)
  const stuck = async (body: any) => {
    const operation = body.KernelTerminal.operation.operation
    if (operation === "close") throw new Error("kernel busy")
    if (operation === "list") return { RoomTerminal: { result: { result: "listed", terminals: [terminal({ state: "running" }, 0)] } } }
    return send(body)
  }
  await assert.rejects(runKernelExec({ command: ["x"], timeoutSeconds: 30 }, stuck, sink().stream), (error: unknown) => error instanceof CommandFailure && error.code === "exec_terminal_left_open")
})

// A relay blip: the gateway answers a request in flight with one of these (retryable) codes while the command keeps running on the kernel.
const blip = (code: string) => Object.assign(new CommandFailure(code, `link ${code}`))
const operationOf = (body: any) => body.KernelTerminal.operation.operation
const lines = () => { const seen: string[] = []; return { seen, notice: (line: string) => { seen.push(line) } } }

for (const code of ["kernel_connection_lost", "kernel_request_timeout", "kernel_offline", "kernel_unreachable"]) {
  test(`a wait that fails with ${code} is retried on the same terminal and the command's real exit code comes back`, async () => {
    const { sent, send } = kernel("built\r\n", 7)
    let dropped = 0
    const flaky = async (body: any) => {
      if (operationOf(body) === "wait" && dropped++ < 3) throw code === "kernel_offline" ? new LocalIpcError("kernel request", "offline", code, false) : blip(code)
      return send(body)
    }
    const out = sink(), said = lines()
    assert.equal(await runKernelExec({ command: ["make"], timeoutSeconds: 30, kernel: "nelpx", retryStepMs: 1, notice: said.notice }, flaky, out.stream), 7)
    assert.equal(out.text(), "built\n")
    const operations = sent.map(operationOf)
    assert.equal(operations.filter(name => name === "open").length, 1, "one terminal, never a second command")
    assert.equal(operations.indexOf("close"), operations.length - 1, "the terminal is closed only after the output was read, never during the blip")
    assert.equal(said.seen.length, 1, "one plain line for the whole outage")
    assert.match(said.seen[0]!, /link to kernel nelpx lost, waiting for it; the command keeps running as terminal t1/)
  })
}

test("a read_output that fails during a blip is retried from the same offset", async () => {
  const { send } = kernel("abc\r\n", 0)
  const reads: any[] = []
  let dropped = false
  const flaky = async (body: any) => {
    if (operationOf(body) === "read_output") reads.push(body)
    if (operationOf(body) === "read_output" && !dropped) { dropped = true; throw blip("kernel_connection_lost") }
    return send(body)
  }
  const out = sink()
  assert.equal(await runKernelExec({ command: ["x"], timeoutSeconds: 30, retryStepMs: 1, notice: () => {} }, flaky, out.stream), 0)
  assert.equal(out.text(), "abc\n")
  assert.deepEqual(reads.map(body => body.KernelTerminal.operation.after_offset), [0, 0], "the failed read and its retry ask for the same offset")
})

test("a link that is down past --timeout is named, the command is not stopped, and its terminal is findable", async () => {
  const { sent, send } = kernel("x", 0)
  const down = async (body: any) => {
    if (operationOf(body) === "wait") throw blip("kernel_offline")
    return send(body)
  }
  const said = lines()
  await assert.rejects(runKernelExec({ command: ["make"], timeoutSeconds: 1, kernel: "nelpx", retryStepMs: 20, notice: said.notice }, down, sink().stream), (error: unknown) => {
    if (!(error instanceof CommandFailure) || error.code !== "exec_link_lost") return false
    const text = JSON.stringify(error.message) + JSON.stringify(error.details)
    return /not stopped/.test(text) && /terminal t1/.test(text) && /nelpx/.test(text) && /terminal wait --kernel nelpx --terminal t1/.test(text) && /terminal read --kernel nelpx --terminal t1/.test(text) && /terminal close --kernel nelpx --terminal t1/.test(text) && /60 s/.test(text)
  })
  assert.equal(sent.some(body => operationOf(body) === "close"), false, "no close: it would end the command it cannot report on")
  assert.equal(said.seen.length, 1)
})

test("a failure that is not a link drop still closes the terminal as before", async () => {
  const { sent, send } = kernel("x", 0)
  const refused = async (body: any) => {
    if (operationOf(body) === "wait") throw new CommandFailure("terminal_refused", "no")
    return send(body)
  }
  await assert.rejects(runKernelExec({ command: ["x"], timeoutSeconds: 30, retryStepMs: 1, notice: () => {} }, refused, sink().stream), /no/)
  assert.equal(sent.at(-1).KernelTerminal.operation.operation, "close")
})

test("Ctrl-C while waiting for a dropped link closes the terminal and returns 130", async () => {
  const { sent, send } = kernel("x", 0)
  const signals = new EventEmitter() as NodeJS.Process
  const down = async (body: any) => {
    if (operationOf(body) === "wait") { setTimeout(() => signals.emit("SIGINT"), 30); throw blip("kernel_connection_lost") }
    return send(body)
  }
  const code = await runKernelExec({ command: ["x"], timeoutSeconds: 30, signals, retryStepMs: 1000, notice: () => {} }, down, sink().stream)
  assert.equal(code, 130)
  assert.equal(sent.at(-1).KernelTerminal.operation.operation, "close")
  for (const name of ["SIGINT", "SIGTERM", "SIGHUP"]) assert.equal(signals.listenerCount(name), 0)
})

test("a drop while the terminal is being opened is not retried: its id is unknown, nothing is waited on", async () => {
  let requests = 0
  await assert.rejects(runKernelExec({ command: ["x"], timeoutSeconds: 30, retryStepMs: 1, notice: () => {} }, async () => { requests++; throw blip("kernel_connection_lost") }, sink().stream), (error: unknown) => error instanceof CommandFailure && error.code === "kernel_connection_lost")
  assert.equal(requests, 1)
})

test("a terminal that cannot be closed is named with its kernel, so the way to close it works from anywhere", async () => {
  const { send } = kernel("x", 0)
  const signals = new EventEmitter() as NodeJS.Process
  const down = async (body: any) => {
    if (operationOf(body) === "wait") { setTimeout(() => signals.emit("SIGINT"), 20); throw blip("kernel_connection_lost") }
    if (operationOf(body) === "close" || operationOf(body) === "list") throw blip("kernel_offline")
    return send(body)
  }
  const written: string[] = []
  const realWrite = process.stderr.write.bind(process.stderr)
  process.stderr.write = ((chunk: string | Uint8Array) => { written.push(String(chunk)); return true }) as typeof process.stderr.write
  try {
    assert.equal(await runKernelExec({ command: ["x"], timeoutSeconds: 30, signals, kernel: "nelpx", retryStepMs: 1000, notice: () => {} }, down, sink().stream), 130)
  } finally { process.stderr.write = realWrite }
  assert.match(written.join(""), /chariox terminal close --kernel nelpx --terminal t1/)
})

test("a retry after a drop waits only for what is left of --timeout, not a fresh full wait", async () => {
  const { send } = kernel("ok\r\n", 0)
  const waits: number[] = []
  let dropped = false
  const flaky = async (body: any) => {
    if (operationOf(body) === "wait") {
      waits.push(body.KernelTerminal.operation.timeout_ms)
      if (!dropped) { dropped = true; throw blip("kernel_connection_lost") }
    }
    return send(body)
  }
  assert.equal(await runKernelExec({ command: ["x"], timeoutSeconds: 3, retryStepMs: 1500, notice: () => {} }, flaky, sink().stream), 0)
  assert.equal(waits.length, 2)
  assert.ok(waits[0]! <= 3000 && waits[0]! > 2500, `first wait ${waits[0]}`)
  assert.ok(waits[1]! <= 1600, `the retry asks for what is left (about 1500 ms), not ${waits[1]}`)
})

test("a retry that never answers ends at --timeout with the command left running", async () => {
  const { sent, send } = kernel("x", 0)
  let waits = 0
  const stalled = async (body: any) => {
    if (operationOf(body) === "wait") {
      if (waits++ === 0) throw blip("kernel_connection_lost")
      return new Promise<never>(() => {})
    }
    return send(body)
  }
  const started = Date.now()
  await assert.rejects(runKernelExec({ command: ["x"], timeoutSeconds: 1, retryStepMs: 20, kernel: "nelpx", notice: () => {} }, stalled, sink().stream), (error: unknown) => error instanceof CommandFailure && error.code === "exec_link_lost")
  assert.ok(Date.now() - started < 2500, `ended after ${Date.now() - started} ms`)
  assert.equal(sent.some(body => operationOf(body) === "close"), false)
})

test("the pause between tries never runs past --timeout", async () => {
  const { send } = kernel("x", 0)
  const down = async (body: any) => {
    if (operationOf(body) === "wait") throw blip("kernel_offline")
    return send(body)
  }
  const started = Date.now()
  await assert.rejects(runKernelExec({ command: ["x"], timeoutSeconds: 1, retryStepMs: 5000, notice: () => {} }, down, sink().stream), (error: unknown) => error instanceof CommandFailure && error.code === "exec_link_lost")
  assert.ok(Date.now() - started < 2000, `ended after ${Date.now() - started} ms`)
})

// The kernel removes a finished one-command terminal as it answers the read of its last byte; a drop that loses
// that answer leaves a retry with "no such terminal". The exit code is known from the wait; the lost page is said plainly.
function lostLastPage(text: string, code: number, firstPage: number) {
  const { send } = kernel(text, code)
  const bytes = Buffer.from(text)
  let lastReads = 0
  const gone = () => Object.assign(new Error("this kernel has no terminal `t1`"), { code: "terminal_refused" })
  return async (body: any) => {
    const operation = operationOf(body)
    if (operation === "read_output") {
      const after = body.KernelTerminal.operation.after_offset
      if (after === 0 && firstPage < bytes.length) {
        return { RoomTerminal: { result: { result: "output", output: { terminal: terminal(ended(code), bytes.length), data_base64: bytes.subarray(0, firstPage).toString("base64"), next_offset: firstPage, gap: false } } } }
      }
      if (lastReads++ === 0) throw blip("kernel_connection_lost") // the kernel answered; the reply was lost with the link
      throw gone()
    }
    if (operation === "close") throw gone()
    if (operation === "list") return { RoomTerminal: { result: { result: "listed", terminals: [] } } }
    return send(body)
  }
}

test("B-1: the last output page lost with the link: the command's exit code comes back and one plain line says so", async () => {
  const said = lines(), out = sink()
  const code = await runKernelExec({ command: ["x"], timeoutSeconds: 30, retryStepMs: 1, notice: said.notice }, lostLastPage("abc\r\n", 7, 99), out.stream)
  assert.equal(code, 7)
  assert.equal(said.seen.filter(line => /last page of the output/.test(line)).length, 1, JSON.stringify(said.seen))
})

test("B-2: the plain line says how much of the output was delivered", async () => {
  const said = lines(), out = sink()
  const code = await runKernelExec({ command: ["x"], timeoutSeconds: 30, retryStepMs: 1, notice: said.notice }, lostLastPage("abcde123", 7, 5), out.stream)
  assert.equal(code, 7)
  assert.equal(out.text(), "abcde")
  assert.match(said.seen.find(line => /last page of the output/.test(line)) ?? "", /5 of 8 bytes/)
})

test("a retried read that fails for another reason is still an error, not a lost page", async () => {
  const { send } = kernel("abc", 0)
  let reads = 0
  const refused = async (body: any) => {
    if (operationOf(body) === "read_output") { if (reads++ === 0) throw blip("kernel_connection_lost"); throw new CommandFailure("terminal_refused", "busy") }
    if (operationOf(body) === "list") return { RoomTerminal: { result: { result: "listed", terminals: [terminal({ state: "ended" }, 3)] } } }
    return send(body)
  }
  await assert.rejects(runKernelExec({ command: ["x"], timeoutSeconds: 30, retryStepMs: 1, notice: () => {} }, refused, sink().stream), /busy/)
})
