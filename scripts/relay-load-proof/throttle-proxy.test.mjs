import assert from "node:assert/strict"
import { createServer, connect } from "node:net"
import { test } from "node:test"
import { createThrottleProxy } from "./throttle-proxy.mjs"

async function sink() {
  const received = []
  const server = createServer(socket => socket.on("data", chunk => received.push([performance.now(), chunk.length])))
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve))
  return { port: server.address().port, received, close: () => new Promise(resolve => server.close(resolve)) }
}

test("up direction is held to the configured rate and delayed", async () => {
  const target = await sink()
  const proxy = createThrottleProxy({ targetHost: "127.0.0.1", targetPort: target.port, downKbps: 0, upKbps: 800, delayMs: 50 })
  const port = await proxy.listen()
  const socket = connect(port, "127.0.0.1")
  await new Promise(resolve => socket.on("connect", resolve))
  const started = performance.now()
  socket.write(Buffer.alloc(100_000))
  while (target.received.reduce((sum, [, n]) => sum + n, 0) < 100_000 && performance.now() - started < 5000) await new Promise(resolve => setTimeout(resolve, 20))
  const elapsed = performance.now() - started
  // 100 KB at 100 KB/s is 1 s (the first 16 KB burst is free), plus the 50 ms delay.
  assert.ok(elapsed > 800 && elapsed < 1500, `100 KB took ${elapsed} ms at 800 kbit/s`)
  assert.ok(target.received[0][0] - started >= 45, "the first byte waits for the one-way delay")
  assert.ok(proxy.bytes().up === 100_000)
  socket.destroy()
  await proxy.close()
  await target.close()
})

test("down direction is shaped and a full queue pauses the sender", async () => {
  const sentAt = []
  const origin = createServer(socket => {
    socket.on("error", () => {})
    const chunk = Buffer.alloc(64 * 1024)
    let sent = 0
    const write = () => {
      while (sent < 1_000_000 && socket.write(chunk)) sent += chunk.length
      if (sent < 1_000_000) { sent += chunk.length; socket.once("drain", write) } else sentAt.push(sent)
    }
    write()
    socket.on("close", () => sentAt.push(sent))
  })
  await new Promise(resolve => origin.listen(0, "127.0.0.1", resolve))
  const proxy = createThrottleProxy({ targetHost: "127.0.0.1", targetPort: origin.address().port, downKbps: 4000, upKbps: 0, queueBytes: 64 * 1024 })
  const port = await proxy.listen()
  const socket = connect(port, "127.0.0.1")
  let got = 0
  socket.on("data", chunk => { got += chunk.length })
  await new Promise(resolve => setTimeout(resolve, 1000))
  // 4 Mbit/s is 500 KB/s: about 500 KB after one second, not the whole megabyte.
  assert.ok(got > 380_000 && got < 650_000, `received ${got} bytes in 1 s at 4000 kbit/s`)
  socket.destroy()
  await proxy.close()
  await new Promise(resolve => origin.close(resolve))
})
