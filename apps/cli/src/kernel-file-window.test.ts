import assert from "node:assert/strict"
import test from "node:test"
import { transferWindowed, FILE_CHUNK_BYTES, MAX_IN_FLIGHT_BYTES, MIN_WINDOW } from "./kernel-file-window.js"

// Virtual time: a timer fires only once every promise that can make progress has, so a run is
// deterministic and a 100 MiB transfer at 8 Mbit/s takes milliseconds.
function simulation() {
  let now = 0
  const timers: { at: number; fire: () => void }[] = []
  const sleep = (ms: number) => new Promise<void>(resolve => timers.push({ at: now + Math.max(0, ms), fire: resolve }))
  async function run<T>(work: Promise<T>): Promise<T> {
    let settled = false
    const finished = work.then(() => { settled = true }, () => { settled = true })
    while (!settled) {
      for (let turn = 0; turn < 8 && !settled; turn++) await new Promise<void>(resolve => setImmediate(resolve))
      if (settled) break
      timers.sort((a, b) => a.at - b.at)
      const next = timers.shift()
      assert.ok(next, "the transfer is waiting on nothing")
      now = next.at
      next.fire()
    }
    await finished
    return work
  }
  return { now: () => now, sleep, run }
}

// A link of `bytesPerSecond` that carries replies one after another, `rttMs` away.
function link(sim: ReturnType<typeof simulation>, bytesPerSecond: number, rttMs: number) {
  let free = 0
  return async (bytes: number) => {
    const sent = Math.max(sim.now() + rttMs / 2, free) + bytes / bytesPerSecond * 1000
    free = sent
    await sim.sleep(sent + rttMs / 2 - sim.now())
  }
}

function source(size: number): Buffer {
  const bytes = Buffer.alloc(size)
  for (let index = 0; index < size; index++) bytes[index] = (index * 31 + (index >> 8)) & 0xff
  return bytes
}

const MEGABIT = 125_000
const MIB = 1024 * 1024

// Reads from `bytes` over `carry`, and counts what is asked at once and what waits to be written.
function remote(sim: ReturnType<typeof simulation>, carry: (bytes: number) => Promise<void>, bytes: Buffer) {
  const stats = { inFlight: 0, peakInFlight: 0, requests: [] as number[], unwritten: 0, peakUnwritten: 0 }
  return {
    stats,
    request: async (offset: number, length: number) => {
      stats.requests.push(offset)
      stats.inFlight++
      stats.unwritten += length
      stats.peakInFlight = Math.max(stats.peakInFlight, stats.inFlight)
      stats.peakUnwritten = Math.max(stats.peakUnwritten, stats.unwritten)
      try { await carry(length) } finally { stats.inFlight-- }
      return bytes.subarray(offset, offset + length)
    },
    written: (length: number) => { stats.unwritten -= length },
  }
}

async function pull(sim: ReturnType<typeof simulation>, bytes: Buffer, carry: (bytes: number) => Promise<void>, from = 0, roundTripMs = 60) {
  const kernel = remote(sim, carry, bytes)
  const received: Buffer[] = []
  const started = sim.now()
  await sim.run(transferWindowed({
    size: bytes.length,
    from,
    roundTripMs,
    now: sim.now,
    request: kernel.request,
    accept: async chunk => { received.push(Buffer.from(chunk)); kernel.written(chunk.length) },
  }))
  return { kernel, received: Buffer.concat(received), elapsedMs: sim.now() - started }
}

const linkUse = (bytes: number, elapsedMs: number, bytesPerSecond: number) => bytes / (elapsedMs / 1000) / bytesPerSecond

test("one request at a time leaves an 8 Mbit/s link with 60 ms of round trip about half idle", async () => {
  const sim = simulation()
  const carry = link(sim, 8 * MEGABIT, 60)
  const size = 20 * MIB
  await sim.run((async () => {
    for (let offset = 0; offset < size; offset += FILE_CHUNK_BYTES) await carry(FILE_CHUNK_BYTES)
  })())
  assert.ok(linkUse(size, sim.now(), 8 * MEGABIT) < 0.6)
})

