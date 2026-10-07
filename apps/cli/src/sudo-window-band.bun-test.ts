import assert from "node:assert/strict"
import test from "node:test"
import { BoxRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import type { KernelSudoTurn } from "@chariox/kernel-client/kernel-types"
import { createSudoWindowBand } from "./sudo-window-band.js"

const window: KernelSudoTurn = {
  entry_id: "sudo:00aa", session_id: "session-1", agent_id: "agent-1", owner_user_id: "local", terminal_id: "t",
  prompt_id: "p", provider_run_id: "r", task_id: "sudo:00aa", duration_minutes: 60,
  expires_at_ms: Date.parse("2026-10-07T14:05:00Z"), revision: 2, warning_sent: true,
}

test("MP-08/MP-10/MP-11 A04 the sudo band shows the kernel deadline with Extend and Revoke", async () => {
  const harness = await createTestRenderer({ width: 120, height: 10, useThread: false })
  const box = new BoxRenderable(harness.renderer, { flexDirection: "column", visible: false })
  harness.renderer.root.add(box)
  const clicked: string[] = []
  const band = createSudoWindowBand(harness.renderer, {
    extend: (w) => clicked.push(`extend ${w.entry_id}@${w.revision}`),
    revoke: (w) => clicked.push(`revoke ${w.entry_id}`),
  })
  band.assign(box)
  try {
    band.render([window], Date.parse("2026-10-07T13:58:00Z"), () => "builder")
    await harness.renderOnce()
    const frame = harness.captureCharFrame()
    assert.match(frame, /sudo · builder · 7m left \(until 14:05 UTC\) · expiring soon · sudo:00aa/)
    assert.match(frame, /\[Extend\]/)
    assert.match(frame, /\[Revoke\]/)
    const row = frame.split("\n").findIndex((line) => line.includes("[Extend]"))
    await harness.mockMouse.click(frame.split("\n")[row]!.indexOf("[Extend]") + 1, row)
    await harness.mockMouse.click(frame.split("\n")[row]!.indexOf("[Revoke]") + 1, row)
    assert.deepEqual(clicked, ["extend sudo:00aa@2", "revoke sudo:00aa"])
    band.render([], Date.now(), () => "builder")
    await harness.renderOnce()
    assert.equal(box.visible, false)
  } finally { harness.renderer.destroy() }
})
