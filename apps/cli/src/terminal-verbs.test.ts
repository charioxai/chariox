import assert from "node:assert/strict"
import test from "node:test"
import { CommandFailure } from "./agent-command-table.js"
import { runTerminalVerb, waitMsOf, type TerminalFlags } from "./terminal-verbs.js"

const summary = (offset = 0) => ({ terminal_id: "t1", session_id: "s1", title: "ops", output_offset: offset })
const flags = (extra: Partial<TerminalFlags> = {}): TerminalFlags => ({ noEnter: false, session: "s1", terminal: "t1", ...extra })

function recorder(answers: Record<string, unknown>[]) {
  const sent: any[] = []
  return { sent, send: async (body: Record<string, unknown>) => { sent.push(body); return answers.shift()! } }
}

// Real control bytes, spelled out so the fixtures cannot be misread as printable caret notation.
const ESC = String.fromCharCode(27)
const BEL = String.fromCharCode(7)

test("send takes the current offset first, then types the text with Enter as one input", async () => {
  const { sent, send } = recorder([
    { RoomTerminal: { result: { result: "snapshot", snapshot: { terminal: summary(40), output_offset: 40, screen_text: "" } } } },
    { RoomTerminal: { result: { result: "input", input: { accepted: true, driver: { state: "busy" } } } } },
  ])
  const result = await runTerminalVerb("send", ["make test"], flags(), send) as any
  assert.equal(result.after_offset, 40)
  assert.equal(sent[0].RoomTerminal.operation.operation, "snapshot")
  assert.equal(sent[1].RoomTerminal.operation.operation, "input")
  assert.equal(Buffer.from(sent[1].RoomTerminal.operation.data_base64, "base64").toString(), "make test\r")
})

test("send says why the terminal refused instead of printing success", async () => {
  const { send } = recorder([
    { RoomTerminal: { result: { result: "snapshot", snapshot: { terminal: summary(1), output_offset: 1, screen_text: "" } } } },
    { RoomTerminal: { result: { result: "input", input: { accepted: false, driver: {}, refusal: "busy (driven by agent-a)" } } } },
  ])
  await assert.rejects(runTerminalVerb("send", ["ls"], flags(), send), (error: unknown) => error instanceof CommandFailure && error.code === "terminal_input_refused" && /driven by agent-a/.test(error.message))
})

// p8: typing then reading is two round trips, so a terminal echo missed the latency gate.
test("send with --wait-ms and --since puts the input and the read in flight together, with no snapshot", async () => {
  const sent: any[] = []
  let release!: () => void
  const gate = new Promise<void>(resolve => { release = resolve })
  const output = Buffer.from(`${ESC}[32mfiles${ESC}[0m\r\n`).toString("base64")
  const send = async (body: Record<string, unknown>) => {
    sent.push(body)
    await gate
    return (body as any).RoomTerminal.operation.operation === "input"
      ? { RoomTerminal: { result: { result: "input", input: { accepted: true, driver: { state: "busy" } } } } }
      : { RoomTerminal: { result: { result: "output", output: { terminal: summary(), data_base64: output, next_offset: 52, gap: false } } } }
  }
  const run = runTerminalVerb("send", ["ls"], flags({ since: "40", waitMs: "5000" }), send) as Promise<any>
  await new Promise(resolve => setImmediate(resolve))
  assert.deepEqual(sent.map(body => body.RoomTerminal.operation.operation), ["input", "read_output"], "both requests left before any answer")
  assert.deepEqual(sent[1].RoomTerminal.operation, { operation: "read_output", terminal_id: "t1", after_offset: 40, wait_ms: 5000 })
  release()
  const result = await run
  assert.deepEqual([result.accepted, result.text, result.next_offset, result.after_offset], [true, "files\r\n", 52, 40])
})

