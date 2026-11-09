// A TCP proxy that shapes a link: a token bucket per direction, a one-way delay with jitter and a bounded
// queue. A full queue stops reading from the sender, so the sender's TCP window closes like on a real
// bottleneck (it does not buffer without limit). Test tool only.
//   node throttle-proxy.mjs --target 127.0.0.1:43130 --down-kbps 8000 --up-kbps 2000 --delay-ms 30 --jitter-ms 5
// "down" is target -> client, "up" is client -> target. It prints one JSON line with the listening port,
// and one JSON line with the byte counters at SIGTERM.
import { createServer, connect } from "node:net"
import { pathToFileURL } from "node:url"

const TICK_MS = 2
const WRITE_SLICE = 16 * 1024

// One direction of the link: chunks wait for their release time, then leave as the tokens allow.
class Lane {
  constructor({ kbps, delayMs, jitterMs, queueBytes }) {
    this.bytesPerSecond = kbps === 0 ? Infinity : kbps * 1000 / 8
    this.delayMs = delayMs
    this.jitterMs = jitterMs
    this.queueBytes = queueBytes
    this.burst = Math.max(WRITE_SLICE, this.bytesPerSecond * 0.02)
    this.tokens = this.burst
    this.refilledAt = performance.now()
    this.queue = []
    this.queued = 0
    this.lastRelease = 0
    this.sent = 0
    this.timer = null
  }

  attach(from, to) {
    this.from = from
    this.to = to
    from.on("data", chunk => {
      // Releases stay in order: jitter can delay a chunk but never let a later one overtake it.
      const release = Math.max(this.lastRelease, performance.now() + this.delayMs + Math.random() * this.jitterMs)
      this.lastRelease = release
      this.queue.push({ data: chunk, offset: 0, release })
      this.queued += chunk.length
      if (this.queued >= this.queueBytes) from.pause()
      this.pump()
    })
    from.on("end", () => { this.ended = true; this.pump() })
  }

  refill(now) {
    this.tokens = Math.min(this.burst, this.tokens + (now - this.refilledAt) / 1000 * this.bytesPerSecond)
    this.refilledAt = now
  }

  pump() {
    if (this.timer) return
    this.timer = setTimeout(() => { this.timer = null; this.drain() }, TICK_MS)
  }

  drain() {
    if (this.to.destroyed) return
    const now = performance.now()
    this.refill(now)
    while (this.queue.length && this.queue[0].release <= now && this.tokens >= 1 && !this.to.writableNeedDrain) {
      const item = this.queue[0]
      const take = Math.min(item.data.length - item.offset, WRITE_SLICE, Math.floor(this.tokens))
      this.to.write(item.data.subarray(item.offset, item.offset + take))
      item.offset += take
      this.tokens -= take
      this.sent += take
      this.queued -= take
      if (item.offset === item.data.length) this.queue.shift()
    }
    if (this.queued < this.queueBytes / 2) this.from.resume()
    if (this.queue.length) {
      if (this.to.writableNeedDrain) this.to.once("drain", () => this.pump())
      else this.pump()
    } else if (this.ended) this.to.end()
  }
}

export function createThrottleProxy({ host = "127.0.0.1", port = 0, targetHost, targetPort, downKbps, upKbps, delayMs = 0, jitterMs = 0, queueBytes = 256 * 1024 }) {
  const sockets = new Set()
  const lanes = { down: [], up: [] }
  const server = createServer(client => {
    const upstream = connect(targetPort, targetHost)
    const down = new Lane({ kbps: downKbps, delayMs, jitterMs, queueBytes })
    const up = new Lane({ kbps: upKbps, delayMs, jitterMs, queueBytes })
    lanes.down.push(down)
    lanes.up.push(up)
    for (const socket of [client, upstream]) {
      sockets.add(socket)
      socket.setNoDelay(true)
      socket.on("error", () => socket.destroy())
      socket.on("close", () => { sockets.delete(socket); const other = socket === client ? upstream : client; if (!other.destroyed) other.destroy() })
    }
    down.attach(upstream, client)
    up.attach(client, upstream)
  })
  return {
    server,
    listen: () => new Promise(resolve => server.listen(port, host, () => resolve(server.address().port))),
    bytes: () => ({ down: lanes.down.reduce((sum, lane) => sum + lane.sent, 0), up: lanes.up.reduce((sum, lane) => sum + lane.sent, 0) }),
    close: () => { for (const socket of sockets) socket.destroy(); return new Promise(resolve => server.close(() => resolve())) },
  }
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  const flags = new Map()
  for (let i = 2; i < process.argv.length; i += 2) flags.set(process.argv[i], process.argv[i + 1])
  const [targetHost, targetPort] = String(flags.get("--target")).split(":")
  const number = (key, fallback) => (flags.has(key) ? Number(flags.get(key)) : fallback)
  const proxy = createThrottleProxy({
    targetHost, targetPort: Number(targetPort), port: number("--listen", 0),
    downKbps: number("--down-kbps", 0), upKbps: number("--up-kbps", 0),
    delayMs: number("--delay-ms", 0), jitterMs: number("--jitter-ms", 0), queueBytes: number("--queue-bytes", 256 * 1024),
  })
  const listening = await proxy.listen()
  console.log(JSON.stringify({ listening }))
  process.on("SIGTERM", () => { console.log(JSON.stringify({ bytes: proxy.bytes() })); process.exit(0) })
}
