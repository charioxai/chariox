import assert from "node:assert/strict"
import test from "node:test"
import { BoxRenderable, TextRenderable, TextareaRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import type { PasskeyPrompt } from "@chariox/kernel-client/kernel-types"
import { createPasskeyPopupController, type PasskeyPopupView } from "./passkey-popup-controller.js"
import { createPasskeyPopupRenderer } from "./passkey-popup-renderer.js"
import { routeRawPastes } from "./raw-paste-routing.js"

const prompt: PasskeyPrompt = {
  kind: "critical_approval", session_id: "session-1", session_alias: "Payments", interaction_id: "app_validation_op-1",
  title: "Approve App action", message: "An App asks to perform a protected action.\n\nAction: pay",
  approve_choice_id: "approve", refuse_choice_id: "deny", requested_at_ms: 1, expires_at_ms: Date.now() + 300_000,
}
const view: PasskeyPopupView = {
  open: true, count: 1, index: 0, prompt, passkey: { length: 6, rememberMinutes: 0 },
  pending: false, connected: true, error: null,
}
const noActions = { show() {}, approve() {}, refuse() {}, cycleRemember() {} }

test("MP-08/MP-10/MP-11 sudo scope approval names the operation without renewing time", async () => {
  const harness = await createTestRenderer({ width: 90, height: 30, useThread: false })
  const box = new BoxRenderable(harness.renderer, { position: "absolute", left: 0, top: 0 })
  harness.renderer.root.add(box)
  const surface = createPasskeyPopupRenderer(harness.renderer, noActions)
  surface.assign(box)
  try {
    surface.render({ ...view, prompt: { ...prompt, kind: "sudo", title: "Authorize sudo operation scope", message: "Review this exact operation for the original owner work." } }, { width: 90, height: 30 })
    await harness.renderOnce()
    const frame = harness.captureCharFrame()
    assert.match(frame, /Sudo operation scope/)
    assert.match(frame, /Operation scope · fresh passkey required/)
    assert.doesNotMatch(frame, /Tab duration|Tab remember|undefined|maximum/)
  } finally { harness.renderer.destroy() }
})

test("the passkey popup is shaped like the hot-keys popup and shows only the kernel's facts", async () => {
  const harness = await createTestRenderer({ width: 90, height: 30, useThread: false })
  const box = new BoxRenderable(harness.renderer, { position: "absolute", left: 0, top: 0 })
  // A terminal in the waiting room: no session is attached.
  harness.renderer.root.add(new TextRenderable(harness.renderer, { content: "Waiting room · Projects", top: 0 }))
  harness.renderer.root.add(box)
  const surface = createPasskeyPopupRenderer(harness.renderer, noActions)
  surface.assign(box)
  try {
    surface.render(view, { width: 90, height: 30 })
    await harness.renderOnce()
    let frame = harness.captureCharFrame()
    assert.match(frame, /Chariox passkey +Esc hides/)
    assert.match(frame, /Critical approval/)
    assert.match(frame, /Approve App action/)
    assert.match(frame, /Action: pay/)
    assert.match(frame, /Session: Payments \(session-1\) · expires/)
    assert.match(frame, /Passkey: ••••••▏/)
    assert.match(frame, /Remember for: off/)
    assert.match(frame, /\[ Approve \] +\[ Refuse \]/)
    assert.match(frame, /Enter approves · Ctrl\+R refuses · Tab remember/)
    surface.render({ ...view, count: 2, error: "That passkey is not correct." }, { width: 90, height: 30 })
    await harness.renderOnce()
    frame = harness.captureCharFrame()
    assert.match(frame, /Critical approval · 1 of 2/)
    assert.match(frame, /That passkey is not correct\./)
    assert.match(frame, /←\/→ requests/)
    surface.render({ ...view, connected: false }, { width: 90, height: 30 })
    await harness.renderOnce()
    assert.match(harness.captureCharFrame(), /Disconnected · reconnect to answer/)
    // Hidden, it leaves an indicator over the terminal.
    surface.render({ ...view, open: false }, { width: 90, height: 30 })
    await harness.renderOnce()
    frame = harness.captureCharFrame()
    assert.match(frame, /Chariox · 1 passkey request · F8/)
    assert.doesNotMatch(frame, /Approve App action/)
    assert.match(frame, /Waiting room · Projects/)
  } finally { harness.renderer.destroy() }
})

test("only the popup's own buttons answer, with the primary button", async () => {
  const harness = await createTestRenderer({ width: 90, height: 30, useThread: false })
  const box = new BoxRenderable(harness.renderer, { position: "absolute", left: 0, top: 0 })
  harness.renderer.root.add(box)
  const clicks: string[] = []
  const surface = createPasskeyPopupRenderer(harness.renderer, {
    show() {}, approve: () => clicks.push("approve"), refuse: () => clicks.push("refuse"), cycleRemember: () => clicks.push("remember"),
  })
  surface.assign(box)
  try {
    surface.render(view, { width: 90, height: 30 })
    await harness.renderOnce()
    const lines = harness.captureCharFrame().split("\n")
    const at = (label: string) => {
      const y = lines.findIndex((line) => line.includes(label))
      return [lines[y]!.indexOf(label) + 2, y] as const
    }
    await harness.mockMouse.click(...at("[ Refuse ]"), 2)
    await harness.mockMouse.click(0, 0, 0)
    assert.deepEqual(clicks, [], "a stray or secondary click answers nothing")
    await harness.mockMouse.click(...at("[ Refuse ]"), 0)
    await harness.mockMouse.click(...at("[ Approve ]"), 0)
    await harness.mockMouse.click(...at("Remember for"), 0)
    assert.deepEqual(clicks, ["refuse", "approve", "remember"])
    surface.render({ ...view, pending: true }, { width: 90, height: 30 })
    await harness.renderOnce()
    await harness.mockMouse.click(...at("[ Approve ]"), 0)
    assert.deepEqual(clicks, ["refuse", "approve", "remember"])
  } finally { harness.renderer.destroy() }
})

// The popup as cli-kernel-approval-composition.ts wires it: keys from
// OpenTUI's keyboard, pastes through routeRawPastes, the prompt blurred while
// the popup is open. These run OpenTUI's own parser over a terminal's bytes.
async function popupHarness(kittyKeyboard: boolean) {
  const harness = await createTestRenderer({ width: 80, height: 24, useThread: false, kittyKeyboard })
  const textarea = new TextareaRenderable(harness.renderer, { initialValue: "draft kept" })
  harness.renderer.root.add(textarea)
  const proofs: unknown[] = []
  const popup = createPasskeyPopupController({
    connected: () => true, onView() {}, scroll() {}, notify() {},
    onOpen: () => textarea.blur(), onClose: () => textarea.focus(),
    respond: async (_prompt, choice, proof) => { proofs.push([choice, proof?.passkey]); return new Promise(() => {}) },
  })
  harness.renderer.keyInput.on("keypress", popup.handleKey)
  const stopPastes = routeRawPastes(harness.renderer.keyInput, popup.handlePaste)
  textarea.focus()
  popup.apply([prompt])
  assert.equal(textarea.focused, true, "arrival leaves the composer focused")
  harness.renderer.stdin.emit("data", Buffer.from("\x1b[19~"))
  assert.equal(textarea.focused, false, "F8 deliberately focuses the popup")
  return {
    harness, textarea, proofs, popup,
    send: (bytes: string) => { harness.renderer.stdin.emit("data", Buffer.from(bytes)) },
    dispose() {
      stopPastes()
      harness.renderer.keyInput.off("keypress", popup.handleKey)
      popup.dispose()
      harness.renderer.destroy()
    },
  }
}

for (const kittyKeyboard of [false, true]) {
  test(`a typed and pasted passkey reaches the kernel exactly (kitty keyboard ${kittyKeyboard})`, async () => {
    const h = await popupHarness(kittyKeyboard)
    try {
      if (kittyKeyboard) {
        // Shift+c without associated text, o, Shift+1 reporting its shifted key.
        h.send("\u001b[99;2u")
        h.send("\u001b[111u")
        h.send("\u001b[49:33;2u")
      } else {
        await h.harness.mockInput.typeText("Co!")
      }
      h.send("\u{1D11E}")
      await h.harness.mockInput.pasteBracketedText("Pa$te Ünï\u{1F511} \n")
      // OpenTUI strips ANSI codes from a paste; the raw paste is refused.
      await h.harness.mockInput.pasteBracketedText("Ab\u001b[31mCd")
      assert.match(h.popup.view().error ?? "", /^Paste not added/)
      assert.equal(h.textarea.plainText, "draft kept", "no passkey text reaches the prompt")

      h.send("\r")
      await new Promise((resolve) => setTimeout(resolve, 0))
      assert.deepEqual(h.proofs, [["approve", "Co!\u{1D11E}Pa$te Ünï\u{1F511} "]])
    } finally { h.dispose() }
  })
}

test("styled text pasted without bracketed paste clears the passkey instead of losing its codes", async () => {
  const h = await popupHarness(false)
  try {
    h.send("Ab\u001b[31mCd\r")
    assert.equal(h.proofs.length, 0)
    assert.equal(h.popup.view().passkey.length, 0)
    assert.match(h.popup.view().error ?? "", /control sequence/)
    await new Promise((resolve) => setTimeout(resolve, 0))
    await h.harness.mockInput.typeText("Ok")
    h.send("\r")
    await new Promise((resolve) => setTimeout(resolve, 0))
    assert.deepEqual(h.proofs, [["approve", "Ok"]])
  } finally { h.dispose() }
})

test("Escape hides the popup and gives the prompt back; the passkey is gone", async () => {
  const h = await popupHarness(false)
  try {
    await h.harness.mockInput.typeText("secret")
    await h.harness.mockInput.pressKeys(["ESCAPE"])
    // A bare ESC is held briefly to distinguish it from a terminal sequence.
    await new Promise((resolve) => setTimeout(resolve, 50))
    assert.equal(h.popup.view().open, false)
    assert.equal(h.textarea.focused, true)
    assert.equal(h.textarea.plainText, "draft kept")
    await h.harness.mockInput.pressKeys(["F8"])
    assert.equal(h.popup.view().open, true)
    assert.equal(h.popup.view().passkey.length, 0)
  } finally { h.dispose() }
})

test("MP-08/MP-10/MP-11 A04 sudo popup shows the window duration and its 8-hour maximum", async () => {
  const harness = await createTestRenderer({ width: 90, height: 30, useThread: false })
  const box = new BoxRenderable(harness.renderer, { position: "absolute", left: 0, top: 0 })
  harness.renderer.root.add(box)
  const surface = createPasskeyPopupRenderer(harness.renderer, noActions)
  surface.assign(box)
  try {
    surface.render({ ...view, passkey: { ...view.passkey, accessLifetimeMinutes: 120 }, prompt: { ...prompt, kind: "sudo", lifetime_minutes: 60, max_lifetime_minutes: 480 } }, { width: 90, height: 30 })
    await harness.renderOnce()
    const frame = harness.captureCharFrame()
    assert.match(frame, /Sudo window/)
    assert.match(frame, /Window: 2 hours · maximum 8 hours/)
    assert.match(frame, /fresh passkey/i)
    assert.match(frame, /Tab duration/)
    assert.doesNotMatch(frame, /External agent access|undefined|Tab lifetime|Tab remember|Remember for|One sudo turn/)
  } finally { harness.renderer.destroy() }
})

test("MP-08 / MP-10 / MP-11 access popup describes the whole local kernel without a session", async () => {
  const harness = await createTestRenderer({ width: 90, height: 30 })
  const box = new BoxRenderable(harness.renderer, { position: "absolute", left: 0, top: 0 })
  harness.renderer.root.add(box)
  const surface = createPasskeyPopupRenderer(harness.renderer, noActions)
  surface.assign(box)
  try {
    surface.render({ ...view, prompt: { ...prompt, kind: "access_grant", session_id: "kernel-access", session_alias: null,
      lifetime_minutes: 480, max_lifetime_minutes: 1440 } }, { width: 90, height: 30 })
    await harness.renderOnce()
    const frame = harness.captureCharFrame()
    assert.match(frame, /Local kernel/)
    assert.doesNotMatch(frame, /Session:|Session kernel-access/)
  } finally { harness.renderer.destroy() }
})

for (const kind of ["access_grant", "access_extension"] as const) {
  test(`MP-08 / MP-10 / MP-11 legacy ${kind} popup keeps its session scope visible`, async () => {
    const harness = await createTestRenderer({ width: 90, height: 30 })
    const box = new BoxRenderable(harness.renderer, { position: "absolute", left: 0, top: 0 })
    harness.renderer.root.add(box)
    const surface = createPasskeyPopupRenderer(harness.renderer, noActions)
    surface.assign(box)
    try {
      surface.render({ ...view, prompt: { ...prompt, kind } }, { width: 90, height: 30 })
      await harness.renderOnce()
      const frame = harness.captureCharFrame()
      assert.match(frame, /Session: Payments \(session-1\)/)
      assert.doesNotMatch(frame, /Local kernel/)
    } finally { harness.renderer.destroy() }
  })
}

for (const kind of ["access_grant", "access_extension"] as const) {
  test(`MP-08 / MP-10 / MP-11 ${kind} labels the structured requester independently of message text`, async () => {
    const h = await createTestRenderer({ width: 100, height: 50, useThread: false })
    const box = new BoxRenderable(h.renderer, { position: "absolute", left: 0, top: 0 })
    h.renderer.root.add(box)
    const surface = createPasskeyPopupRenderer(h.renderer, noActions)
    surface.assign(box)
    try {
      surface.render({ ...view, prompt: { ...prompt, kind, session_id: "kernel-access", session_alias: null,
        message: "Display text mentions Claude (pid 999) but cannot supply the requester label.",
        requester: { executable: "/opt/codex\n\u001b[31m\u202e", pid: 42, process_start_id: "18446744073709551615", process_exec_version: 7, provider_harness: "codex" },
        lifetime_minutes: 30, max_lifetime_minutes: 1440,
      } }, { width: 100, height: 50 })
      await h.renderOnce()
      const frame = h.captureCharFrame()
      assert.match(frame, /Requester: Codex · PID 42/)
      assert.match(frame, /Executable: "\/opt\/codex\\n\\u001b\[31m\\u202e"/)
      assert.match(frame, /Process start: 18446744073709551615 · exec version 7/)
      assert.match(frame, /Display text mentions Claude/)
      surface.render({ ...view, prompt: { ...prompt, kind, requester: null } }, { width: 100, height: 50 })
      await h.renderOnce()
      assert.match(h.captureCharFrame(), /Requester identity unavailable/)
    } finally { h.renderer.destroy() }
  })
}