test("send with --wait-ms but no --since takes the offset first, then types and reads together", async () => {
  const { sent, send } = recorder([
    { RoomTerminal: { result: { result: "snapshot", snapshot: { terminal: summary(7), output_offset: 7, screen_text: "" } } } },
    { RoomTerminal: { result: { result: "input", input: { accepted: true, driver: {} } } } },
    { RoomTerminal: { result: { result: "output", output: { terminal: summary(), data_base64: "", next_offset: 7, gap: false } } } },
  ])
  const result = await runTerminalVerb("send", ["ls"], flags({ waitMs: "100" }), send) as any
  assert.deepEqual(sent.map(body => body.RoomTerminal.operation.operation), ["snapshot", "input", "read_output"])
  assert.equal(sent[2].RoomTerminal.operation.after_offset, 7)
  assert.equal(result.after_offset, 7)
})

test("send with --wait-ms still says why the terminal refused, without waiting for the read", async () => {
  const sent: any[] = []
  const send = async (body: Record<string, unknown>) => {
    sent.push(body)
    if ((body as any).RoomTerminal.operation.operation === "read_output") return new Promise<never>(() => {})
    return { RoomTerminal: { result: { result: "input", input: { accepted: false, driver: {}, refusal: "busy (driven by agent-a)" } } } }
  }
  await assert.rejects(runTerminalVerb("send", ["ls"], flags({ since: "1", waitMs: "60000" }), send), (error: unknown) => error instanceof CommandFailure && error.code === "terminal_input_refused")
})

test("read prints plain text without colour or title escapes and the next offset", async () => {
  const output = Buffer.from(`${ESC}]0;ops${BEL}${ESC}[32mok${ESC}[0m done\r\n`).toString("base64")
  const { sent, send } = recorder([{ RoomTerminal: { result: { result: "output", output: { terminal: summary(), data_base64: output, next_offset: 31, gap: false } } } }])
  const result = await runTerminalVerb("read", [], flags({ since: "12", waitMs: "2000" }), send) as any
  assert.equal(result.text, "ok done\r\n")
  assert.equal(result.next_offset, 31)
  assert.deepEqual(sent[0].RoomTerminal.operation, { operation: "read_output", terminal_id: "t1", after_offset: 12, wait_ms: 2000 })
})

test("open runs the program after -- with no shell, and a login shell without it", async () => {
  const opened = { RoomTerminal: { result: { result: "opened", terminal: summary() } } }
  const { sent, send } = recorder([opened, opened])
  await runTerminalVerb("open", ["tail", "-f", "log"], flags({ title: "logs" }), send)
  await runTerminalVerb("open", [], flags(), send)
  assert.deepEqual(sent[0].RoomTerminal.operation, { operation: "open", cols: 120, rows: 30, title: "logs", command: ["tail", "-f", "log"] })
  assert.equal("command" in sent[1].RoomTerminal.operation, false)
})

test("wait asks again in four-minute pieces until it fires or the time is up", async () => {
  const waited = (fired: unknown, offset: number) => ({ RoomTerminal: { result: { result: "waited", waited: { fired, terminal: summary(offset), screen_text: "$ ", output_offset: offset } } } })
  const { sent, send } = recorder([waited(null, 10), waited({ kind: "finished", exit_code: 0 }, 20)])
  const result = await runTerminalVerb("wait", [], flags({ for: "finished", timeout: "600" }), send) as any
  assert.deepEqual(result.fired, { kind: "finished", exit_code: 0 })
  assert.equal(sent.length, 2)
  assert.equal(sent[0].RoomTerminal.operation.timeout_ms, 240_000)
  assert.equal(sent[1].RoomTerminal.operation.after_offset, 10, "the next piece continues from where the last one looked")
  assert.deepEqual(sent[0].RoomTerminal.operation.watches, [{ kind: "finished" }])
  assert.equal(waitMsOf("wait", flags({ timeout: "5" })), 5000)
  await assert.rejects(runTerminalVerb("wait", [], flags({ for: "output_contains=" }), send), CommandFailure)
})

test("a command that needs a session or terminal names the missing flag", async () => {
  const { send } = recorder([])
  await assert.rejects(runTerminalVerb("list", [], { noEnter: false }, send), /--session/)
  await assert.rejects(runTerminalVerb("close", [], { noEnter: false, session: "s1" }, send), /--terminal/)
})

