import assert from "node:assert/strict"
import { mkdtemp, rm } from "node:fs/promises"
import { createConnection, createServer } from "node:net"
import { once } from "node:events"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { test } from "node:test"
import {
  SLICE_DISK_QUOTA_FRAME_MAX_BYTES,
  SLICE_DISK_QUOTA_PROTOCOL_VERSION,
} from "./slice-disk-quota-contract.mjs"
import { requestSliceDiskQuota } from "./slice-disk-quota-client.mjs"
import { handleSliceDiskQuotaConnection } from "./slice-disk-quota-service.mjs"

const PROBE_REQUEST = Object.freeze({
  protocolVersion: SLICE_DISK_QUOTA_PROTOCOL_VERSION,
  operation: "probe",
})

async function createUnixPeer(context, onConnection) {
  const root = await mkdtemp(join(tmpdir(), "chariox-slice-quota-transport-"))
  const socketPath = join(root, "peer.sock")
  const connections = new Set()
  const server = createServer({ allowHalfOpen: true }, (socket) => {
    connections.add(socket)
    socket.once("close", () => connections.delete(socket))
    onConnection(socket)
  })
  server.listen(socketPath)
  await once(server, "listening")
  context.after(async () => {
    for (const socket of connections) socket.destroy()
    await new Promise((resolve) => server.close(resolve))
    await rm(root, { recursive: true, force: true })
  })
  return { server, socketPath }
}

function boundedSettlement(promise, timeoutMs = 200) {
  return new Promise((resolve) => {
    const timer = setTimeout(() => resolve("pending"), timeoutMs)
    promise.then(
      () => {
        clearTimeout(timer)
        resolve("resolved")
      },
      () => {
        clearTimeout(timer)
        resolve("rejected")
      },
    )
  })
}

function socketExchange(socketPath, onConnect) {
  return new Promise((resolve, reject) => {
    const socket = createConnection(socketPath)
    const chunks = []
    let peerError
    socket.on("data", (chunk) => chunks.push(Buffer.from(chunk)))
    socket.on("error", (error) => { peerError = error })
    socket.once("connect", () => {
      try {
        onConnect(socket)
      } catch (error) {
        socket.destroy()
        reject(error)
      }
    })
    socket.once("close", () => resolve({ data: Buffer.concat(chunks), peerError }))
  })
}

function quotaProbeLine() {
  return `${JSON.stringify(PROBE_REQUEST)}\n`
}

function createTestAllocator() {
  const calls = []
  return {
    calls,
    handle(request) {
      calls.push(request)
      return { operation: request.operation, handled: true }
    },
  }
}

async function createQuotaServicePeer(context, allocator, options = {}) {
  return createUnixPeer(context, (socket) => {
    handleSliceDiskQuotaConnection(socket, allocator, options)
  })
}

test("client rejects when the allocator closes after receiving a request without a response", async (context) => {
  let received = ""
  const { socketPath } = await createUnixPeer(context, (socket) => {
    socket.on("data", (chunk) => {
      received += chunk.toString("utf8")
      socket.end()
    })
  })

  const outcome = await boundedSettlement(
    requestSliceDiskQuota(PROBE_REQUEST, { socketPath, requestTimeoutMs: 80 }),
  )
  assert.equal(received, `${JSON.stringify(PROBE_REQUEST)}\n`)
  assert.equal(outcome, "rejected", "the client must settle promptly when the peer ends without a response")
})

test("client accepts one correctly framed response delivered in fragments", async (context) => {
  const response = `${JSON.stringify({
    protocolVersion: SLICE_DISK_QUOTA_PROTOCOL_VERSION,
    ok: true,
    result: { bounded: false },
  })}\n`
  const { socketPath } = await createUnixPeer(context, (socket) => {
    socket.once("data", () => {
      socket.write(response.slice(0, 5))
      setTimeout(() => socket.end(response.slice(5)), 5)
    })
  })

  assert.deepEqual(await requestSliceDiskQuota(PROBE_REQUEST, { socketPath, requestTimeoutMs: 300 }), { bounded: false })
})

test("client rejects a response with trailing bytes in a later socket fragment", async (context) => {
  const response = `${JSON.stringify({
    protocolVersion: SLICE_DISK_QUOTA_PROTOCOL_VERSION,
    ok: true,
    result: { bounded: false },
  })}\n`
  const { socketPath } = await createUnixPeer(context, (socket) => {
    socket.once("data", () => {
      socket.write(response)
      setTimeout(() => socket.end("extra"), 5)
    })
  })

  await assert.rejects(
    requestSliceDiskQuota(PROBE_REQUEST, { socketPath, requestTimeoutMs: 300 }),
    /extra data/,
  )
})

test("client rejects an incomplete response frame at EOF", async (context) => {
  const { socketPath } = await createUnixPeer(context, (socket) => {
    socket.once("data", () => socket.end('{"protocolVersion":1,"ok":true,"result":{}'))
  })

  await assert.rejects(
    requestSliceDiskQuota(PROBE_REQUEST, { socketPath, requestTimeoutMs: 300 }),
    /frame was complete/,
  )
})