test("a window of requests keeps an 8 Mbit/s link with 60 ms of round trip at 90 percent or more", async () => {
  const sim = simulation()
  const bytes = source(100 * MIB)
  const { received, elapsedMs } = await pull(sim, bytes, link(sim, 8 * MEGABIT, 60))
  assert.ok(received.equals(bytes))
  const use = linkUse(bytes.length, elapsedMs, 8 * MEGABIT)
  assert.ok(use >= 0.9, `link use ${use}`)
})

test("a faster link and a longer round trip are filled too, the window growing with the bandwidth-delay product", async () => {
  for (const [mbit, rtt] of [[50, 60], [100, 120]] as const) {
    const sim = simulation()
    const bytes = source(64 * MIB)
    const { kernel, received, elapsedMs } = await pull(sim, bytes, link(sim, mbit * MEGABIT, rtt), 0, rtt)
    assert.ok(received.equals(bytes))
    const use = linkUse(bytes.length, elapsedMs, mbit * MEGABIT)
    assert.ok(use >= 0.9, `${mbit} Mbit/s ${rtt} ms: link use ${use}`)
    assert.ok(kernel.stats.peakInFlight > MIN_WINDOW, `${mbit} Mbit/s ${rtt} ms: the window stayed at ${kernel.stats.peakInFlight}`)
  }
})

test("a slow link keeps the window at its floor so replies do not queue behind each other", async () => {
  const sim = simulation()
  const bytes = source(2 * MIB)
  const { kernel } = await pull(sim, bytes, link(sim, 600 * 1000 / 8, 60))
  assert.equal(kernel.stats.peakInFlight, MIN_WINDOW)
})

test("replies that arrive in a burst on a slow link do not open the window", async () => {
  const sim = simulation()
  const slow = link(sim, 600 * 1000 / 8, 60)
  let carried = 0
  // The first replies of a transfer land a millisecond apart after a stall: a rate measured over
  // that gap is the speed of the burst, not of the link.
  const carry = async (bytes: number) => {
    const turn = carried++
    if (turn < MIN_WINDOW) return sim.sleep(500 + turn)
    return slow(bytes)
  }
  const { kernel } = await pull(sim, source(2 * MIB), carry)
  assert.ok(kernel.stats.peakInFlight <= 2 * MIN_WINDOW, `${kernel.stats.peakInFlight} requests were in flight at once on a 600 kbit/s link`)
})

test("what waits to be written never passes the memory bound, however long and fast the link", async () => {
  const sim = simulation()
  const bytes = source(256 * MIB)
  const { kernel, received } = await pull(sim, bytes, link(sim, 1000 * MEGABIT, 200), 0, 200)
  assert.ok(received.equals(bytes))
  assert.ok(kernel.stats.peakUnwritten <= MAX_IN_FLIGHT_BYTES, `${kernel.stats.peakUnwritten} bytes`)
  assert.ok(kernel.stats.peakInFlight * FILE_CHUNK_BYTES <= MAX_IN_FLIGHT_BYTES)
  assert.ok(MAX_IN_FLIGHT_BYTES < 8 * MIB, "the bound is bytes, not file size")
})

test("chunks are written in file order and the file is intact when replies arrive in another order", async () => {
  const sim = simulation()
  const bytes = source(5 * MIB + 123)
  let state = 12345
  const jitter = async () => {
    state = (state * 1103515245 + 12345) & 0x7fffffff
    await sim.sleep(10 + state % 200)
  }
  const { received, kernel } = await pull(sim, bytes, jitter)
  assert.ok(received.equals(bytes))
  assert.ok(kernel.stats.peakInFlight >= MIN_WINDOW)
})

test("a download starts at the resume offset and asks for exactly what is missing", async () => {
  const sim = simulation()
  const bytes = source(3 * FILE_CHUNK_BYTES + 77)
  const from = FILE_CHUNK_BYTES + 5
  const { received, kernel } = await pull(sim, bytes, link(sim, 8 * MEGABIT, 60), from)
  assert.ok(received.equals(bytes.subarray(from)))
  assert.deepEqual(kernel.stats.requests, [from, from + FILE_CHUNK_BYTES, from + 2 * FILE_CHUNK_BYTES])
})

