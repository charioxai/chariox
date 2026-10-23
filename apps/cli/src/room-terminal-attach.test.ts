import assert from "node:assert/strict"
import test from "node:test"
import { LocalIpcError } from "@chariox/kernel-client/ipc"

import { attachRoomTerminal, TERMINAL_DETACH_BYTE } from "./room-terminal-attach.js"
import { FakeTerminalIo, FakeTerminalKernel, until } from "./room-terminal-fake-kernel.test-fixture.js"

function attach(kernel: FakeTerminalKernel, io: FakeTerminalIo) {
  return attachRoomTerminal({
    send: kernel.send, target: { sessionId: "room" }, terminal: kernel.summary(), input: io, output: io,
  })
}

for (const stage of ["resize", "snapshot"]) {
  for (const failure of ["pending", "rejected"]) {
    test(`Ctrl-] interrupts ${failure} initial ${stage} and restores the terminal`, async () => {
      const kernel = new FakeTerminalKernel(), io = new FakeTerminalIo()
      const abort = new AbortController()
      let entered = false, release: () => void = () => {}
      const blocked = new Promise<void>(resolve => { release = resolve })
      const done = attachRoomTerminal({
        target: { sessionId: "room" }, terminal: kernel.summary(), input: io, output: io, signal: abort.signal,
        send: async request => {
          if ((request as { RoomTerminal: { operation: { operation: string } } }).RoomTerminal.operation.operation === stage) {
            entered = true
            if (failure === "rejected") throw new LocalIpcError("write kernel request", "connection lost", "connection_closed", true)
            await blocked
          }
          return kernel.send(request)
        },
      })
      await until(() => entered, "initial setup request")
      io.type(Buffer.from([TERMINAL_DETACH_BYTE]))
      try {
        const result = await Promise.race([done, new Promise(resolve => setTimeout(() => resolve("stuck"), 150))])
        assert.deepEqual(result, { reason: "detached" })
        assert.deepEqual(io.rawModes, [true, false])
        assert.ok(io.paused)
        const restored = io.text
        release()
        await new Promise(resolve => setTimeout(resolve, 10))
        assert.equal(io.text, restored, "late setup output must not repaint the restored shell")
      } finally { release(); abort.abort(); await done }
    })
  }
}

for (const stage of ["resize", "read_output", "input"]) {
  test(`a real IPC refusal during ${stage} ends the attach without retrying`, async () => {
    const kernel = new FakeTerminalKernel(), io = new FakeTerminalIo(), abort = new AbortController()
    let refused = 0
    const failure = new LocalIpcError("handle kernel response", "this terminal was closed by another client", "terminal_refused", false)
    const done = attachRoomTerminal({
      target: { sessionId: "room" }, terminal: kernel.summary(), input: io, output: io, signal: abort.signal,
      send: async request => {
        if ((request as { RoomTerminal: { operation: { operation: string } } }).RoomTerminal.operation.operation === stage) { refused++; throw failure }
        return kernel.send(request)
      },
    })
    if (stage === "input") { await until(() => io.text.includes("PROMPT$ "), "screen"); io.type("x") }
    try {
      const result = await Promise.race([done, new Promise(resolve => setTimeout(() => resolve("stuck"), 150))])
      assert.deepEqual(result, { reason: "ended", description: failure.message })
      assert.equal(refused, 1)
      assert.deepEqual(io.rawModes, [true, false])
    } finally { abort.abort(); await done }
  })
}

test("attach redraws the screen, streams output both ways and detaches on Ctrl-]", async () => {
  const kernel = new FakeTerminalKernel()
  const io = new FakeTerminalIo()
  const done = attach(kernel, io)
  await until(() => io.text.includes("PROMPT$ "), "the redrawn screen")
  assert.deepEqual(kernel.size, { cols: 100, rows: 40 }, "the terminal takes the outer terminal's size")
  assert.deepEqual(io.rawModes, [true])

  io.type("ls\r")
  await until(() => kernel.typed.length === 1, "typed bytes reach the terminal")
  assert.equal(kernel.typed[0]!.toString(), "ls\r")
  kernel.emit("file-a\r\nPROMPT$ ")
  await until(() => io.text.includes("file-a"), "terminal output on the screen")

  io.type(Buffer.from([...Buffer.from("x"), TERMINAL_DETACH_BYTE, ...Buffer.from("never sent")]))
  assert.deepEqual(await done, { reason: "detached" })
  assert.equal(kernel.typed.map(bytes => bytes.toString()).join("|"), "ls\r|x", "keys before Ctrl-] are delivered, Ctrl-] and what follows are not")
  assert.deepEqual(io.rawModes, [true, false])
  assert.ok(io.paused)
  assert.ok(io.text.endsWith("\x1b[?7h") || io.text.includes("\x1b[?25h"), "the outer terminal's modes are reset")
})

test("a detach reported with the kitty keyboard protocol also detaches", async () => {
  const kernel = new FakeTerminalKernel()
  const io = new FakeTerminalIo()
  const done = attach(kernel, io)
  await until(() => io.text.includes("PROMPT$ "), "the redrawn screen")
  io.type("\x1b[93;5u")
  assert.deepEqual(await done, { reason: "detached" })
  assert.equal(kernel.typed.length, 0)
})