test("client rejects malformed UTF-8 in a response frame", async (context) => {
  const { socketPath } = await createUnixPeer(context, (socket) => {
    socket.once("data", () => {
      socket.end(Buffer.concat([
        Buffer.from('{"protocolVersion":1,"ok":false,"error":"bad '),
        Buffer.from([0xc3, 0x28]),
        Buffer.from('"}\n'),
      ]))
    })
  })

  await assert.rejects(
    requestSliceDiskQuota(PROBE_REQUEST, { socketPath, requestTimeoutMs: 300 }),
    TypeError,
  )
})

test("client validates response envelope keys, protocol version, and error shape", async (context) => {
  const invalidEnvelopes = [
    ["extra response key", { protocolVersion: 1, ok: true, result: {}, extra: true }, /response is invalid/],
    ["missing result field", { protocolVersion: 1, ok: true }, /response is invalid/],
    ["wrong protocol version", { protocolVersion: 2, ok: true, result: {} }, /protocol version mismatch/],
    ["non-string error", { protocolVersion: 1, ok: false, error: 7 }, /response is invalid/],
    ["missing error field", { protocolVersion: 1, ok: false }, /response is invalid/],
  ]

  for (const [label, envelope, message] of invalidEnvelopes) {
    await context.test(label, async (subtest) => {
      const { socketPath } = await createUnixPeer(subtest, (socket) => {
        socket.once("data", () => socket.end(`${JSON.stringify(envelope)}\n`))
      })
      await assert.rejects(
        requestSliceDiskQuota(PROBE_REQUEST, { socketPath, requestTimeoutMs: 300 }),
        message,
      )
    })
  }
})

test("client enforces its absolute deadline while the peer keeps trickling response bytes", async (context) => {
  const { socketPath } = await createUnixPeer(context, (socket) => {
    socket.once("data", () => {
      socket.on("error", () => {})
      const drip = setInterval(() => socket.write("."), 20)
      socket.once("close", () => clearInterval(drip))
    })
  })

  await assert.rejects(
    requestSliceDiskQuota(PROBE_REQUEST, { socketPath, requestTimeoutMs: 180, inactivityTimeoutMs: 90 }),
    /absolute deadline/,
  )
})

test("client rejects an oversized response frame", async (context) => {
  const { socketPath } = await createUnixPeer(context, (socket) => {
    socket.once("data", () => socket.end(Buffer.alloc(SLICE_DISK_QUOTA_FRAME_MAX_BYTES + 1, 0x78)))
  })

  await assert.rejects(
    requestSliceDiskQuota(PROBE_REQUEST, { socketPath, requestTimeoutMs: 300 }),
    /too large/,
  )
})

test("client retains its inactivity timeout when the peer makes no response progress", async (context) => {
  const { socketPath } = await createUnixPeer(context, (socket) => socket.on("data", () => {}))

  await assert.rejects(
    requestSliceDiskQuota(PROBE_REQUEST, { socketPath, requestTimeoutMs: 300, inactivityTimeoutMs: 80 }),
    /request timed out/,
  )
})

test("client rejects a peer reset instead of leaving the request pending", async (context) => {
  const { socketPath } = await createUnixPeer(context, (socket) => {
    socket.once("data", () => {
      socket.once("error", () => {})
      socket.destroy(new Error("quota peer reset fixture"))
    })
  })

  const outcome = await boundedSettlement(
    requestSliceDiskQuota(PROBE_REQUEST, { socketPath, requestTimeoutMs: 300 }),
  )
  assert.equal(outcome, "rejected")
})

test("service handles one fragmented request exactly once", async (context) => {
  const allocator = createTestAllocator()
  const { socketPath } = await createQuotaServicePeer(context, allocator, { requestTimeoutMs: 300 })
  const line = quotaProbeLine()
  const exchange = socketExchange(socketPath, (socket) => {
    socket.write(line.slice(0, 12))
    setTimeout(() => socket.end(line.slice(12)), 5)
  })
  const { data } = await exchange

  assert.equal(allocator.calls.length, 1)
  assert.deepEqual(allocator.calls[0], PROBE_REQUEST)
  assert.deepEqual(JSON.parse(data.toString("utf8")), {
    protocolVersion: SLICE_DISK_QUOTA_PROTOCOL_VERSION,
    ok: true,
    result: { operation: "probe", handled: true },
  })
})

test("client and service handler round-trip a probe through the real Unix socket", async (context) => {
  const allocator = createTestAllocator()
  const { socketPath } = await createQuotaServicePeer(context, allocator, { requestTimeoutMs: 300 })

  assert.deepEqual(
    await requestSliceDiskQuota(PROBE_REQUEST, { socketPath, requestTimeoutMs: 300 }),
    { operation: "probe", handled: true },
  )
  assert.deepEqual(allocator.calls, [PROBE_REQUEST])
})

