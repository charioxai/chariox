import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import test from "node:test"
import { assertDrillDAcceptanceProof } from "./live-browser-computer-drill-d.mjs"

const digest = (bytes) => createHash("sha256").update(bytes).digest("hex")

function proofInput() {
  const projection = {
    sessionId: "session-live",
    environmentId: "room-live",
    runtimeGeneration: 3,
    focusedTabId: "tab-live",
    tabs: [{ tabId: "tab-live", url: "https://mail.invalid/", title: "Mail", documentRevision: 4, focused: true }],
    viewport: { revision: 2, desktopPixelWidth: 1280, desktopPixelHeight: 720 },
  }
  const action = (action_id, sequence, mode, kind, targets) => ({
    action_id, sequence, actor_id: "agent:agent-live", mode, kind,
    arguments: { utf8_byte_count: 42 }, targets, state: "completed", outcome: { status: "completed" },
  })
  const tab = { kind: "browser_tab", id: "tab-live" }
  const history = [
    action("computer-open", 11, "computer", "keyboard_key", [{ kind: "desktop" }]),
    action("computer-type", 12, "computer", "keyboard_text", [{ kind: "desktop" }]),
    action("browser-activate", 13, "browser", "browser_tab_activate", [{ kind: "desktop" }, tab]),
    action("browser-upload", 14, "browser", "upload", [tab]),
    action("browser-submit", 15, "browser", "submit", [tab]),
  ]
  const image = Buffer.from("bounded fake kernel screenshot")
  const screenshot = () => ({
    artifactId: "screenshot-live", bytes: image, sizeBytes: image.length,
    sha256: digest(image), width: 1280, height: 720,
  })
  return {
    options: { sessionId: "session-live", agentId: "agent-live" },
    phase: "passed",
    office: {
      agentId: "agent-live", fixtureClosed: true, document: "/tmp/drill-d-note.txt",
      installed: "mousepad 0.0\nxterm 1.0",
      desktop: { editorDefaultSettings: true, editorSessionBus: true },
      edit: { exactDocument: true, focusPreserved: true, typedActionId: "computer-type", typedSequence: 12 },
      mail: {
        visibleBrowser: true, submissions: 1,
        received: { name: "drill-d-note.txt", sizeBytes: 42, sha256: "a".repeat(64) },
        activationActionId: "browser-activate", uploadActionId: "browser-upload", submitActionId: "browser-submit",
      },
    },
    history,
    baseline: { sequence: 10, projection },
    preBrowserProjection: structuredClone(projection),
    firstBrowser: history[2],
    baselineScreenshot: screenshot(),
    editorScreenshot: screenshot(),
    finalScreenshot: screenshot(),
    finalProjection: structuredClone(projection),
  }
}

test("rejects a stale or wrong Room identity in public acceptance evidence", () => {
  const stale = proofInput()
  stale.preBrowserProjection.sessionId = "session-old"
  assert.throws(() => assertDrillDAcceptanceProof(stale), /Room identity|session/)

  const wrongRoom = proofInput()
  wrongRoom.finalProjection.environmentId = "different-room"
  assert.throws(() => assertDrillDAcceptanceProof(wrongRoom), /Room Environment identity/)
})

test("rejects stale action ids and actions attributed to a different actor", () => {
  const staleAction = proofInput()
  staleAction.office.edit.typedActionId = "action-from-an-earlier-run"
  assert.throws(() => assertDrillDAcceptanceProof(staleAction), /kernel history omitted/)

  const wrongActor = proofInput()
  wrongActor.history[1].actor_id = "agent:other"
  assert.throws(() => assertDrillDAcceptanceProof(wrongActor), /expected agent/)

  const wrongBrowserAction = proofInput()
  wrongBrowserAction.firstBrowser = { ...wrongBrowserAction.firstBrowser, action_id: "stale-browser-action" }
  assert.throws(() => assertDrillDAcceptanceProof(wrongBrowserAction), /first Browser action changed/)
})

test("rejects Browser state changes observed before the first explicit Browser action", () => {
  const input = proofInput()
  input.preBrowserProjection.tabs[0].documentRevision += 1
  assert.throws(() => assertDrillDAcceptanceProof(input), /browser state changed before/)
})

test("rejects a saved attachment whose byte count differs from the attributed edit", () => {
  const input = proofInput()
  input.office.mail.received.sizeBytes = 41
  assert.throws(() => assertDrillDAcceptanceProof(input), /byte count differs/)
})

test("rejects screenshot evidence whose bytes do not match the kernel SHA-256", () => {
  const input = proofInput()
  input.editorScreenshot.sha256 = "b".repeat(64)
  assert.throws(() => assertDrillDAcceptanceProof(input), /bytes changed/)
})
