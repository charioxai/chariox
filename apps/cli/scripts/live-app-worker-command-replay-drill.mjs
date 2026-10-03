#!/usr/bin/env node
// Restart a disposable fixture worker, then replay the exact command after it
// reaches Running. A cached reply alone is insufficient: the worker's current
// timestamp must also stay unchanged. Does not restart the kernel.
// bun scripts/live-app-worker-command-replay-drill.mjs --installation ID
//   [--kernel-url ws://127.0.0.1:44240/kernel] [--evidence FILE]
import assert from "node:assert/strict"
import { randomUUID } from "node:crypto"
import { writeFile } from "node:fs/promises"

const options = { kernelUrl: "ws://127.0.0.1:44240/kernel" }
for (let i = 2; i < process.argv.length; i += 2) {
  const key = { "--installation": "installation", "--kernel-url": "kernelUrl", "--evidence": "evidence" }[process.argv[i]]
  if (!key || !process.argv[i + 1]) throw new Error("Expected --installation ID [--kernel-url URL] [--evidence FILE]")
  options[key] = process.argv[i + 1]
}
assert.ok(options.installation, "Select a disposable fixture installation")
const records = [], pending = new Map(), commandId = `app-restart-replay-${randomUUID()}`
const socket = new WebSocket(options.kernelUrl)
socket.addEventListener("message", (event) => {
  const frame = JSON.parse(String(event.data))
  pending.get(frame.request_id)?.resolve(frame)
})
socket.addEventListener("close", () => {
  for (const request of pending.values()) request.reject(new Error("Kernel connection closed"))
})
const send = async (request, command_id = randomUUID()) => {
  const frame = { type: "request", request_id: randomUUID(), command_id, request }
  let timer
  try {
    const reply = new Promise((resolve, reject) => {
      pending.set(frame.request_id, { resolve, reject })
      timer = setTimeout(() => reject(new Error("Kernel request timed out")), 15_000)
    })
    socket.send(JSON.stringify(frame))
    const response = await reply
    records.push({ at: new Date().toISOString(), frame, response })
    assert.equal(response.error, null, JSON.stringify(response))
    return response.response
  } finally {
    clearTimeout(timer)
    pending.delete(frame.request_id)
  }
}
const running = async () => {
  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    const result = await send({ GetAppWorker: { installation_id: options.installation } })
    const worker = result.AppWorker?.worker
    assert.notEqual(worker?.phase, "failed", JSON.stringify(result))
    if (worker?.phase === "running") return worker
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
  throw new Error("Worker did not reach Running")
}
let failure
try {
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("Kernel connection timed out")), 10_000)
    socket.addEventListener("open", () => { clearTimeout(timer); resolve() }, { once: true })
    socket.addEventListener("error", () => { clearTimeout(timer); reject(new Error("Kernel connection failed")) }, { once: true })
  })
  const request = { ControlAppWorker: { installation_id: options.installation, action: "restart" } }
  const first = await send(request, commandId)
  assert.equal(first.AppWorker?.worker?.phase, "starting")
  const settled = await running()
  const replay = await send(request, commandId)
  assert.deepEqual(replay, first, "Replay must return the original receipt")
  assert.deepEqual(await running(), settled, "Replay restarted the worker")
  await new Promise((resolve) => setTimeout(resolve, 1_000))
  assert.deepEqual(await running(), settled, "Replay scheduled a delayed restart")
  console.log("PASS: original receipt replayed; running worker unchanged immediately and after 1 second")
} catch (error) {
  failure = String(error.stack ?? error)
  process.exitCode = 1
  console.error(failure)
} finally {
  socket.close()
  if (options.evidence) await writeFile(options.evidence, JSON.stringify({ command_id: commandId, passed: !failure, failure, records }, null, 2) + "\n")
}
