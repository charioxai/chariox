import assert from "node:assert/strict"
import test from "node:test"

import { BoxRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"

import { renderCliDialogOverlay } from "./cli-dialog-overlay.js"
import { dialogPageSize, dialogTextWidth } from "./cli-dialog-panel-size.js"
import { createProjectEnvironmentController } from "./project-environment-controller.js"
import type { EnvironmentDialogView } from "./project-environment-projection.js"
import { sessionBrowserFixture } from "./session-browser-card.test-fixture.js"

const environmentRow = (text: string, focused = false) => ({ text, tone: "text" as const, focused, bold: false })

for (const [width, height, count] of [[80, 24, 16], [140, 48, 35], [140, 100, 76]] as const) {
  test(`Environment keeps every page row visible at ${width}x${height}`, async () => {
    assert.equal(dialogPageSize(height), count)
    const harness = await createTestRenderer({ width, height, useThread: false })
    const overlayBox = new BoxRenderable(harness.renderer, { position: "absolute", left: 0, top: 0, width, height })
    harness.renderer.root.add(overlayBox)
    const lines = Array.from({ length: count }, (_, index) => `ROW_${String(index + 1).padStart(3, "0")} · Present`)
    const environmentView: EnvironmentDialogView = { hint: "Esc close", rows: lines.map((line, index) => environmentRow(line, index === 0)), message: null }
    try {
      renderCliDialogOverlay({ overlayBox, renderer: harness.renderer, dimensions: { width, height }, mode: "environment", onDismiss() {}, sessions: [], normalizeSessionBrowserIndex: () => 0, terminalPairing: null, terminalPairingQrLines: [], hotkeySections: [], environmentView })
      await harness.renderOnce()
      const frame = harness.captureCharFrame()
      for (const line of lines) assert(frame.includes(line), `page clipped ${line}`)
    } finally { harness.renderer.destroy() }
  })
}

test("Environment shows the failure message under the rows", async () => {
  const [width, height] = [80, 24]
  const harness = await createTestRenderer({ width, height, useThread: false })
  const overlayBox = new BoxRenderable(harness.renderer, { position: "absolute", left: 0, top: 0, width, height })
  harness.renderer.root.add(overlayBox)
  const environmentView: EnvironmentDialogView = { hint: "Esc close", rows: [environmentRow("Kernel · Setup not run")], message: "Vault is locked" }
  try {
    renderCliDialogOverlay({ overlayBox, renderer: harness.renderer, dimensions: { width, height }, mode: "environment", onDismiss() {}, sessions: [], normalizeSessionBrowserIndex: () => 0, terminalPairing: null, terminalPairingQrLines: [], hotkeySections: [], environmentView })
    await harness.renderOnce()
    const frame = harness.captureCharFrame()
    assert(frame.includes("Environment"), frame)
    assert(frame.includes("Kernel · Setup not run"), frame)
    assert(frame.includes("Vault is locked"), frame)
  } finally { harness.renderer.destroy() }
})

for (const [width, height] of [[80, 24], [60, 24], [40, 24]] as const) {
  test(`Environment dialog from the controller fits at ${width}x${height}`, async () => {
    const harness = await createTestRenderer({ width, height, useThread: false })
    const overlayBox = new BoxRenderable(harness.renderer, { position: "absolute", left: 0, top: 0, width, height })
    harness.renderer.root.add(overlayBox)
    const controller = createProjectEnvironmentController({ otherKernels: async () => [], kernelId: async () => "kernel",
      client: () => ({ send: async () => ({ ProjectEnvironment: { environment: {
        projectId: "project-1", exists: true, revision: 1, setup: { script: "make setup" }, variables: "A=1",
        secrets: [{ name: "API_KEY", presence: "missing" }], files: [{ path: ".env", presence: "present", sizeBytes: 12 }], mcps: [], skills: [], proposals: [],
        changedAtMs: 1, machineLabel: "Kernel", checkoutPath: "/p", setupState: { state: "notRun" },
      } } }) }),
      render() {}, schedule: () => undefined, newOperationId: () => "op", now: () => 0,
      pageSize: () => dialogPageSize(height), textWidth: () => dialogTextWidth(width),
    })
    try {
      await controller.open("project-1")
      renderCliDialogOverlay({ overlayBox, renderer: harness.renderer, dimensions: { width, height }, mode: "environment", onDismiss() {}, sessions: [], normalizeSessionBrowserIndex: () => 0, terminalPairing: null, terminalPairingQrLines: [], hotkeySections: [], environmentView: controller.view() })
      await harness.renderOnce()
      const frame = harness.captureCharFrame()
      for (const phrase of ["Environment", "Setup not run", "make setup", "API_KEY", "Missing here", "Files outside Git"]) assert(frame.includes(phrase), `clipped ${phrase}\n${frame}`)
    } finally { harness.renderer.destroy() }
  })
}

test("session browser keeps nonzero ERROR visible at 80 columns", async () => {
  const harness = await createTestRenderer({ width: 80, height: 24, useThread: false })
  const overlayBox = new BoxRenderable(harness.renderer, {
    position: "absolute",
    left: 0,
    top: 0,
    width: 80,
    height: 24,
  })
  harness.renderer.root.add(overlayBox)
  try {
    renderCliDialogOverlay({
      overlayBox,
      renderer: harness.renderer,
      dimensions: { width: 80, height: 24 },
      mode: "session-browser",
      onDismiss() {},
      sessions: Array.from({ length: 6 }, (_, index) => ({
        ...sessionBrowserFixture,
        id: index === 5 ? sessionBrowserFixture.id : `session-${index + 1}`,
        alias: `workspace-${index + 1}`,
        activity: {
          agent_count: 3,
          working_agent_count: 0,
          active_prompt_count: 0,
          queued_prompt_count: 0,
          error_agent_count: index === 5 ? 1 : 0,
          unread_idle_agent_count: index === 5 ? 2 : 3,
        },
      })),
      normalizeSessionBrowserIndex: () => 5,
      sessionBrowserProject: {
        id: "project-1",
        owner_user_id: "owner",
        workspace_id: "/workspace",
        name: "Project-1",
        kind: "named",
        status: "active",
        created_at_ms: 1,
        updated_at_ms: 2,
        session_count: 6,
        joined_collaborator_count: 1,
        pending_collaboration_invite_count: 1,
      },
      terminalPairing: null,
      terminalPairingQrLines: [],
      hotkeySections: [],
    })
    await harness.renderOnce()
    const frame = harness.captureCharFrame()
    assert.match(frame, /18ca9569919075f8/)
    assert.match(frame, /1 ERROR/)
    assert.match(frame, /1 workflows/)
    assert.match(frame, /1 collaborators joined/)
    assert.match(frame, /5-6 of 6/)
  } finally {
    harness.renderer.destroy()
  }
})

test("the machine offer dialog renders read-only service details with a working close button", async () => {
  const harness = await createTestRenderer({ width: 100, height: 30, useThread: false })
  const overlayBox = new BoxRenderable(harness.renderer, {
    position: "absolute",
    left: 0,
    top: 0,
    width: 100,
    height: 30,
  })
  harness.renderer.root.add(overlayBox)
  let dismissed = 0
  try {
    renderCliDialogOverlay({
      overlayBox,
      renderer: harness.renderer,
      dimensions: { width: 100, height: 30 },
      mode: "managed-machine",
      onDismiss() { dismissed++ },
      sessions: [],
      normalizeSessionBrowserIndex: () => 0,
      terminalPairing: null,
      terminalPairingQrLines: [],
      hotkeySections: [],
      managedMachineRows: [{
        id: "machine-offer:type",
        title: "Machine type",
        value: "General purpose",
        titleWidth: 28,
        indent: 1,
        focused: false,
        selectable: false,
        scrollbar: "",
      }],
    })
    await harness.renderOnce()
    const frame = harness.captureCharFrame()
    assert.match(frame, /New machine/)
    assert.match(frame, /Machine type/)
    assert.match(frame, /General purpose/)
    assert.doesNotMatch(frame, /fields|values|Compute class|Chariox-managed/)
    const lines = frame.split("\n"), y = lines.findIndex(line => line.includes("×"))
    assert.ok(y >= 0, "the header includes a close button")
    const x = lines[y]!.indexOf("×")
    await harness.mockMouse.click(x, y, 2)
    assert.equal(dismissed, 0)
    await harness.mockMouse.click(x, y, 0)
    assert.equal(dismissed, 1)
  } finally {
    harness.renderer.destroy()
  }
})
