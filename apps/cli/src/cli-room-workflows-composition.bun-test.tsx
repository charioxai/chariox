import assert from "node:assert/strict"
import test, { after } from "node:test"
import { mkdtemp, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { createRoot, createSignal } from "solid-js"
import { testRender } from "@opentui/solid"
import { createCliRoomWorkflowsComposition } from "./cli-room-workflows-composition.js"
import { RoomWorkflowsPane } from "./room-workflows-pane.js"
import { WorkspaceLayout, type WorkspaceLayoutProps } from "./workspace-layout.js"
import { theme } from "./theme.js"
import { saveRoomWorkflowPaneDismissed, type CharioxPreferences } from "./preferences.js"
import type { RoomWorkflowInventory } from "@chariox/kernel-client/kernel-types"
import type { LocalIpcClient } from "./ipc.js"
import type { RuntimeSession } from "./cli-types.js"

const previousConfig = process.env.XDG_CONFIG_HOME
const scratch = await mkdtemp(join(tmpdir(), "chariox-workflow-focus-"))
process.env.XDG_CONFIG_HOME = scratch
// Await the existing serialized preference writer before deleting disposable state.
after(async () => {
  await saveRoomWorkflowPaneDismissed("test-barrier", false)
  await rm(scratch, { recursive: true, force: true })
  if (previousConfig === undefined) delete process.env.XDG_CONFIG_HOME
  else process.env.XDG_CONFIG_HOME = previousConfig
})

function inventory(count = 1): RoomWorkflowInventory {
  return { session_id: "room", home_kernel_id: "home", revision: String(count), workflow_count: count,
    workflows: count ? [{ workflow_id: "flow", workflow_revision: 1, label: "Review", state: "running", running_count: 1, paused_count: 0, queued_count: 0,
      endpoints: [{ endpoint_id: "entry", entry_node_id: "node", label: "Check", can_start: true }],
      runs: [{ run_id: "current", endpoint_id: "entry", status: "Running", can_pause: true, can_resume: false, can_stop: true }] }] : [] }
}
function fixture(width = 160, initial = inventory()) {
  return createRoot(dispose => {
    const [canvas, setCanvas] = createSignal(false)
    const [dialog, setDialog] = createSignal(false)
    let receive: (event: { event: string; inventory: RoomWorkflowInventory }) => void = () => {}
    const requests: unknown[] = []
    let blurred = 0
    const deps = {
      clientId: "focus-test", preferences: { roomWorkflowPaneDismissed: {} } as CharioxPreferences,
      client: { onKernelEvent: (handler: typeof receive) => { receive = handler; return () => {} },
        send: async (request: unknown) => { requests.push(request); return { WorkflowRunInvoked: { workflow_run: { id: "started" } } } } } as unknown as LocalIpcClient,
      session: () => ({ id: "room", room_workflows: initial, room_workflows_fresh: true }) as RuntimeSession,
      attached: () => true, connected: () => true, narrow: () => width < 110,
      workflowScreenActive: canvas, showRoom: () => setCanvas(false), dialogOverlayOpen: dialog,
      manage: () => setCanvas(true), focusAgents() {}, blurAgents: () => { blurred++ },
    }
    const pane = createCliRoomWorkflowsComposition(deps)
    const rendered = pane.visible
    return { pane, canvas, setCanvas, setDialog, requests, dispose, rendered, blurred: () => blurred,
      receive: (value: RoomWorkflowInventory) => receive({ event: "room_workflows_changed", inventory: value }) }
  })
}
async function renderFixture(h: ReturnType<typeof fixture>) {
  const harness = await testRender(() => <RoomWorkflowsPane controller={h.pane.controller} revision={h.pane.revision} width={70} onAgents={() => h.pane.controller.focus(false)} onFocus={() => h.pane.controller.focus(true)} />,
    { width: 70, height: 28, useThread: false, exitOnCtrlC: false })
  harness.renderer.keyInput.on("keypress", h.pane.handleKey)
  return harness
}

test("Manage then Ctrl+W returns to the rendered room pane and its composer", async () => {
  const h = fixture()
  const ui = await renderFixture(h)
  try {
    ui.mockInput.pressKey("w", { ctrl: true }); await ui.renderOnce()
    ui.mockInput.pressKey("g", { ctrl: true }); await ui.renderOnce()
    assert.equal(h.canvas(), true)
    assert.equal(h.pane.ownsInput(), false)
    assert.equal(h.rendered(), false, "the Manage canvas hides the room pane")
    ui.mockInput.pressKey("w", { ctrl: true }); await ui.renderOnce()
    assert.equal(h.canvas(), false, "opening workflows must leave the Manage canvas")
    assert.equal(h.rendered(), true)
    assert.equal(h.pane.ownsInput(), true)
    await ui.mockInput.typeText("after Manage")
    await ui.mockInput.pressKeys(["RETURN"])
    await ui.renderOnce()
    assert.equal(h.requests.length, 1)
    assert.match(ui.captureCharFrame(), /Started run started/)
  } finally { ui.renderer.keyInput.off("keypress", h.pane.handleKey); h.dispose(); ui.renderer.destroy() }
})

test("Manage then a real workspace header click returns to the room and focuses workflows", async () => {
  const h = fixture()
  const noop = () => {}
  const props: WorkspaceLayoutProps = {
    width: 70, height: 24, fatalError: false, themeRevision: 0, responsePaneRows: () => [],
    promptPlaceholder: "Agent prompt", promptInputMaxHeight: 1, promptKeyBindings: [], promptAreaBackground: theme.background,
    onPaneGridBottomBorderRowRef: noop, onPaneGridBottomHorizontalSegmentRef: noop, onPaneGridBottomJunctionTextRef: noop,
    onRootMouseUp: noop, onResponseSurfaceMouseUp: noop, onFooterMouseUp: noop,
    onResponseLayoutBoxRef: noop, onResponseRowBoxRef: noop, onPaneGridBorderRowRef: noop, onSudoBandRef: noop,
    onPaneGridHorizontalSegmentRef: noop, onPaneGridJunctionTextRef: noop, onPaneGridVerticalSegmentRef: noop,
    onResponsePrimaryPaneRef: noop, onHistoryLoadingBoxRef: noop, onTranscriptScrollboxRef: noop,
    onResponsePrimaryInteractionBoxRef: noop, onResponsePrimaryFooterBoxRef: noop,
    onResponseAuxiliaryPaneRef: noop, onResponseAuxiliaryScrollboxRef: noop,
    onResponseAuxiliaryInteractionBoxRef: noop, onResponseAuxiliaryFooterBoxRef: noop,
    onCommandCenterBoxRef: noop, onPromptInputRef: noop, onPromptKeyDown: noop, onPromptContentChange: noop, onPromptSubmit: noop,
    onPromptMetaProviderTextRef: noop, onPromptMetaProviderDividerTextRef: noop, onPromptMetaModelTextRef: noop,
    onPromptMetaModelDividerTextRef: noop, onPromptMetaVariantTextRef: noop, onPromptMetaUsageDividerTextRef: noop,
    onPromptMetaUsageTokensTextRef: noop, onPromptMetaUsageBarOpenTextRef: noop, onPromptMetaUsageBarFilledTextRef: noop,
    onPromptMetaUsageBarEmptyTextRef: noop, onPromptMetaUsageBarCloseTextRef: noop, onPromptMetaUsagePercentTextRef: noop,
    onStatusIndicatorBoxRef: noop, onFooterSummaryBoxRef: noop, onHotkeysOverlayBoxRef: noop,
    onKernelApprovalBannerRef: noop, onKernelApprovalBoxRef: noop, onPasskeyPopupBoxRef: noop,
    roomWorkflowsAvailable: () => h.pane.controller.available, roomWorkflowsVisible: h.pane.visible,
    roomWorkflowsOpen: h.pane.open, roomWorkflowsPane: () => <text>Room workflow pane</text>,
  }
  const ui = await testRender(() => <WorkspaceLayout {...props} />, { width: 70, height: 24, useThread: false, exitOnCtrlC: false })
  try {
    h.pane.controller.open(); h.pane.controller.draft("kept through Manage"); h.pane.controller.manage()
    await ui.renderOnce()
    assert.equal(h.canvas(), true)
    assert.equal(h.pane.visible(), false)
    const lines = ui.captureCharFrame().split("\n")
    const y = lines.findIndex(line => line.includes("Workflows · Ctrl+W"))
    assert.ok(y >= 0, "the reopen header remains visible over Manage")
    await ui.mockMouse.click(lines[y]!.indexOf("Workflows") + 2, y, 0)
    await ui.renderOnce()
    assert.equal(h.canvas(), false, "the header must leave the Manage canvas")
    assert.equal(h.pane.visible(), true)
    assert.equal(h.pane.ownsInput(), true)
    assert.equal(h.blurred(), 1)
    assert.equal(h.pane.controller.record?.draft, "kept through Manage")
    assert.match(ui.captureCharFrame(), /Room workflow pane/)
  } finally { h.dispose(); ui.renderer.destroy() }
})

test("a hidden workflow pane cannot own input or control runs", () => {
  const h = fixture()
  try {
    h.pane.controller.open(); h.setCanvas(true)
    assert.equal(h.rendered(), false)
    assert.equal(h.pane.ownsInput(), false)
    for (const name of ["p", "r", "s"]) assert.equal(h.pane.handleKey({ name, ctrl: true, preventDefault() {}, stopPropagation() {} }), false)
    assert.equal(h.requests.length, 0)
  } finally { h.dispose() }
})

test("dialogs retain priority over workflow opening and run controls", () => {
  const h = fixture()
  try {
    h.setDialog(true)
    h.pane.open()
    assert.equal(h.blurred(), 0, "the shared header open action respects dialogs")
    assert.equal(h.pane.handleKey({ name: "w", ctrl: true, preventDefault() {}, stopPropagation() {} }), false)
    assert.equal(h.pane.controller.focused, false)
    h.pane.controller.open()
    assert.equal(h.pane.ownsInput(), false)
    assert.equal(h.pane.handleKey({ name: "p", ctrl: true, preventDefault() {}, stopPropagation() {} }), false)
    assert.equal(h.requests.length, 0)
  } finally { h.dispose() }
})

for (const firstInventory of [false, true]) test(`narrow ${firstInventory ? "first inventory" : "attachment"} transfers focus and accepts typing/Enter`, async () => {
  const h = fixture(70, inventory(firstInventory ? 0 : 1))
  const ui = await renderFixture(h)
  try {
    if (firstInventory) { h.pane.controller.focus(false); h.receive(inventory()) }
    await ui.renderOnce()
    assert.equal(h.rendered(), true)
    assert.equal(h.pane.controller.focused, true)
    assert.equal(h.pane.ownsInput(), true)
    assert.equal(h.blurred(), 1)
    await ui.mockInput.typeText("automatic narrow input")
    await ui.mockInput.pressKeys(["RETURN"])
    await ui.renderOnce()
    assert.equal(h.requests.length, 1)
    assert.match(ui.captureCharFrame(), /Started run started/)
    await ui.mockInput.pressKeys(["TAB"]); await ui.renderOnce()
    assert.equal(h.rendered(), false, "Tab returns to agents without a prior Ctrl+W")
    ui.mockInput.pressKey("w", { ctrl: true }); await ui.renderOnce()
    await ui.mockInput.pressKeys(["ESCAPE"])
    // A bare ESC is held briefly to distinguish it from a terminal sequence.
    await new Promise(resolve => setTimeout(resolve, 50))
    assert.equal(h.pane.controller.visible, false, "Escape dismisses the automatically opened pane")
  } finally { ui.renderer.keyInput.off("keypress", h.pane.handleKey); h.dispose(); ui.renderer.destroy() }
})
