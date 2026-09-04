import assert from "node:assert/strict"
import test from "node:test"
import { captureRoomStreamerDiagnostics } from "./room-streamer-diagnostics.mjs"

test("streamer failure probe is bounded, read-only and scoped to the exact owned slice", async () => {
  const name = "chariox-slice-room-pointer-123-2026-09-04T00-00-00-000Z"
  const expected = { recorded: true, owned: false, healthy: false,
    recordedProcess: { exists: false }, cgroup: { "memory.events": { oom_kill: 1 }, "memory.peak": 2147483648 } }
  const result = await captureRoomStreamerDiagnostics(name, async (command, args, timeout) => {
    assert.equal(command, "docker")
    assert.deepEqual(args.slice(0, -1), ["exec", "-u", "slice", name, "timeout", "--kill-after=1s", "5s",
      "/opt/chariox-selkies/bin/python", "-c"])
    assert.equal(timeout, 8000)
    const probe = args.at(-1)
    assert.match(probe, /memory\.events/)
    assert.match(probe, /oom_kill/)
    assert.doesNotMatch(probe, /cmdline|environ|master_token/)
    return { code: 0, stdout: JSON.stringify(expected) }
  })
  assert.deepEqual(result, { status: "captured", ...expected })
  await assert.rejects(captureRoomStreamerDiagnostics("unrelated-container", () => {
    throw new Error("must not execute")
  }), /drill-owned slice/)
})

test("unavailable or malformed streamer diagnostics do not pretend there were no OOMs", async () => {
  const name = "chariox-slice-room-pointer-123-test"
  assert.deepEqual(await captureRoomStreamerDiagnostics(name, async () => ({ code: 124 })),
    { status: "unavailable", exitCode: 124 })
  assert.deepEqual(await captureRoomStreamerDiagnostics(name, async () => ({ code: 0, stdout: "not-json" })),
    { status: "unavailable", reason: "invalid-diagnostic-json" })
})
