import { CommandFailure } from "./agent-command-table.js"

// Chunk size of a file transfer, on both ends. One reply is one indivisible relay message: at
// 64 KiB it is about 117 KB on the wire, a hundred milliseconds of an 8 Mbit/s link, where a
// 256 KiB chunk held it for half a second. Do not go much lower: the kernel queues a reply as bulk
// only above about 36 KiB of data; a smaller one goes on the priority lane, outside the per-client bound.
export const FILE_CHUNK_BYTES = 64 * 1024
// Requests in flight at the start, and the least ever: a transfer on a slow link stays here.
export const MIN_WINDOW = 4
// What one transfer may hold requested and not yet written, however fast or long the link: below the
// 32 requests a kernel admits at once from one connection.
export const MAX_IN_FLIGHT_BYTES = 30 * FILE_CHUNK_BYTES
// The window is the bandwidth-delay product in chunks, measured, plus this many for jitter.
const HEADROOM = 2
// A window-limited transfer measures only its own window: asking for twice the measured product lets it double per step.
const GAIN = 2
const RATE_SAMPLES = 16

// One request for `length` bytes at `offset`. It answers with `length` set to the bytes it moved,
// at least one, or throws.
type Request<T extends { length: number }> = (offset: number, length: number) => Promise<T>

// Estimates how many chunks fit in the link: the round trip of a small request, which the kernel
// answers ahead of bulk replies and so measures the link and not the transfer's own queue, times
// the rate the transfer is getting.
class WindowSizer {
  private readonly done: { at: number; bytes: number }[] = []
  private readonly most = MAX_IN_FLIGHT_BYTES / FILE_CHUNK_BYTES
  size = MIN_WINDOW
  private grewAt = Number.NEGATIVE_INFINITY

  constructor(private readonly roundTripMs: number) {}

  completed(now: number, bytes: number): void {
    this.done.push({ at: now, bytes })
    if (this.done.length > RATE_SAMPLES) this.done.shift()
    const first = this.done[0]!
    const spanMs = now - first.at
    if (spanMs <= 0) return
    const bytesPerMs = this.done.slice(1).reduce((sum, sample) => sum + sample.bytes, 0) / spanMs
    const wanted = Math.min(this.most, Math.max(MIN_WINDOW, Math.ceil(GAIN * bytesPerMs * this.roundTripMs / FILE_CHUNK_BYTES) + HEADROOM))
    if (wanted <= this.size) { this.size = wanted; return }
    // Replies that land together after a stall say how fast the burst was, not how fast the link is: a
    // larger window is earned by at most doubling per half round trip, and the next samples take it back.
    if (now - this.grewAt < this.roundTripMs / 2) return
    this.size = Math.min(wanted, 2 * this.size)
    this.grewAt = now
  }
}

type Issued<T extends { length: number }> = { offset: number; length: number; reply: Promise<T> }

// Moves [from, size) with several requests in flight and hands each answer to `accept` in file order,
// so whatever `accept` has been given is always a whole prefix of the file. A failed request or
// accept ends the transfer; requests still in flight are awaited first, so nothing outlives the call.
export async function transferWindowed<T extends { length: number }>(input: {
  size: number
  from: number
  roundTripMs: number
  now: () => number
  request: Request<T>
  accept: (answer: T) => Promise<void>
}): Promise<void> {
  const { size, now, request, accept } = input
  const sizer = new WindowSizer(Math.max(input.roundTripMs, 1))
  const issued: Issued<T>[] = []
  let next = input.from
  const settleAll = async () => { await Promise.allSettled(issued.map(item => item.reply)) }
  try {
    while (next < size || issued.length > 0) {
      while (next < size && issued.length < sizer.size) {
        const length = Math.min(FILE_CHUNK_BYTES, size - next)
        const reply = request(next, length)
        // The head's failure is raised below; a later reply that fails is dropped with the transfer.
        reply.catch(() => undefined)
        issued.push({ offset: next, length, reply })
        next += length
      }
      const head = issued.shift()!
      const answer = await head.reply
      if (answer.length > head.length) throw new CommandFailure("file_io_failed", `The kernel moved ${answer.length} bytes for a request of ${head.length} at ${head.offset}; run the same command again to continue`)
      sizer.completed(now(), answer.length)
      if (answer.length < head.length) {
        // The source gave less than asked: what was requested beyond it would leave a gap.
        await settleAll()
        issued.length = 0
        next = head.offset + answer.length
      }
      await accept(answer)
    }
  } finally { await settleAll() }
}
