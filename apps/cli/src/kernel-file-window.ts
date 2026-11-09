import { CommandFailure } from "./agent-command-table.js"

// Chunk size of a file transfer, on both ends. A reply of 64 KiB is the smallest the kernel still
// queues as bulk (its encrypted payload passes 64 KiB), so a client's replies stay bounded
// there; one reply is about 117 KB on the wire, a hundred milliseconds of an 8 Mbit/s link, never the
// half megabyte a 256 KiB chunk made.
export const FILE_CHUNK_BYTES = 64 * 1024
// Requests in flight at the start, and the least ever: a transfer on a slow link stays here.
export const MIN_WINDOW = 4
// What one transfer may hold requested and not yet written, however fast or long the link.
export const MAX_IN_FLIGHT_BYTES = 4 * 1024 * 1024
// The window is the bandwidth-delay product in chunks, measured, plus this many for jitter.
const HEADROOM = 2
const RATE_SAMPLES = 16

// One request for `length` bytes at `offset`. It answers with at least one byte or throws.
type Read = (offset: number, length: number) => Promise<Buffer>

// Estimates how many chunks fit in the link: the round trip of a small request, which the kernel
// answers ahead of bulk replies and so measures the link and not the transfer's own queue, times
// the rate the transfer is getting.
class WindowSizer {
  private readonly done: { at: number; bytes: number }[] = []
  private readonly most = MAX_IN_FLIGHT_BYTES / FILE_CHUNK_BYTES
  size = MIN_WINDOW

  constructor(private readonly roundTripMs: number) {}

  completed(now: number, bytes: number): void {
    this.done.push({ at: now, bytes })
    if (this.done.length > RATE_SAMPLES) this.done.shift()
    const first = this.done[0]!
    const spanMs = now - first.at
    if (spanMs <= 0) return
    const bytesPerMs = this.done.slice(1).reduce((sum, sample) => sum + sample.bytes, 0) / spanMs
    this.size = Math.min(this.most, Math.max(MIN_WINDOW, Math.ceil(bytesPerMs * this.roundTripMs / FILE_CHUNK_BYTES) + HEADROOM))
  }
}

type Issued = { offset: number; length: number; reply: Promise<Buffer> }

// Downloads [from, size) with several reads in flight and hands the bytes to `write` in file order,
// so whatever `write` has been given is always a whole prefix of the file. A failed read or write
// ends the transfer; reads still in flight are awaited first, so nothing outlives the call.
export async function downloadWindowed(input: { size: number; from: number; roundTripMs: number; now: () => number; read: Read; write: (bytes: Buffer) => Promise<void> }): Promise<void> {
  const { size, now, read, write } = input
  const sizer = new WindowSizer(Math.max(input.roundTripMs, 1))
  const issued: Issued[] = []
  let next = input.from
  const settleAll = async () => { await Promise.allSettled(issued.map(item => item.reply)) }
  try {
    while (next < size || issued.length > 0) {
      while (next < size && issued.length < sizer.size) {
        const length = Math.min(FILE_CHUNK_BYTES, size - next)
        const reply = read(next, length)
        // The head's failure is raised below; a later reply that fails is dropped with the transfer.
        reply.catch(() => undefined)
        issued.push({ offset: next, length, reply })
        next += length
      }
      const head = issued.shift()!
      const bytes = await head.reply
      if (bytes.length > head.length) throw new CommandFailure("file_io_failed", `The kernel sent ${bytes.length} bytes for a request of ${head.length} at ${head.offset}; run the same command again to continue`)
      sizer.completed(now(), bytes.length)
      if (bytes.length < head.length) {
        // The source gave less than asked: what was requested beyond it would leave a gap.
        await settleAll()
        issued.length = 0
        next = head.offset + bytes.length
      }
      await write(bytes)
    }
  } finally { await settleAll() }
}