test("a terminal on a worker or slice agent is driven with --host-agent on every request", async () => {
  const { sent, send } = recorder([
    { RoomTerminal: { result: { result: "snapshot", snapshot: { terminal: summary(7), output_offset: 7, screen_text: "" } } } },
    { RoomTerminal: { result: { result: "input", input: { accepted: true, driver: { state: "busy" } } } } },
    { RoomTerminal: { result: { result: "closed" } } },
  ])
  await runTerminalVerb("send", ["ls"], flags({ hostAgent: "worker-agent" }), send)
  await runTerminalVerb("close", [], flags({ hostAgent: "worker-agent" }), send)
  assert.deepEqual(sent.map(request => request.RoomTerminal.host_agent), ["worker-agent", "worker-agent", "worker-agent"])
})

test("an escape sequence cut by the end of a page is held back for the next read, not printed as fragments", async () => {
  const page = (text: string, next: number) => ({ RoomTerminal: { result: { result: "output", output: { terminal: summary(), data_base64: Buffer.from(text).toString("base64"), next_offset: next, gap: false } } } })
  const first = await runTerminalVerb("read", [], flags({ since: "0" }), recorder([page(`ok ${ESC}[3`, 100)]).send) as any
  assert.equal(first.text, "ok ")
  assert.equal(first.next_offset, 97, "the next read starts at the ESC, so the sequence arrives whole")
  const second = await runTerminalVerb("read", [], flags({ since: "97" }), recorder([page(`${ESC}[32mgreen${ESC}[0m`, 110)]).send) as any
  assert.equal(second.text, "green")
  assert.equal(second.next_offset, 110)
  const title = await runTerminalVerb("read", [], flags({ since: "0" }), recorder([page(`a${ESC}]0;ops`, 20)]).send) as any
  assert.deepEqual([title.text, title.next_offset], ["a", 13])
})

test("plain text strips numeric ESC/BEL bytes and preserves printable caret notation", async () => {
  const esc = String.fromCharCode(27), bel = String.fromCharCode(7)
  const page = (text: string, next: number) => ({ RoomTerminal: { result: { result: "output", output: { terminal: summary(), data_base64: Buffer.from(text).toString("base64"), next_offset: next, gap: false } } } })
  const raw = `${esc}]0;private title${bel}${esc}[32mok${esc}[0m ^[[32m`
  const result = await runTerminalVerb("read", [], flags({ since: "0" }), recorder([page(raw, Buffer.byteLength(raw))]).send) as any
  assert.equal(result.text, "ok ^[[32m")
  const partial = `ok${esc}]133;A${esc}`
  const cut = await runTerminalVerb("read", [], flags({ since: "0" }), recorder([page(partial, Buffer.byteLength(partial))]).send) as any
  assert.deepEqual([cut.text, cut.next_offset], ["ok", 2])
  const completed = `${esc}]133;A${esc}\\done`
  const rest = await runTerminalVerb("read", [], flags({ since: "2" }), recorder([page(completed, 2 + Buffer.byteLength(completed))]).send) as any
  assert.equal(rest.text, "done")
})

for (const verb of ["read", "send"] as const) {
  test(`${verb} retains incomplete UTF-8 bytes for the next output page`, async () => {
    const page = (bytes: Buffer, next: number) => ({ RoomTerminal: { result: { result: "output", output: { terminal: summary(), data_base64: bytes.toString("base64"), next_offset: next, gap: false } } } })
    const bytes = Buffer.from("€😀", "utf8")
    for (const split of [1, 2, 4, 5, 6]) {
      const prefix = Buffer.concat([Buffer.from("ok"), bytes.subarray(0, split)])
      const replies = verb === "send" ? [{ RoomTerminal: { result: { result: "input", input: { accepted: true, driver: {} } } } }, page(prefix, prefix.length)] : [page(prefix, prefix.length)]
      const first = await runTerminalVerb(verb, ["echo"], flags({ since: "0", waitMs: "100" }), recorder(replies).send) as any
      assert.equal(first.text.includes("�"), false)
      const rest = await runTerminalVerb("read", [], flags({ since: String(first.next_offset) }), recorder([page(Buffer.concat([Buffer.from("ok"), bytes]).subarray(first.next_offset), 9)]).send) as any
      assert.equal(first.text + rest.text, "ok€😀")
      assert.equal(rest.next_offset, 9)
    }
  })
}
