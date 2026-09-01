import assert from "node:assert/strict"
import test from "node:test"

import { BoxRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"

import { renderCliDialogOverlay } from "./cli-dialog-overlay.js"
import { sessionBrowserFixture } from "./session-browser-card.test-fixture.js"

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

test("managed-machine dialog renders deployment fields outside the session form", async () => {
  const harness = await createTestRenderer({ width: 100, height: 30, useThread: false })
  const overlayBox = new BoxRenderable(harness.renderer, {
    position: "absolute",
    left: 0,
    top: 0,
    width: 100,
    height: 30,
  })
  harness.renderer.root.add(overlayBox)
  try {
    renderCliDialogOverlay({
      overlayBox,
      renderer: harness.renderer,
      dimensions: { width: 100, height: 30 },
      mode: "managed-machine",
      onDismiss() {},
      sessions: [],
      normalizeSessionBrowserIndex: () => 0,
      terminalPairing: null,
      terminalPairingQrLines: [],
      hotkeySections: [],
      managedMachineRows: [{
        id: "managed-compute",
        title: "Compute class",
        value: "agent-small",
        titleWidth: 28,
        indent: 1,
        focused: true,
        selectable: true,
        scrollbar: "",
      }],
    })
    await harness.renderOnce()
    const frame = harness.captureCharFrame()
    assert.match(frame, /New Chariox-managed machine/)
    assert.match(frame, /Compute class/)
    assert.match(frame, /agent-small/)
  } finally {
    harness.renderer.destroy()
  }
})
