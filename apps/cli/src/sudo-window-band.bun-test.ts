import assert from "node:assert/strict"
import test from "node:test"
import { BoxRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import type { KernelSudoTurn } from "@chariox/kernel-client/kernel-types"
import { createSudoWindowBand } from "./sudo-window-band.js"
import { sudoWindowLine } from "./sudo-window-line.js"
import { renderCommandCenterOverlay } from "./command-center-renderer.js"

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

test("MP-08/MP-10/MP-11 A04 cached rendering restores visibility after layout mount", async () => {
  const harness = await createTestRenderer({ width: 120, height: 10, useThread: false })
  const box = new BoxRenderable(harness.renderer, { flexDirection: "column", visible: false })
  harness.renderer.root.add(box)
  const band = createSudoWindowBand(harness.renderer, { extend: () => {}, revoke: () => {} })
  band.assign(box)
  try {
    const now = Date.parse("2026-10-07T13:58:00Z")
    band.render([window], now, () => "builder")
    // The workspace's initial visible=false property is applied after its ref.
    box.visible = false
    band.render([window], now, () => "builder")
    await harness.renderOnce()
    assert.match(harness.captureCharFrame(), /sudo · builder .* expiring soon/)
  } finally { harness.renderer.destroy() }
})

test("MP-08/MP-10/MP-11 P3 identical text refreshes the Extend handler revision", async () => {
  const harness = await createTestRenderer({ width: 120, height: 10, useThread: false })
  const box = new BoxRenderable(harness.renderer, { flexDirection: "column" })
  harness.renderer.root.add(box)
  const clicked: number[] = []
  const band = createSudoWindowBand(harness.renderer, {
    extend: (w) => clicked.push(w.revision ?? 0), revoke: () => {},
  })
  band.assign(box)
  try {
    const now = Date.parse("2026-10-07T13:58:45Z")
    const updated = { ...window, revision: window.revision! + 1, expires_at_ms: window.expires_at_ms! + 30_000 }
    assert.equal(sudoWindowLine(window, now, "builder"), sudoWindowLine(updated, now, "builder"))
    band.render([window], now, () => "builder")
    await harness.renderOnce()
    const before = harness.captureCharFrame()
    band.render([updated], now, () => "builder")
    await harness.renderOnce()
    const frame = harness.captureCharFrame()
    assert.equal(frame, before)
    const row = frame.split("\n").findIndex((line) => line.includes("[Extend]"))
    await harness.mockMouse.click(frame.split("\n")[row]!.indexOf("[Extend]") + 1, row)
    assert.deepEqual(clicked, [updated.revision])
  } finally { harness.renderer.destroy() }
})

test("MP-08/MP-10/MP-11 live click seam: a closed command center cannot cover Extend", async () => {
  const harness = await createTestRenderer({ width: 120, height: 10, useThread: false })
  const box = new BoxRenderable(harness.renderer, { position: "absolute", top: 1, width: 120, height: 1 })
  harness.renderer.root.add(box)
  // The command center keeps its elevated z-index and footer offset when closed.
  const prompt = new BoxRenderable(harness.renderer, { position: "absolute", top: 2, width: 120, height: 3 })
  const overlay = new BoxRenderable(harness.renderer, { height: 1 })
  harness.renderer.root.add(prompt)
  prompt.add(overlay)
  const clicked: string[] = []
  const band = createSudoWindowBand(harness.renderer, {
    extend: (w) => clicked.push(w.entry_id), revoke: () => {},
  })
  band.assign(box)
  try {
    band.render([window], Date.parse("2026-10-07T13:58:00Z"), () => "builder")
    const options = { box: overlay, renderer: harness.renderer, items: [], selectedIndex: 0, visibleRowCount: 4, promptHeight: 1, overlayFootprint: 2 }
    renderCommandCenterOverlay({ ...options, open: true })
    assert.equal(overlay.visible, true)
    renderCommandCenterOverlay({ ...options, open: false })
    await harness.renderOnce()
    const frame = harness.captureCharFrame().split("\n")
    const row = frame.findIndex((line) => line.includes("[Extend]"))
    assert.ok(row >= 0)
    await harness.mockMouse.click(frame[row]!.indexOf("[Extend]") + 1, row)
    assert.deepEqual(clicked, [window.entry_id])
    assert.equal(overlay.visible, false)
  } finally { harness.renderer.destroy() }
})
