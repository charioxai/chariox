import assert from "node:assert/strict"
import test from "node:test"
import { createKernelConnectionDisplay } from "./kernel-connection-display.js"

test("MP-08/MP-10 only sustained loss escalates shared chrome; duplicate closes keep the deadline", () => {
  const statuses: string[] = []
  const timers = new Map<number, () => void>()
  let next = 0
  const display = createKernelConnectionDisplay({
    setStatusLine: value => statuses.push(value), updateSessionChrome: () => {}, connectedStatusLine: "Connected",
    timer: { setTimeout: (callback, delay) => { assert.equal(delay, 10_000); timers.set(++next, callback); return next as unknown as ReturnType<typeof setTimeout> },
      clearTimeout: id => { timers.delete(id as unknown as number) } },
  })
  display.closed()
  display.closed()
  assert.equal(timers.size, 1)
  assert.deepEqual(statuses, ["Reconnecting to the Chariox kernel…"])
  display.resumed()
  assert.equal(timers.size, 0)
  assert.equal(statuses.at(-1), "Connected")
  display.closed()
  timers.get(next)!()
  assert.equal(statuses.at(-1), "Disconnected from the Chariox kernel · showing last known data")
  display.dispose()
})