test("the attach owns the outer alternate screen; a full-screen program inside never switches it", async () => {
  const kernel = new FakeTerminalKernel()
  const io = new FakeTerminalIo()
  const done = attach(kernel, io)
  await until(() => io.text.includes("PROMPT$ "), "the redrawn screen")
  assert.ok(io.text.startsWith("\x1b[?1049h"), "the attach enters the outer alternate screen first")
  kernel.emit("\x1b[?1049h\x1b[Hvim screen")
  await until(() => io.text.includes("vim screen"), "the full-screen program's output")
  assert.equal(io.text.split("\x1b[?1049h").length - 1, 1, "the program's switch to its own alternate screen is not forwarded")
  assert.ok(io.text.includes("\x1b[2J\x1b[Hvim screen") || io.text.includes("\x1b[0m\x1b[H\x1b[2J\x1b[Hvim screen"), "it becomes a clear")
  // The program quits: the kernel holds the shell screen it left behind, and the attach redraws it.
  kernel.screen = "BACK-AT-PROMPT$ "
  kernel.emit("\x1b[?1049l")
  await until(() => io.text.includes("BACK-AT-PROMPT$ "), "the normal screen redrawn from the kernel")
  assert.equal(io.text.split("\x1b[?1049l").length - 1, 0, "the program's switch back is not forwarded either")
  io.type(Buffer.from([TERMINAL_DETACH_BYTE]))
  await done
  assert.equal(io.text.split("\x1b[?1049l").length - 1, 1, "the outer alternate screen is left exactly once, at the end")
  assert.ok(io.text.endsWith("\x1b[?1049l"))
})

test("a screen-switch sequence split across two outputs is still recognised", async () => {
  const kernel = new FakeTerminalKernel()
  const io = new FakeTerminalIo()
  const done = attach(kernel, io)
  await until(() => io.text.includes("PROMPT$ "), "the redrawn screen")
  kernel.emit("a\x1b[?10")
  await until(() => io.text.includes("a"), "the first half")
  kernel.emit("49hb")
  await until(() => io.text.includes("b"), "the second half")
  assert.equal(io.text.split("\x1b[?1049h").length - 1, 1, "only the attach's own switch reaches the outer terminal")
  assert.ok(!io.text.includes("49hb"), "the split sequence is not leaked as text")
  io.type(Buffer.from([TERMINAL_DETACH_BYTE]))
  await done
})

test("a lone escape byte that nothing completes is shown after a quiet poll", async () => {
  const kernel = new FakeTerminalKernel()
  const io = new FakeTerminalIo()
  const done = attach(kernel, io)
  await until(() => io.text.includes("PROMPT$ "), "the redrawn screen")
  kernel.emit("x\x1b")
  await until(() => io.text.endsWith("x\x1b"), "the held escape byte after a quiet poll")
  io.type(Buffer.from([TERMINAL_DETACH_BYTE]))
  await done
})

test("resizing the outer terminal resizes the Room terminal", async () => {
  const kernel = new FakeTerminalKernel()
  const io = new FakeTerminalIo()
  const done = attach(kernel, io)
  await until(() => io.text.includes("PROMPT$ "), "the redrawn screen")
  io.resizeTo(132, 50)
  await until(() => kernel.size.cols === 132, "the resize")
  assert.deepEqual(kernel.size, { cols: 132, rows: 50 })
  io.type(Buffer.from([TERMINAL_DETACH_BYTE]))
  await done
})

test("a lost connection keeps the screen and resumes from the same offset without gaps or repeats", async () => {
  const kernel = new FakeTerminalKernel()
  const io = new FakeTerminalIo()
  const done = attach(kernel, io)
  await until(() => io.text.includes("PROMPT$ "), "the redrawn screen")
  kernel.emit("one ")
  await until(() => io.text.includes("one "), "first output")
  const before = io.written.length
  kernel.failNext = 3
  kernel.emit("two ")
  await until(() => io.text.includes("two "), "output after the reconnect")
  kernel.emit("three")
  await until(() => io.text.includes("three"), "later output")
  const drawn = io.written.slice(before).filter(chunk => !chunk.startsWith("\x1b]2;"))
  assert.equal(drawn.join(""), "two three")
  assert.ok(!io.text.includes("\x1b[2J", io.text.indexOf("one ")), "the screen is never cleared by a reconnect")
  assert.ok(io.written.some(chunk => chunk.includes("reconnecting")), "the title tells the user")
  io.type(Buffer.from([TERMINAL_DETACH_BYTE]))
  await done
})

