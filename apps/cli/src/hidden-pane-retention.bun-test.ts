import assert from "node:assert/strict"
import test from "node:test"
import { once } from "node:events"
import { setImmediate as nextTick } from "node:timers/promises"
import { BoxRenderable, Renderable, ScrollBoxRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import { WebSocketServer } from "ws"
import { LocalIpcClient } from "./ipc.js"
import { syncAuxiliaryPane } from "./response-layout-render.js"
import { buildEmptyTranscriptRenderable } from "./workspace-renderables.js"
import { createKernelApprovalRenderer } from "./kernel-approval-renderer.js"
import { renderAgentInteractionStrips } from "./interaction-strip-renderer.js"
import type { RuntimeInteraction } from "./cli-types.js"

// The pinned OpenTUI patch fixes a private queue. Mounted-child counts alone
// miss this leak: removed trees remain queued while an ancestor is hidden.
function pendingLayoutChildren(renderable: Renderable): number {
  return (renderable as unknown as { _shouldUpdateBefore: Set<Renderable> })._shouldUpdateBefore.size
}

async function settledHeap(): Promise<number> {
  for (let attempt = 0; attempt < 3; attempt += 1) {
    await nextTick()
    const runtime = globalThis as unknown as { Bun: { gc(force: boolean): void } }
    runtime.Bun.gc(true)
  }
  return process.memoryUsage().heapUsed
}

function replaceEmptyPane(renderer: Parameters<typeof buildEmptyTranscriptRenderable>[0], scrollbox: ScrollBoxRenderable) {
  syncAuxiliaryPane({
    scrollbox, nextAgentId: null, currentAgentId: null, splitMode: true,
    clearAuxiliaryAgentPane() {}, unregisterAgentScrollbox() {}, assignCurrentAgentId() {},
    registerAgentScrollbox() {}, rebuildAuxiliaryAgentPane() {},
    buildEmptyTranscriptRenderable: () => buildEmptyTranscriptRenderable(renderer),
  })
}

test("hidden auxiliary pane releases removed trees across 4,000 updates", async () => {
  const harness = await createTestRenderer({ width: 120, height: 40 })
  const initialCount = Renderable.renderablesByNumber.size
  const pane = new ScrollBoxRenderable(harness.renderer, { visible: false, width: 60, height: 20 })
  harness.renderer.root.add(pane)
  try {
    for (let index = 0; index < 500; index += 1) replaceEmptyPane(harness.renderer, pane)
    await harness.renderOnce()
    const before = await settledHeap()
    for (let index = 0; index < 4_000; index += 1) {
      replaceEmptyPane(harness.renderer, pane)
      if (index % 100 === 0) await harness.renderOnce()
    }
    await harness.renderOnce()
    assert.equal(pane.getChildren().length, 1)
    assert.equal(pendingLayoutChildren(pane.content), 1)
    assert.ok(await settledHeap() - before < 8 * 1024 * 1024, "discarded pane trees grew the settled heap")
    const child = pane.getChildren()[0]!
    pane.remove(child.id)
    child.destroyRecursively()
    assert.equal(pendingLayoutChildren(pane.content), 0)
    pane.destroyRecursively()
    assert.equal(pendingLayoutChildren(pane.content), 0)
    assert.equal(Renderable.renderablesByNumber.size, initialCount)
  } finally {
    harness.renderer.destroy()
  }
})

test("hidden interaction strips release 1,000 appearing and resolved cards", async () => {
  const harness = await createTestRenderer({ width: 120, height: 40 })
  const box = new BoxRenderable(harness.renderer, { visible: false })
  const hiddenParent = new BoxRenderable(harness.renderer, { visible: false })
  hiddenParent.add(box)
  harness.renderer.root.add(hiddenParent)
  let interaction: RuntimeInteraction | null = null
  const render = () => renderAgentInteractionStrips({
    renderer: harness.renderer, primaryBox: box, auxiliaryBoxes: [], visibleAgents: [],
    maxAgentsPerScreen: 1, focusedAgentId: null, activeInteractionForAgent: () => interaction,
    selectedChoiceIndex: () => 0, setSelectedChoiceIndex() {}, customReply: () => "", customEditing: () => false,
    queuedPromptStripItemsForAgent: () => [], selectedQueuedPromptIndexForAgent: () => -1,
    onQueuedPromptAction() {},
  })
  try {
    for (let index = 0; index < 1_000; index += 1) {
      interaction = { id: `card-${index}`, agent_id: "agent", kind: "permission", level: "warning",
        message: "Allow fixture install, uninstall or export?", requested_at_ms: index,
        choices: [{ id: "allow", label: "Allow", reply: "allow" }] }
      render()
      interaction = null
      render()
      if (index % 50 === 0) await harness.renderOnce()
    }
    assert.equal(box.getChildren().length, 0)
    assert.equal(pendingLayoutChildren(box), 0)
  } finally {
    harness.renderer.destroy()
  }
})

test("64 event-stream reconnects and 4,096 hidden-pane resync updates remain bounded", { timeout: 30_000 }, async () => {
  const harness = await createTestRenderer({ width: 120, height: 40 })
  const pane = new ScrollBoxRenderable(harness.renderer, { visible: false })
  harness.renderer.root.add(pane)
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await once(server, "listening")
  const address = server.address()
  assert.ok(address && typeof address !== "string")
  const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, { reconnectJitterMs: 0 })
  let subscriptions = 0
  let resumed = 0
  let onResume: (() => void) | undefined
  server.on("connection", (socket) => socket.on("message", (data) => {
    const frame = JSON.parse(String(data))
    subscriptions += 1
    socket.send(JSON.stringify({ type: "response", request_id: frame.request_id, response: { ok: true }, error: null }))
  }))
  const dispose = client.onKernelEvent((event) => {
    if (event.event !== "transport_resumed") return
    resumed += 1
    for (let update = 0; update < 64; update += 1) replaceEmptyPane(harness.renderer, pane)
    onResume?.()
  })
  try {
    await client.subscribeToKernelEvents("session-fixture", "attachment-fixture")
    const before = await settledHeap()
    for (let index = 0; index < 64; index += 1) {
      const ready = new Promise<void>((resolve) => { onResume = resolve })
      await client.restartKernelEventStream()
      await ready
      assert.equal(pendingLayoutChildren(pane.content), 1)
      assert.equal(server.clients.size, 1)
    }
    assert.equal(resumed, 64)
    assert.equal(subscriptions, 65)
    assert.ok(await settledHeap() - before < 8 * 1024 * 1024, "reconnect resyncs retained discarded pane trees")
  } finally {
    onResume = undefined
    dispose()
    await client.close()
    for (const socket of server.clients) socket.terminate()
    await new Promise<void>((resolve) => server.close(() => resolve()))
    harness.renderer.destroy()
  }
})


