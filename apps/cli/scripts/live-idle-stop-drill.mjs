#!/usr/bin/env node
// MP-08 / MP-09 / MP-10 / MP-11: real TUI Stop before and after a provider turn.
// Supply a running kernel and an idle session with an authenticated real provider.
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { copyFile, mkdir, open, rm, writeFile } from "node:fs/promises"
import net from "node:net"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { setTimeout as delay } from "node:timers/promises"

const options = new Map()
for (let i = 2; i < process.argv.length; i += 2) options.set(process.argv[i], process.argv[i + 1])
for (const key of ["--kernel-url", "--session", "--evidence-dir"]) {
  assert(options.get(key), `required: ${key}`)
}
if (options.get("--relay-url")) {
  assert(options.get("--target-daemon-id"), "relay drill requires --target-daemon-id")
  assert(process.env.IDLE_STOP_RELAY_TOKEN, "relay drill requires IDLE_STOP_RELAY_TOKEN")
}
const repo = fileURLToPath(new URL("../../../", import.meta.url))
const evidence = path.resolve(options.get("--evidence-dir"))
await mkdir(evidence, { recursive: true })
const socketPath = path.join("/tmp", `cx-stop-${process.pid}.sock`)
const output = await open(path.join(evidence, "tui.ansi"), "w", 0o600)
const relayArgs = options.get("--relay-url") ? [
  "--relay-url", quote(options.get("--relay-url")), "--relay-token-env", "IDLE_STOP_RELAY_TOKEN",
  "--target-daemon-id", quote(options.get("--target-daemon-id")),
] : []
const pairingLink = process.env.IDLE_STOP_TERMINAL_PAIRING_LINK
const endpointArgs = pairingLink
  ? ["--terminal-pairing-link", quote(pairingLink)]
  : relayArgs.length ? relayArgs : ["--kernel-url", quote(options.get("--kernel-url"))]
const cli = spawn("script", ["-q", "-c", [
  "stty cols 120 rows 40; exec bun", quote(path.join(repo, "apps/cli/dist/index.js")),
  "--session", quote(options.get("--session")),
  "--automation-socket", quote(socketPath), ...endpointArgs,
].join(" "), "/dev/null"], {
  cwd: repo, env: { ...process.env, TERM: "xterm-256color", CHARIOX_TEST_TUI: "1" },
  stdio: ["pipe", output.fd, output.fd],
})
let socket
let nextId = 0
const pending = new Map()
let buffer = ""
const steps = []
let passed = false
try {
  await until(async () => {
    if (cli.exitCode !== null) throw new Error(`TUI exited: ${cli.exitCode}`)
    try {
      socket = net.createConnection(socketPath)
      await new Promise((resolve, reject) => { socket.once("connect", resolve); socket.once("error", reject) })
      return true
    } catch { socket?.destroy(); return false }
  }, 30_000)
  socket.setEncoding("utf8")
  socket.on("data", chunk => {
    buffer += chunk
    while (buffer.includes("\n")) {
      const end = buffer.indexOf("\n")
      const response = JSON.parse(buffer.slice(0, end))
      buffer = buffer.slice(end + 1)
      const waiter = pending.get(response.id)
      if (!waiter) continue
      pending.delete(response.id)
      clearTimeout(waiter.timer)
      if (response.ok) waiter.resolve(response.data)
      else waiter.reject(new Error(response.error))
    }
  })
  const send = (action, fields = {}) => new Promise((resolve, reject) => {
    const id = ++nextId
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`TUI ${action} timed out`)) }, 30_000)
    pending.set(id, { resolve, reject, timer })
    socket.write(JSON.stringify({ id, action, ...fields }) + "\n")
  })
  const save = async (step, data) => {
    await output.sync()
    await copyFile(path.join(evidence, "tui.ansi"), path.join(evidence, `${step}.ansi`))
    steps.push({ step, at: new Date().toISOString(), data })
    await writeFile(path.join(evidence, "steps.json"), JSON.stringify(steps, null, 2) + "\n")
  }
  await save("ready", await send("snapshot"))
  await send("submit_prompt", { prompt: "/workflow new idle-stop-drill" })
  const workflow = await send("snapshot")
  await save("workflow-ready", workflow)
  assert(workflow.workflows.some(entry => entry.alias === "idle-stop-drill"), "workflow setup failed")
  const stop = async step => {
    await send("switch_screen", { screen: "workflow" })
    await send("submit_prompt", { prompt: "@ stop" })
    await delay(250)
    const snapshot = await send("snapshot")
    await save(step, snapshot)
    const entry = snapshot.shell.entries.at(-1)
    assert.equal(entry?.command, "stop", JSON.stringify(snapshot.footer))
    assert.equal(entry.ok, true)
    assert.equal(entry.output.trim(), "nothing to stop")
  }
  for (const step of ["idle-stop", "repeated-idle-stop"]) await stop(step)
  await send("switch_screen", { screen: "agents" })
  await send("submit_prompt", { prompt: "Reply with exactly STOPDRILLREADY and do not use tools." })
  await until(async () => {
    const snapshot = await send("snapshot")
    return snapshot.transcript.entries.some(entry => entry.role === "assistant" && entry.text.trim() === "STOPDRILLREADY")
      && snapshot.session.agents.every(agent => !agent.isProcessing)
  }, 120_000)
  await save("provider-finished", await send("snapshot"))
  for (const step of ["finished-stop", "repeated-finished-stop"]) await stop(step)
  await send("exit")
  passed = true
} finally {
  socket?.destroy()
  for (const waiter of pending.values()) clearTimeout(waiter.timer)
  if (cli.exitCode === null && cli.signalCode === null) {
    assert(Number.isSafeInteger(cli.pid) && cli.pid > 1, "invalid owned TUI PID")
    cli.stdin.end()
    try { await until(() => cli.exitCode !== null || cli.signalCode !== null, 5000) }
    catch {
      cli.kill("SIGTERM")
      await until(() => cli.exitCode !== null || cli.signalCode !== null, 10_000)
    }
  }
  await output.close()
  await rm(socketPath, { force: true })
  await writeFile(path.join(evidence, "result.json"), JSON.stringify({
    passed, cliPid: cli.pid, exitCode: cli.exitCode, signal: cli.signalCode,
    kernelUrl: options.get("--kernel-url"), session: options.get("--session"),
  }, null, 2) + "\n")
}

function quote(value) { return "'" + value.replaceAll("'", "'\\''") + "'" }
async function until(check, timeout) {
  const deadline = Date.now() + timeout
  do { if (await check()) return; await delay(250) } while (Date.now() < deadline)
  throw new Error("idle Stop drill timed out")
}
