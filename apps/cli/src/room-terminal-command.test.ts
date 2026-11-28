import assert from "node:assert/strict"
import test from "node:test"

import { TERMINAL_DETACH_BYTE } from "./room-terminal-attach.js"
import { formatRoomTerminals, parseRoomTerminalCommand, runRoomTerminalCommand } from "./room-terminal-command.js"
import { FakeTerminalIo, FakeTerminalKernel, until } from "./room-terminal-fake-kernel.test-fixture.js"

test("terminal commands parse to one meaning each and refuse the rest", () => {
  assert.deepEqual(parseRoomTerminalCommand([]), { kind: "attach" })
  assert.deepEqual(parseRoomTerminalCommand(["list"]), { kind: "list" })
  assert.deepEqual(parseRoomTerminalCommand(["new"]), { kind: "new" })
  assert.deepEqual(parseRoomTerminalCommand(["new", "build", "box"]), { kind: "new", title: "build box" })
  assert.deepEqual(parseRoomTerminalCommand(["close", "2"]), { kind: "close", selector: "2" })
  assert.deepEqual(parseRoomTerminalCommand(["term-1"]), { kind: "attach", selector: "term-1" })
  assert.match(String(parseRoomTerminalCommand(["close"])), /usage: terminal/)
  assert.match(String(parseRoomTerminalCommand(["a", "b"])), /usage: terminal/)
})

test("the list shows who drives a terminal and where it runs", () => {
  const kernel = new FakeTerminalKernel()
  const base = kernel.summary()
  const text = formatRoomTerminals([
    base,
    { ...base, terminal_id: "term-2", driver: { state: "busy", driven_by: { kind: "agent", agent_id: "agent-1" } }, host_agent: "worker" },
  ])
  assert.match(text, /1\. term-1  shell  at prompt/)
  assert.match(text, /2\. term-2  shell  busy \(driven by agent-1\) on worker/)
  assert.match(formatRoomTerminals([]), /No terminals/)
})

test("attach hands the screen over only for the attach, and tells how to come back", async () => {
  const kernel = new FakeTerminalKernel()
  const io = new FakeTerminalIo()
  const calls: string[] = []
  const done = runRoomTerminalCommand({
    send: kernel.send, target: { sessionId: "room" }, input: io, output: io,
    async withScreen(attach) {
      calls.push("suspend")
      try { return await attach() } finally { calls.push("resume") }
    },
  }, { kind: "attach" })
  await until(() => io.text.includes("PROMPT$ "), "the redrawn screen")
  assert.deepEqual(calls, ["suspend"])
  io.type(Buffer.from([TERMINAL_DETACH_BYTE]))
  assert.match(await done, /Detached from terminal term-1; it keeps running\. `terminal term-1` re-attaches/)
  assert.deepEqual(calls, ["suspend", "resume"])
  assert.ok(!kernel.requests.some(request => request.operation === "open"), "the running terminal is reused")
})

test("a Room without a running terminal gets a new one; list and close never take the screen", async () => {
  const kernel = new FakeTerminalKernel()
  kernel.ended = "ended: exited"
  const io = new FakeTerminalIo()
  let taken = 0
  const deps = { send: kernel.send, target: { sessionId: "room" }, input: io, output: io, withScreen: async <T>(attach: () => Promise<T>) => { taken++; return attach() } }
  assert.match(await runRoomTerminalCommand(deps, { kind: "list" }), /term-1/)
  assert.match(await runRoomTerminalCommand(deps, { kind: "close", selector: "1" }), /Closed terminal term-1/)
  assert.equal(taken, 0)
  assert.match(await runRoomTerminalCommand(deps, { kind: "attach", selector: "term" }), /has ended: exited/)
  await assert.rejects(runRoomTerminalCommand(deps, { kind: "attach", selector: "nope" }), /No terminal "nope"/)
})

test("chariox terminal reads its flags in any order; without --session it is the kernel's own terminal", async () => {
  const { parseTerminalArgs } = await import("./terminal-command.js")
  assert.deepEqual(parseTerminalArgs([]), { rest: [] })
  assert.deepEqual(parseTerminalArgs(["--kernel", "b3", "list"]), { kernel: "b3", rest: ["list"] })
  assert.match(String(parseTerminalArgs(["--kernel", "b3", "--kernel-url", "ws://x"])), /usage: chariox terminal/)
  assert.match(String(parseTerminalArgs(["--session"])), /usage: chariox terminal/)
  assert.deepEqual(parseTerminalArgs(["list", "--session", "room-a"]), { session: "room-a", rest: ["list"] })
  assert.deepEqual(parseTerminalArgs(["--kernel-url", "ws://127.0.0.1:1", "--session", "room-a", "new", "build"]), {
    session: "room-a", kernelUrl: "ws://127.0.0.1:1", rest: ["new", "build"],
  })
})

// Row 549: a machine the list could not ask may hold terminals; the CLI says so instead of "No terminals".
const askedNot = [{ host_agent: "worker-agent", message: "worker kernel `k9` is too old to list its terminals, which may still be running there; update it" }]
function partialKernel() {
  const requests: { operation: string }[] = []
  const send = async (request: unknown) => {
    const operation = (request as { RoomTerminal: { operation: { operation: string } } }).RoomTerminal.operation
    requests.push(operation)
    if (operation.operation === "list") return { RoomTerminal: { result: { result: "listed", terminals: [], unavailable: askedNot } } } as never
    throw new Error(`unexpected ${operation.operation}`)
  }
  return { send: send as never, requests }
}

test("549 terminal list names the machines it could not ask next to the terminals it found", async () => {
  const { send } = partialKernel()
  const io = new FakeTerminalIo()
  const text = await runRoomTerminalCommand({ send, target: { sessionId: "room" }, input: io, output: io }, { kind: "list" })
  assert.match(text, /worker-agent: worker kernel `k9` is too old to list its terminals/)
  assert.doesNotMatch(text, /^No terminals in this Room yet/)
})

test("549 a selector that matches nothing says which machine was not asked", async () => {
  const { send } = partialKernel()
  const io = new FakeTerminalIo()
  await assert.rejects(runRoomTerminalCommand({ send, target: { sessionId: "room" }, input: io, output: io }, { kind: "attach", selector: "term-7" }), /No terminal "term-7".*worker-agent: worker kernel `k9` is too old/s)
})

test("549 terminal with no terminal listed opens no shell by itself while a machine was not asked; terminal new still does", async () => {
  const { send, requests } = partialKernel()
  const io = new FakeTerminalIo()
  const text = await runRoomTerminalCommand({ send, target: { sessionId: "room" }, input: io, output: io }, { kind: "attach" })
  assert.match(text, /could not be listed.*`terminal new`/s)
  assert.ok(!requests.some(request => request.operation === "open"), "no shell is opened on a guess")
})