test("client receives the allocator error envelope from the real service handler", async (context) => {
  const calls = []
  const allocator = {
    handle(request) {
      calls.push(request)
      throw new Error("fixture allocator rejection")
    },
  }
  const { socketPath } = await createQuotaServicePeer(context, allocator, { requestTimeoutMs: 300 })

  await assert.rejects(
    requestSliceDiskQuota(PROBE_REQUEST, { socketPath, requestTimeoutMs: 300 }),
    /fixture allocator rejection/,
  )
  assert.deepEqual(calls, [PROBE_REQUEST])
})

test("service rejects malformed UTF-8 before allocator dispatch", async (context) => {
  const allocator = createTestAllocator()
  const { socketPath } = await createQuotaServicePeer(context, allocator, { requestTimeoutMs: 300 })
  const malformedRequest = Buffer.concat([
    Buffer.from('{"protocolVersion":1,"operation":"pro'),
    Buffer.from([0xc3, 0x28]),
    Buffer.from('be"}\n'),
  ])
  const { data } = await socketExchange(socketPath, (socket) => socket.end(malformedRequest))

  assert.equal(allocator.calls.length, 0)
  const response = JSON.parse(data.toString("utf8"))
  assert.equal(response.protocolVersion, SLICE_DISK_QUOTA_PROTOCOL_VERSION)
  assert.equal(response.ok, false)
  assert.match(response.error, /encoded data was not valid/i)
})

test("service closes an incomplete request frame without allocator dispatch", async (context) => {
  const allocator = createTestAllocator()
  const { socketPath } = await createQuotaServicePeer(context, allocator, { requestTimeoutMs: 300 })

  const { data } = await socketExchange(socketPath, (socket) => socket.end(quotaProbeLine().slice(0, -1)))
  assert.equal(data.length, 0)
  assert.equal(allocator.calls.length, 0)
})

test("service closes an oversized request without an unhandled socket error", async (context) => {
  const allocator = createTestAllocator()
  const { socketPath } = await createQuotaServicePeer(context, allocator, { requestTimeoutMs: 300 })
  const { data } = await socketExchange(socketPath, (socket) => {
    socket.end(Buffer.alloc(SLICE_DISK_QUOTA_FRAME_MAX_BYTES + 1, 0x78))
  })

  assert.equal(data.length, 0)
  assert.equal(allocator.calls.length, 0)
})

test("service rejects multiple frames before invoking the allocator", async (context) => {
  const allocator = createTestAllocator()
  const { socketPath } = await createQuotaServicePeer(context, allocator, { requestTimeoutMs: 300 })
  const exchange = socketExchange(socketPath, (socket) => {
    socket.write(quotaProbeLine())
    setTimeout(() => socket.end(quotaProbeLine()), 5)
  })
  const { data } = await exchange

  assert.equal(data.length, 0)
  assert.equal(allocator.calls.length, 0)
})

test("service ignores a peer error before request EOF without dispatching", async (context) => {
  const allocator = createTestAllocator()
  const { socketPath } = await createQuotaServicePeer(context, allocator, {
    requestTimeoutMs: 1_000,
    inactivityTimeoutMs: 900,
  })
  const startedAt = Date.now()
  const exchange = socketExchange(socketPath, (socket) => {
    socket.write(quotaProbeLine().slice(0, 8))
    setTimeout(() => socket.destroy(new Error("quota peer reset fixture")), 5)
  })
  const { data } = await exchange

  assert.equal(data.length, 0)
  assert.equal(allocator.calls.length, 0)
  assert.ok(Date.now() - startedAt < 500, "peer disconnect should settle before the request timers")
})

test("service retains its inactivity timeout for a stalled partial request", async (context) => {
  const allocator = createTestAllocator()
  const { socketPath } = await createQuotaServicePeer(context, allocator, {
    requestTimeoutMs: 300,
    inactivityTimeoutMs: 80,
  })
  const { data } = await socketExchange(socketPath, (socket) => socket.write(quotaProbeLine().slice(0, 8)))

  assert.equal(data.length, 0)
  assert.equal(allocator.calls.length, 0)
})

test("service enforces an absolute request deadline despite peer trickle", async (context) => {
  const allocator = createTestAllocator()
  const { socketPath } = await createQuotaServicePeer(context, allocator, {
    requestTimeoutMs: 180,
    inactivityTimeoutMs: 90,
  })
  const startedAt = Date.now()
  const exchange = socketExchange(socketPath, (socket) => {
    const drip = setInterval(() => socket.write("."), 20)
    socket.once("close", () => clearInterval(drip))
  })
  const { data } = await exchange

  assert.equal(data.length, 0)
  assert.equal(allocator.calls.length, 0)
  assert.ok(Date.now() - startedAt < 500, "the total request deadline should remain bounded")
})