test("a short reply is continued where it stopped, with no gap and no repeat", async () => {
  const sim = simulation()
  const bytes = source(10 * FILE_CHUNK_BYTES)
  const received: Buffer[] = []
  const asked: [number, number][] = []
  const carry = link(sim, 8 * MEGABIT, 60)
  await sim.run(transferWindowed({
    size: bytes.length,
    from: 0,
    roundTripMs: 60,
    now: sim.now,
    request: async (offset, length) => {
      asked.push([offset, length])
      await carry(length)
      return bytes.subarray(offset, offset + (offset === 2 * FILE_CHUNK_BYTES ? length / 2 : length))
    },
    accept: async chunk => { received.push(Buffer.from(chunk)) },
  }))
  assert.ok(Buffer.concat(received).equals(bytes))
  assert.ok(asked.some(([offset]) => offset === 2 * FILE_CHUNK_BYTES + FILE_CHUNK_BYTES / 2))
})

test("a reply that fails mid-window stops the transfer after a whole prefix of the file and asks for nothing more", async () => {
  const sim = simulation()
  const bytes = source(40 * FILE_CHUNK_BYTES)
  const written: Buffer[] = []
  const carry = link(sim, 8 * MEGABIT, 60)
  let failed = false
  let askedAfterFailure = 0
  const unhandled: unknown[] = []
  const onUnhandled = (reason: unknown) => { unhandled.push(reason) }
  process.on("unhandledRejection", onUnhandled)
  try {
    await assert.rejects(sim.run(transferWindowed({
      size: bytes.length,
      from: 0,
      roundTripMs: 60,
      now: sim.now,
      request: async (offset, length) => {
        if (failed) askedAfterFailure++
        await carry(length)
        if (offset === 5 * FILE_CHUNK_BYTES) { failed = true; throw new Error("the kernel went away") }
        if (offset > 5 * FILE_CHUNK_BYTES) throw new Error("later request cut off too")
        return bytes.subarray(offset, offset + length)
      },
      accept: async chunk => { written.push(Buffer.from(chunk)) },
    })), /the kernel went away/)
    await new Promise<void>(resolve => setImmediate(resolve))
  } finally { process.off("unhandledRejection", onUnhandled) }
  const prefix = Buffer.concat(written)
  assert.ok(prefix.equals(bytes.subarray(0, prefix.length)))
  assert.equal(prefix.length, 5 * FILE_CHUNK_BYTES)
  assert.equal(askedAfterFailure, 0)
  assert.deepEqual(unhandled, [])
})

test("a failing write stops the transfer without leaving requests unanswered", async () => {
  const sim = simulation()
  const bytes = source(20 * FILE_CHUNK_BYTES)
  const carry = link(sim, 8 * MEGABIT, 60)
  let writes = 0
  await assert.rejects(sim.run(transferWindowed({
    size: bytes.length,
    from: 0,
    roundTripMs: 60,
    now: sim.now,
    request: async (offset, length) => { await carry(length); return bytes.subarray(offset, offset + length) },
    accept: async () => { if (++writes === 3) throw new Error("disk full") },
  })), /disk full/)
  assert.equal(writes, 3)
})

test("three transfers on one link share it: none finishes long before or after the others", async () => {
  const sim = simulation()
  const carry = link(sim, 8 * MEGABIT, 60)
  const bytes = source(12 * MIB)
  const finished: number[] = []
  const one = async () => {
    await transferWindowed({ size: bytes.length, from: 0, roundTripMs: 60, now: sim.now, request: async (offset, length) => { await carry(length); return bytes.subarray(offset, offset + length) }, accept: async () => undefined })
    finished.push(sim.now())
  }
  await sim.run(Promise.all([one(), one(), one()]))
  const [first, , last] = finished
  assert.ok(last! - first! < 0.1 * last!, `finished at ${finished.join(", ")}`)
  assert.ok(linkUse(3 * bytes.length, sim.now(), 8 * MEGABIT) >= 0.9)
})