test("hidden kernel approval dock releases 1,000 App consent cards", async () => {
  const harness = await createTestRenderer({ width: 120, height: 40 })
  const parent = new BoxRenderable(harness.renderer, { visible: false })
  const dock = new BoxRenderable(harness.renderer, {})
  parent.add(dock)
  harness.renderer.root.add(parent)
  const surface = createKernelApprovalRenderer(harness.renderer, {
    show() {}, choose() {},
  })
  surface.assign(dock)
  try {
    for (let index = 0; index < 1_000; index += 1) {
      const view = { handoffEntry: null, choices: [{ id: "allow", label: "Allow", reply: "allow" }], open: true, count: 1, criticalCount: 0, index: 0, selected: null, pending: false,
        connected: true, error: null, passkey: null, interaction: {
          id: `install-${index}`, kernel_operation_id: `operation-${index}`, kind: "permission" as const,
          level: "warning" as const, requested_at_ms: index,
          message: "Approve fixture install or uninstall?", choices: [{ id: "allow", label: "Allow", reply: "allow" }],
        } }
      surface.render(view, { width: 120, height: 40 })
      surface.render({ ...view, count: 0, open: false, interaction: null }, { width: 120, height: 40 })
      if (index % 50 === 0) await harness.renderOnce()
    }
    assert.equal(dock.getChildren().length, 0)
    assert.equal(pendingLayoutChildren(dock), 0)
  } finally {
    harness.renderer.destroy()
  }
})
