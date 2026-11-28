import assert from "node:assert/strict"
import test from "node:test"
import { Readable } from "node:stream"
import { CommandFailure } from "./agent-command-table.js"
import { MAX_INLINE_STDIN_BYTES, lateStdin, ownStdin, readExecInput } from "./kernel-exec.js"

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
