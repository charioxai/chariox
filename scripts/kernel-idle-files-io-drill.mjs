#!/usr/bin/env node
// Idle scratch maintenance must not scan other processes when there is nothing to trim.
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { openSync } from "node:fs"
import { randomUUID } from "node:crypto"
import { mkdir, mkdtemp, open, readFile, readdir, rm, writeFile } from "node:fs/promises"
import { createServer } from "node:net"
import path from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import { LocalIpcClient } from "../packages/kernel-client/dist/ipc.js"

const [binary, stateRoot, evidence] = process.argv.slice(2)
assert.ok(binary && stateRoot && evidence, "usage: kernel-idle-files-io-drill.mjs <kernel-binary> <state-root> <evidence-dir>")
assert.equal(process.platform, "linux", "the syscall accounting drill needs Linux and strace")
await Promise.all([mkdir(stateRoot, { recursive: true }), mkdir(evidence, { recursive: true })])
const scratch = await mkdtemp(path.join(stateRoot, "files-io-"))
const server = createServer()
await new Promise(resolve => server.listen(0, "127.0.0.1", resolve))
const port = server.address().port
await new Promise(resolve => server.close(resolve))
const extraPorts = []
for (let i = 0; i < 3; i++) {
  const listener = createServer()
  await new Promise(resolve => listener.listen(0, "127.0.0.1", resolve))
  extraPorts.push(listener.address().port)
  await new Promise(resolve => listener.close(resolve))
}
const home = path.join(scratch, "home"), state = path.join(scratch, "state")
await Promise.all([mkdir(home), mkdir(state)])
await writeFile(path.join(state, "config.toml"), `[credential_vault]\nbackend = "chariox_encrypted"\npath = ${JSON.stringify(path.join(state, "vault.json"))}\nunlock_policy = "kernel_init"\n`)
const env = { PATH: process.env.PATH, HOME: home, TMPDIR: scratch, SHELL: "/bin/bash", LANG: "C.UTF-8", RUST_LOG: "error",
  CHARIOX_HOME: state, CHARIOX_DAEMON_SOCKET: path.join(scratch, "kernel.sock"), CHARIOX_MACHINE_ID: randomUUID(),
  CHARIOX_KERNEL_HOST: "127.0.0.1", CHARIOX_KERNEL_PORT: String(port), CHARIOX_MCP_PORT: String(extraPorts[0]), CHARIOX_CODEX_PORT: String(extraPorts[1]), CHARIOX_OPENCODE_PORT: String(extraPorts[2]) }
const kernel = spawn(path.resolve(binary), [], { cwd: scratch, env, stdio: ["ignore", "ignore", openSync(path.join(evidence, "kernel.log"), "a", 0o600)] })
const client = new LocalIpcClient(`ws://127.0.0.1:${port}/kernel`, { localAuthEnvironment: env, controlRequestRetryDeadlineMs: 1000 })
let trace
const outcome = { reads: 0, requestedBytes: 0 }
async function stop(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return
  assert.ok(Number.isSafeInteger(child.pid) && child.pid > 1)
  const ended = new Promise(resolve => child.once("exit", resolve))
  child.kill("SIGTERM")
  const timer = setTimeout(() => { if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL") }, 15_000)
  try { await ended } finally { clearTimeout(timer) }
}
try {
  const ready = Date.now() + 60_000
  for (;;) {
    try { if ((await client.send({ GetDaemonHealth: null })).DaemonHealth) break } catch {}
    if (Date.now() > ready) throw Error(`isolated kernel did not become ready; see ${path.join(evidence, "kernel.log")}`)
    await delay(100)
  }
  const file = path.join(home, "large.log"), size = 200 * 1024 * 1024
  const log = await open(file, "w")
  await log.truncate(size)
  await log.write(Buffer.from("first line\n"), 0, 11, 0)
  await log.write(Buffer.from("last line\n"), 0, 10, size - 10)
  await log.close()
  const traceFile = path.join(evidence, "syscalls.log")
  // No buffer contents: the trace retains paths and byte counts only.
  trace = spawn("strace", ["-f", "-qq", "-s", "0", "-yy", "-e", "trace=read,pread64", "-p", String(kernel.pid), "-o", traceFile], { stdio: "ignore" })
  await delay(500)
  assert.equal(trace.exitCode, null, "strace must attach before measuring")
  const startedAt = Date.now(), until = startedAt + 65_000
  let emptyRootCreated = false
  do {
    for (const offset of [0, size - 65536]) {
      const response = await client.send({ ReadKernelFile: { path: file, offset, length: 65536 } })
      assert.ok(response.KernelFileChunk, "real Files read answered")
      assert.equal(Buffer.from(response.KernelFileChunk.data_base64, "base64").length, 65536)
      outcome.reads++
      outcome.requestedBytes += 65536
    }
    if (!emptyRootCreated && Date.now() - startedAt >= 30_000) {
      await mkdir(path.join(state, "t"))
      emptyRootCreated = true
    }
    await delay(500)
  } while (Date.now() < until)
  assert.deepEqual(await readdir(path.join(state, "t")), [], "the drill created no agent scratch")
  await stop(trace)
  const syscalls = await readFile(traceFile, "utf8")
  const maps = syscalls.split("\n").flatMap(line => {
    const match = /^(\d+) (?:read|pread64)\(\d+<\/proc\/(\d+)\/maps>/.exec(line)
    return match ? [{ reader: Number(match[1]), target: Number(match[2]) }] : []
  })
  // Child runtimes legitimately read their own mappings; the scratch sweep reads other processes.
  outcome.selfMapReads = maps.filter(({ reader, target }) => reader === target).length
  outcome.foreignProcessMapReads = maps.filter(({ reader, target }) => reader !== target).length
  outcome.result = outcome.foreignProcessMapReads === 0 ? "PASS" : "FAIL"
  await writeFile(path.join(evidence, "result.json"), JSON.stringify(outcome, null, 2))
  assert.equal(outcome.foreignProcessMapReads, 0, "idle scratch maintenance scanned other processes despite there being nothing to trim")
  console.log(JSON.stringify(outcome))
} finally {
  await stop(trace)
  client.close()
  await stop(kernel)
  await rm(scratch, { recursive: true, force: true })
}
