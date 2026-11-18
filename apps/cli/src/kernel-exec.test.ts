import assert from "node:assert/strict"
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