test("Ctrl-] detaches while the first requests keep failing and restores the outer terminal", async () => {
  const kernel = new FakeTerminalKernel()
  const io = new FakeTerminalIo()
  kernel.failNext = 1_000_000
  const done = attach(kernel, io)
  await until(() => io.written.some(chunk => chunk.includes("reconnecting")), "the attach is still connecting")
  io.type(Buffer.from([TERMINAL_DETACH_BYTE]))
  const end = await Promise.race([done, new Promise<string>(resolve => setTimeout(() => resolve("stuck"), 3000))])
  assert.deepEqual(end, { reason: "detached" }, "Ctrl-] ends an attach that never connected")
  assert.deepEqual(io.rawModes, [true, false])
  assert.ok(io.paused)
  assert.ok(io.text.endsWith("\x1b[?1049l") || io.text.includes("\x1b[?1049l"), "the outer alternate screen is left")
  assert.equal(kernel.typed.length, 0, "nothing typed before the screen is in sync reaches the terminal")
})

test("Ctrl-] restores the outer terminal while an input request is unanswered and drops the keys queued behind it", async () => {
  const kernel = new FakeTerminalKernel()
  const io = new FakeTerminalIo()
  const done = attach(kernel, io)
  await until(() => io.text.includes("PROMPT$ "), "the redrawn screen")
  kernel.stallInput = true
  io.type("a")
  await until(() => kernel.stalled.length === 1, "the first key is in flight and unanswered")
  io.type("b")
  io.type(Buffer.from([TERMINAL_DETACH_BYTE]))
  const end = await Promise.race([done, new Promise<string>(resolve => setTimeout(() => resolve("stuck"), 3000))])
  assert.deepEqual(end, { reason: "detached" }, "a stalled input request does not hold the detach")
  assert.deepEqual(io.rawModes, [true, false])
  assert.ok(io.paused)
  assert.ok(io.text.includes("\x1b[?1049l"), "the outer alternate screen is left")
  kernel.stallInput = false
  await new Promise(resolve => setTimeout(resolve, 50))
  assert.equal(kernel.stalled.length, 1, "the key queued behind the stalled one is never sent after the detach")
  assert.equal(kernel.typed.length, 0)
})

test("a snapshot still in flight when Ctrl-] detaches draws nothing on the restored outer terminal", async () => {
  const kernel = new FakeTerminalKernel()
  const io = new FakeTerminalIo()
  const done = attach(kernel, io)
  await until(() => io.text.includes("PROMPT$ "), "the redrawn screen")
  kernel.holdSnapshots = true
  kernel.retainedFrom = 1_000_000
  kernel.screen = "LATE$ "
  kernel.emit("later")
  await until(() => kernel.requests.filter(request => request.operation === "snapshot").length === 2, "the resync snapshot is in flight")
  io.type(Buffer.from([TERMINAL_DETACH_BYTE]))
  assert.deepEqual(await done, { reason: "detached" })
  const written = io.written.length
  kernel.releaseSnapshots()
  await new Promise(resolve => setTimeout(resolve, 50))
  assert.equal(io.written.length, written, "nothing is written after the detach")
  assert.ok(!io.text.includes("LATE$ "))
})

test("output the kernel no longer retains triggers a fresh redraw instead of a hole", async () => {
  const kernel = new FakeTerminalKernel()
  const io = new FakeTerminalIo()
  const done = attach(kernel, io)
  await until(() => io.text.includes("PROMPT$ "), "the redrawn screen")
  kernel.retainedFrom = 1_000_000
  kernel.screen = "FRESH$ "
  kernel.emit("later")
  await until(() => io.text.includes("FRESH$ "), "the resynced screen")
  io.type(Buffer.from([TERMINAL_DETACH_BYTE]))
  await done
})

test("an ended terminal ends the attach with the kernel's reason after its last output", async () => {
  const kernel = new FakeTerminalKernel()
  const io = new FakeTerminalIo()
  const done = attach(kernel, io)
  await until(() => io.text.includes("PROMPT$ "), "the redrawn screen")
  kernel.emit("bye\r\n")
  kernel.end("ended: exited with code 0")
  assert.deepEqual(await done, { reason: "ended", description: "ended: exited with code 0" })
  assert.ok(io.text.includes("bye"))
})

test("a kernel refusal ends the attach with its message and does not retry", async () => {
  const kernel = new FakeTerminalKernel()
  const io = new FakeTerminalIo()
  kernel.refuse = "terminal belongs to the Room owner"
  assert.deepEqual(await attach(kernel, io), { reason: "ended", description: "terminal belongs to the Room owner" })
  assert.equal(kernel.requests.filter(request => request.operation === "resize").length, 1)
})

test("a terminal on a worker is addressed through its host agent", async () => {
  const kernel = new FakeTerminalKernel()
  const io = new FakeTerminalIo()
  const seen: unknown[] = []
  const done = attachRoomTerminal({
    send: request => { seen.push(request); return kernel.send(request) },
    target: { sessionId: "room" }, terminal: { ...kernel.summary(), host_agent: "worker-agent" }, input: io, output: io,
  })
  await until(() => io.text.includes("PROMPT$ "), "the redrawn screen")
  io.type(Buffer.from([TERMINAL_DETACH_BYTE]))
  await done
  assert.ok(seen.every(request => (request as { RoomTerminal: { host_agent?: string } }).RoomTerminal.host_agent === "worker-agent"))
})
