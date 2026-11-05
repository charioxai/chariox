// MP-08 / MP-10 / MP-11: one process-bound grant for the whole local kernel.
import assert from "node:assert/strict"
import { test } from "node:test"
import { parseAccessRequest, runAccessCommand } from "./access-command.js"
import { replyWith, startFakeKernel } from "./fake-unix-kernel.test-support.js"

test("access request has no session scope and leaves the holder to the kernel unless --holder-pid names a host pid", () => {
  const result = parseAccessRequest(["--holder-pid", "42", "--minutes", "480", "--socket", "/tmp/k.sock"])
  assert.deepEqual(result, { request: { RequestKernelAccess: { holder_pid: 42, term: { minutes: 480 } } }, socket: "/tmp/k.sock", noWait: false })
  assert.deepEqual(parseAccessRequest([]).request, { RequestKernelAccess: {} })
  for (const argv of [["--session", "s"], ["--holder-pid", "0"], ["--holder-pid", "1"], ["--minutes", "1.5"], ["--holder-pid", "5", "--holder-pid", "6"], ["--passkey", "never"], ["--until-exit", "--minutes", "30"], ["--until-exit", "--until-exit"]]) {
    assert.throws(() => parseAccessRequest(argv))
  }
})

test("access request can ask to last until the agent exits, and --no-wait asks the kernel to wait instead", () => {
  assert.deepEqual(parseAccessRequest(["--until-exit"]).request, { RequestKernelAccess: { term: "until_holder_exits" } })
  assert.deepEqual(parseAccessRequest(["--socket", "/tmp/k.sock", "--until-exit", "--holder-pid", "7"]), { request: { RequestKernelAccess: { holder_pid: 7, term: "until_holder_exits" } }, socket: "/tmp/k.sock", noWait: false })
  assert.deepEqual(parseAccessRequest(["--until-exit", "--no-wait"]), { request: { RequestKernelAccess: { term: "until_holder_exits", no_wait: true } }, socket: undefined, noWait: true })
  assert.throws(() => parseAccessRequest(["--no-wait", "--no-wait"]))
})

async function run(argv: string[], answer: Parameters<typeof replyWith>[2]) {
  const kernel = await startFakeKernel((socket, frame) => replyWith(socket, frame, answer))
  const previous = { out: console.log, err: console.error, exit: process.exitCode }
  let out = ""
  console.log = (line: string) => { out += line }
  console.error = () => {}
  try {
    await runAccessCommand([...argv, "--socket", kernel.path])
    return { out, exit: process.exitCode as number | undefined, requests: kernel.frames.map(frame => frame.request) }
  } finally {
    console.log = previous.out
    console.error = previous.err
    process.exitCode = previous.exit
    await kernel.close()
  }
}

const status = (state: string, extra: Record<string, unknown> = {}) => ({ response: { KernelAccessStatus: { status: { state, holder_pid: 4242, holder_executable: "/usr/bin/codex", ...extra } } } })

test("--no-wait leaves the pending request with the kernel: exit 6 and no process left behind", async () => {
  const pending = await run(["access", "request", "--until-exit", "--no-wait"], status("pending"))
  assert.deepEqual(pending.requests, [{ RequestKernelAccess: { term: "until_holder_exits", no_wait: true } }])
  assert.deepEqual(JSON.parse(pending.out), { status: "pending", holder_pid: 4242, note: "approve in your Chariox client, then run chariox access status" })
  assert.equal(pending.exit, 6)
  const grant = { grant_id: "access-1", owner_user_id: "local", holder_pid: 4242, holder_executable: "/usr/bin/codex", term: "until_holder_exits" }
  const standing = await run(["access", "request", "--until-exit", "--no-wait"], status("granted", { grant }))
  assert.deepEqual(JSON.parse(standing.out), grant)
  assert.equal(standing.exit, undefined)
})

test("access status asks the kernel about one holder identity and prints its state and reason", async () => {
  const granted = await run(["access", "status"], status("granted"))
  assert.deepEqual(granted.requests, [{ GetKernelAccessStatus: {} }])
  assert.equal(granted.out, '{"status":"granted","holder_pid":4242}')
  const named = await run(["access", "status", "--holder-pid", "4243"], status("not_requested"))
  assert.deepEqual(named.requests, [{ GetKernelAccessStatus: { holder_pid: 4243 } }])
  assert.equal(named.out, '{"status":"not_requested","holder_pid":4242}')
  const refused = await run(["access", "status"], status("ended", { reason: { code: "access_refused", message: "the user refused access" } }))
  assert.deepEqual(JSON.parse(refused.out), { status: "ended", holder_pid: 4242, reason: "access_refused", message: "the user refused access" })
  // A pid outside the caller's chain is the kernel's refusal, never the caller's own grant.
  const outside = await run(["access", "status", "--holder-pid", "99999999"], { error: { code: "kernel_access_not_covered", message: "not covered", retryable: false } })
  assert.equal(outside.out, "")
  assert.notEqual(outside.exit, 0)
})

test("access cover names one pid to the kernel and prints the grant id and the process it now covers", async () => {
  const process_ = { pid: 4242, start: 77, executable: "/usr/bin/shipper" }
  const kernel = await startFakeKernel((socket, frame) => replyWith(socket, frame, { response: { KernelAccessCovered: { grant_id: "access-1", process: process_ } } }))
  const previous = { out: console.log, err: console.error, exit: process.exitCode }
  let out = ""
  console.log = (line: string) => { out += line }
  try {
    assert.equal(await runAccessCommand(["access", "cover", "--pid", "4242", "--socket", kernel.path]), true)
    assert.deepEqual(kernel.frames.map(frame => frame.request), [{ CoverKernelAccessProcess: { pid: 4242 } }])
    assert.deepEqual(JSON.parse(out), { grant_id: "access-1", process: process_ })
    for (const pid of ["0", "1", "-3", "x"]) {
      process.exitCode = undefined
      console.error = () => {}
      await runAccessCommand(["access", "cover", "--pid", pid, "--socket", kernel.path])
      assert.equal(process.exitCode, 2, `pid ${pid}`)
    }
    assert.equal(kernel.frames.length, 1)
  } finally {
    console.log = previous.out
    console.error = previous.err
    process.exitCode = previous.exit
    await kernel.close()
  }
})

test("access --help prints the usage and exits 0", async () => {
  const printed: string[] = []
  const log = console.log
  const previous = process.exitCode
  console.log = (line: string) => { printed.push(line) }
  try {
    process.exitCode = undefined
    assert.equal(await runAccessCommand(["access", "--help"]), true)
    assert.match(printed.join("\n"), /chariox access request/)
    assert.ok(!process.exitCode)
  } finally { console.log = log; process.exitCode = previous }
})

test("an unknown access verb is a usage answer, not a crash", async () => {

  const previous = { err: process.stderr.write, exit: process.exitCode }
  let err = ""
  process.stderr.write = ((chunk: string) => { err += chunk; return true }) as typeof process.stderr.write
  try {
    for (const verb of ["bogus", "constructor", "toString", "__proto__", "hasOwnProperty"]) {
      process.exitCode = undefined
      err = ""

      assert.equal(await runAccessCommand(["access", verb]), true)
      assert.equal(JSON.parse(err.trim().split("\n").pop()!).error.code, "usage", verb)
      assert.equal(process.exitCode, 2, verb)
    }
  } finally {
    process.stderr.write = previous.err
    process.exitCode = previous.exit
  }
})
