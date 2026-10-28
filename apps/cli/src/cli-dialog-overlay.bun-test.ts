import assert from "node:assert/strict"
import test from "node:test"

import { BoxRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"

import { renderCliDialogOverlay } from "./cli-dialog-overlay.js"
import { createProjectEnvironmentReadController, projectEnvironmentPageSize, projectEnvironmentTextWidth } from "./project-environment-read-controller.js"
import { sessionBrowserFixture } from "./session-browser-card.test-fixture.js"

for (const [width, height, count] of [[80, 24, 16], [140, 48, 35], [140, 100, 76]] as const) {
  test(`MP-08/MP-10: Environment keeps every page row visible at ${width}x${height}`, async () => {
    assert.equal(projectEnvironmentPageSize(height), count)
    const harness = await createTestRenderer({ width, height, useThread: false })
    const overlayBox = new BoxRenderable(harness.renderer, { position: "absolute", left: 0, top: 0, width, height })
    harness.renderer.root.add(overlayBox)
    const lines = Array.from({ length: count }, (_, index) => `ROW_${String(index + 1).padStart(3, "0")} · Not checked`)
    try {
      renderCliDialogOverlay({ overlayBox, renderer: harness.renderer, dimensions: { width, height }, mode: "environment", onDismiss() {}, sessions: [], normalizeSessionBrowserIndex: () => 0, terminalPairing: null, terminalPairingQrLines: [], hotkeySections: [], environmentLines: lines })
      await harness.renderOnce()
      const frame = harness.captureCharFrame()
      for (const line of lines) assert(frame.includes(line), `page clipped ${line}`)
    } finally { harness.renderer.destroy() }
  })
}

for (const [width, height] of [[80, 24], [60, 24], [40, 24]] as const) {
  test(`MP-08/MP-10/MP-11: Environment explanation is fully visible at ${width}x${height}`, async () => {
    const harness = await createTestRenderer({ width, height, useThread: false })
    const overlayBox = new BoxRenderable(harness.renderer, { position: "absolute", left: 0, top: 0, width, height })
    harness.renderer.root.add(overlayBox)
    const snapshot={schema_version:1,local_project_id:"project",lineage:{project_id:"lineage",environment_id:"env"},revision:0,content_digest:"digest",parent_revision_digest:null,evidence_digest:null,project_requirements:[],folders:[],proposals:[],observations:[],operations:[],delivered_capabilities:{enabled_environment_operations:["get"],supported_schema:1}}
    const controller = createProjectEnvironmentReadController({ send: async () => ({ProjectEnvironment:{environment:snapshot}}), render() {}, pageSize: () => projectEnvironmentPageSize(height), textWidth: () => projectEnvironmentTextWidth(width) })
    try {
      await controller.open("project")
      const frames:string[]=[]
      for(let page=0;page<80;page++) {
        renderCliDialogOverlay({ overlayBox, renderer: harness.renderer, dimensions: { width, height }, mode: "environment", onDismiss() {}, sessions: [], normalizeSessionBrowserIndex: () => 0, terminalPairing: null, terminalPairingQrLines: [], hotkeySections: [], environmentLines: controller.visibleLines() })
        await harness.renderOnce()
        frames.push(harness.captureCharFrame().split("\n").map(row=>row.trim()).join(" ").replace(/\s+/g," "))
        controller.handleKey({name:"down"})
      }
      const seen=frames.join(" ")
      for (const phrase of ["An Environment lists", "cannot change them yet.", "Update Chariox on this machine to edit."]) assert(seen.includes(phrase), `unreachable ${phrase}`)
    } finally { harness.renderer.destroy() }
  })
}

// MP-08 / MP-10 / MP-11: transport failures get their own honest fixture.
test("Environment transport failure shows connection guidance and Refresh",async()=>{
  const controller=createProjectEnvironmentReadController({send:async()=>{throw new Error("not available")},render(){},pageSize:()=>16,textWidth:()=>72})
  await controller.open("project")
  assert.deepEqual(controller.visibleLines(),["An Environment lists what this project needs on this machine.","Environment unavailable. Check the kernel connection and refresh.","r · Refresh"])
  controller.close()
})

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
