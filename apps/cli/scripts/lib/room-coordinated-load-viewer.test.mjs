import assert from "node:assert/strict"
import test from "node:test"
import { setImmediate as yieldTurn } from "node:timers/promises"
import { startLoadViewerReader } from "./room-coordinated-load-viewer.mjs"

function fixture() {
  let pending
  const queue = []
  const stream = {
    async close() { pending?.reject(new Error("closed")) },
    push(value) { if (pending) { const receiver = pending; pending = null; receiver.resolve(value) } else queue.push(value) },
  }
  return { stream, read: () => queue.length ? Promise.resolve(queue.shift()) : new Promise((resolve, reject) => { pending = { resolve, reject } }) }
}

test("viewer drains startup traffic and retains only the latest frame", async () => {
  const { stream, read } = fixture()
  const reader = startLoadViewerReader(stream, read)
  try {
    for (let index = 0; index < 200; index++) { stream.push(index); await yieldTurn() }
    assert.equal(await reader.takeFrame(), 199)
    const next = reader.takeFrame()
    stream.push(201)
    assert.equal(await next, 201)
  } finally { await reader.stop() }
})

test("slow-viewer pause stops reads, resumes, and cleanup unblocks paused reader", async () => {
  const { stream, read } = fixture()
  let reads = 0
  const reader = startLoadViewerReader(stream, () => { reads++; return read() })
  reader.pause()
  stream.push(1)
  await yieldTurn()
  await yieldTurn()
  assert.equal(reads, 1)
  reader.resume()
  stream.push(2)
  await yieldTurn()
  assert.ok(reads > 1)
  reader.pause()
  await reader.stop()
  await assert.rejects(reader.takeFrame(), /stopped/)
})

test("viewer stream failure remains a failed measurement", async () => {
  const reader = startLoadViewerReader({ async close() {} }, async () => { throw new Error("capacity exceeded") })
  await assert.rejects(reader.takeFrame(), /capacity exceeded/)
  await reader.stop()
})
