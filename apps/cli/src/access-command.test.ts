// MP-08 / MP-10 / MP-11: one process-bound grant for the whole local kernel.
import assert from "node:assert/strict"
import { test } from "node:test"
import { mkdtempSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { parseAccessRequest, runAccessCommand } from "./access-command.js"
import { replyWith, startFakeKernel } from "./fake-unix-kernel.test-support.js"

test("access request has no session scope and defaults to the caller's OS ancestor", () => {
  const result = parseAccessRequest(["--holder-pid", "42", "--minutes", "480", "--socket", "/tmp/k.sock"], () => { throw new Error("unused") })
  assert.deepEqual(result, { request: { RequestKernelAccess: { holder_pid: 42, term: { minutes: 480 } } }, socket: "/tmp/k.sock", noWait: false })
  assert.equal(parseAccessRequest([], () => 43).request.RequestKernelAccess.holder_pid, 43)
  for (const argv of [["--session", "s"], ["--holder-pid", "0"], ["--holder-pid", "1"], ["--minutes", "1.5"], ["--holder-pid", "5", "--holder-pid", "6"], ["--passkey", "never"], ["--until-exit", "--minutes", "30"], ["--until-exit", "--until-exit"]]) {
    assert.throws(() => parseAccessRequest(argv, () => 43))
  }
})

test("access request can ask to last until the agent exits, and says nothing when no term is given", () => {
  assert.deepEqual(parseAccessRequest(["--until-exit"], () => 43).request, { RequestKernelAccess: { holder_pid: 43, term: "until_holder_exits" } })
  assert.deepEqual(parseAccessRequest(["--socket", "/tmp/k.sock", "--until-exit", "--holder-pid", "7"], () => 43), { request: { RequestKernelAccess: { holder_pid: 7, term: "until_holder_exits" } }, socket: "/tmp/k.sock", noWait: false })
  assert.deepEqual(parseAccessRequest([], () => 43).request, { RequestKernelAccess: { holder_pid: 43 } })
})

test("--no-wait is a flag of access request and changes nothing else", () => {
  assert.deepEqual(parseAccessRequest(["--until-exit", "--no-wait"], () => 43), {
    request: { RequestKernelAccess: { holder_pid: 43, term: "until_holder_exits" } }, socket: undefined, noWait: true,
  })
  assert.throws(() => parseAccessRequest(["--no-wait", "--no-wait"], () => 43))
})

async function statusOf(answer: Parameters<typeof replyWith>[2], runtimeDir: string, holder: number): Promise<{ out: string; exit: number | undefined }> {
  const kernel = await startFakeKernel((socket, frame) => replyWith(socket, frame, answer))
  const previous = { out: console.log, dir: process.env.XDG_RUNTIME_DIR, exit: process.exitCode }
  let out = ""
  console.log = (line: string) => { out += line }
  process.env.XDG_RUNTIME_DIR = runtimeDir
  try {
    await runAccessCommand(["access", "status", "--holder-pid", String(holder), "--socket", kernel.path])
    return { out, exit: process.exitCode as number | undefined }
  } finally {
    console.log = previous.out
    if (previous.dir === undefined) delete process.env.XDG_RUNTIME_DIR; else process.env.XDG_RUNTIME_DIR = previous.dir
    process.exitCode = previous.exit
    await kernel.close()
  }
}

test("access status reports granted, ended, pending and not_requested from a probe and the waiter pid file", async () => {
  const dir = mkdtempSync(join(tmpdir(), "chx-status-"))
  const refusal = (code: string) => ({ error: { code, message: code, retryable: false } })
  try {
    assert.equal((await statusOf({ response: { GatewayKernelsListed: { kernels: [] } } }, dir, 4242)).out, '{"status":"granted"}')
    assert.equal((await statusOf(refusal("kernel_access_ended"), dir, 4242)).out, '{"status":"ended"}')
    assert.equal((await statusOf(refusal("kernel_access_not_covered"), dir, 4242)).out, '{"status":"not_requested"}')
    writeFileSync(join(dir, "access-waiter-4242.pid"), String(process.pid), { mode: 0o600 })
    assert.equal((await statusOf(refusal("kernel_access_not_covered"), dir, 4242)).out, '{"status":"pending"}')
    assert.equal((await statusOf(refusal("kernel_access_not_covered"), dir, 4243)).out, '{"status":"not_requested"}')
    for (const invalid of ["0", "1", "-1", "NaN", ""]) {
      writeFileSync(join(dir, "access-waiter-4242.pid"), invalid, { mode: 0o600 })
      assert.equal((await statusOf(refusal("kernel_access_not_covered"), dir, 4242)).out, '{"status":"not_requested"}')
    }
  } finally { rmSync(dir, { recursive: true, force: true }) }
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
