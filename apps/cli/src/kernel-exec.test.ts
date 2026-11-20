import assert from "node:assert/strict"
import { EventEmitter } from "node:events"
import test from "node:test"
import { Writable } from "node:stream"
import { CommandFailure } from "./agent-command-table.js"
import { runKernelExec } from "./kernel-exec.js"

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
