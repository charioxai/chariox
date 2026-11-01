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
    assert.match(frame, /Chariox · Approve App action · F8/)
    assert.doesNotMatch(frame, /Action: pay|Passkey:|Enter approves/)
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
    surface.render({ ...view, passkey: { ...view.passkey, lifetime: 120 }, prompt: { ...prompt, kind: "sudo", lifetime_minutes: 60, max_lifetime_minutes: 480 } }, { width: 90, height: 30 })
    await harness.renderOnce()
    const frame = harness.captureCharFrame()
    assert.match(frame, /Sudo window/)
    assert.match(frame, /Window: 2 hours · maximum 8 hours/)
    assert.match(frame, /fresh passkey/i)
    assert.match(frame, /Tab duration/)
    assert.doesNotMatch(frame, /External agent access|undefined|Tab lifetime|Tab remember|Remember for|One sudo turn/)
  } finally { harness.renderer.destroy() }
})

test("MP-08 / MP-10 / MP-11 access popup for the local kernel shows no session or scope line", async () => {
  const harness = await createTestRenderer({ width: 90, height: 30 })
  const box = new BoxRenderable(harness.renderer, { position: "absolute", left: 0, top: 0 })
  harness.renderer.root.add(box)
  const surface = createPasskeyPopupRenderer(harness.renderer, noActions)
  surface.assign(box)
  try {
    surface.render({ ...view, prompt: { ...prompt, kind: "access_grant", session_id: "kernel-access", session_alias: null,
      term: { minutes: 480 }, max_lifetime_minutes: 1440 } }, { width: 90, height: 30 })
    await harness.renderOnce()
    const frame = harness.captureCharFrame()
    assert.doesNotMatch(frame, /Local kernel|Session|kernel-access|expires/)
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

// EXT-UX-01 / EXT-UX-02: the owner sees one short question; pid, path, process
// start and scope stay in the requester record and the kernel's audit.
for (const [kind, title, message] of [
  ["access_grant", "External agent access", "Claude Code (an external agent) wants access to this kernel and your other kernels. Allow?"],
  ["access_extension", "Extend external agent access", "Claude Code's access ends in 5 minutes. Extend?"],
] as const) {
  test(`EXT-UX ${kind} shows only the kernel's short question`, async () => {
    const h = await createTestRenderer({ width: 100, height: 50, useThread: false })
    const box = new BoxRenderable(h.renderer, { position: "absolute", left: 0, top: 0 })
    h.renderer.root.add(box)
    const surface = createPasskeyPopupRenderer(h.renderer, noActions)
    surface.assign(box)
    try {
      surface.render({ ...view, passkey: { length: 6, rememberMinutes: 0, lifetime: 30 }, prompt: { ...prompt, kind, session_id: "kernel-access", session_alias: null, title, message,
        requester: { executable: "/opt/codex\n\u001b[31m\u202e", pid: 42, process_start_id: "18446744073709551615", process_exec_version: 7, provider_harness: "claude" },
        term: { minutes: 30 }, max_lifetime_minutes: 1440,
      } }, { width: 100, height: 50 })
      await h.renderOnce()
      const frame = h.captureCharFrame()
      assert.match(frame, new RegExp(title))
      assert.ok(frame.includes(message.slice(0, 60)))
      assert.match(frame, /Access for: 30 minutes/)
      assert.doesNotMatch(frame, /PID|pid|Executable|Process start|exec version|Local kernel|expires|maximum|Requester/)
    } finally { h.renderer.destroy() }
  })
}

// EXT-UX-04: the TUI's status row is always present, so "Waiting for the kernel…" moves nothing.
test("EXT-UX-04 the pending status takes the hint row's place and no row moves", async () => {
  const h = await createTestRenderer({ width: 100, height: 40, useThread: false })
  const box = new BoxRenderable(h.renderer, { position: "absolute", left: 0, top: 0 })
  h.renderer.root.add(box)
  const surface = createPasskeyPopupRenderer(h.renderer, noActions)
  surface.assign(box)
  const rowOf = (frame: string, text: RegExp) => frame.split("\n").findIndex((line) => text.test(line))
  try {
    const access = { ...prompt, kind: "access_grant" as const, session_id: "kernel-access", session_alias: null,
      title: "External agent access", message: "An external agent wants access to this kernel and your other kernels. Allow?", term: { minutes: 30 }, max_lifetime_minutes: 1440 }
    const idle = { ...view, passkey: { length: 0, rememberMinutes: 0, lifetime: 30 }, prompt: access }
    surface.render(idle, { width: 100, height: 40 })
    await h.renderOnce()
    const before = h.captureCharFrame()
    surface.render({ ...idle, pending: true }, { width: 100, height: 40 })
    await h.renderOnce()
    const after = h.captureCharFrame()
    assert.match(after, /Waiting for the kernel…/)
    for (const marker of [/External agent access/, /Passkey:/, /\[ Approve \]/]) assert.equal(rowOf(after, marker), rowOf(before, marker))
    assert.equal(rowOf(after, /Waiting for the kernel…/), rowOf(before, /Enter approves/))
  } finally { h.renderer.destroy() }
})

// MP-08 / MP-11: a pending notice reserves a row instead of covering code.
test("MP-08/MP-11 passkey notice reserves an opaque row above the prompt", async () => {
  const h = await createTestRenderer({ width: 90, height: 12, useThread: false })
  const column = new BoxRenderable(h.renderer, { flexDirection: "column", width: 90 })
  h.renderer.root.add(column)
  column.add(new TextRenderable(h.renderer, { content: "const denseCode = 42;" }))
  const banner = new BoxRenderable(h.renderer, { flexShrink: 0, visible: false })
  column.add(banner)
  column.add(new TextRenderable(h.renderer, { content: "Prompt > draft kept" }))
  const overlay = new BoxRenderable(h.renderer, { position: "absolute", left: 0, top: 0 })
  h.renderer.root.add(overlay)
  let opened = false
  const surface = createPasskeyPopupRenderer(h.renderer, { ...noActions, show() { opened = true } })
  surface.assign(overlay)
  try {
    ;(surface as unknown as { assignBanner(value: BoxRenderable): void }).assignBanner(banner)
    surface.render({ ...view, open: false }, { width: 90, height: 12 })
    await h.renderOnce()
    const lines = h.captureCharFrame().split("\n")
    assert.match(lines[0]!, /const denseCode = 42;/)
    assert.match(lines[1]!, /Chariox · Approve App action/)
    assert.match(lines[2]!, /Prompt > draft kept/)
    await h.mockMouse.click(5, 1, 0)
    assert.equal(opened, true)
    surface.render({ ...view, count: 0, prompt: null }, { width: 90, height: 12 })
    await h.renderOnce()
    assert.match(h.captureCharFrame().split("\n")[1]!, /Prompt > draft kept/)
  } finally { h.renderer.destroy() }
})

// MP-08 / MP-11: requester attribution uses only kernel-verified harness metadata.
test("MP-08/MP-11 notice names the request and verified requester", async () => {
  const h = await createTestRenderer({ width: 100, height: 12, useThread: false })
  const box = new BoxRenderable(h.renderer, { position: "absolute", left: 0, top: 0 })
  h.renderer.root.add(box)
  const surface = createPasskeyPopupRenderer(h.renderer, noActions)
  surface.assign(box)
  try {
    surface.render({ ...view, open: false, prompt: { ...prompt, kind: "access_extension", requester: {
      executable: "/opt/claude", pid: 42, process_start_id: "1", process_exec_version: 0, provider_harness: "claude",
    } } }, { width: 100, height: 12 })
    await h.renderOnce()
    assert.match(h.captureCharFrame(), /Claude Code asks to extend access/)
    assert.doesNotMatch(h.captureCharFrame(), /42|\/opt\/claude/)
  } finally { h.renderer.destroy() }
})

test("MP-08/MP-11 both pending notice and popup show the live request countdown", async () => {
  const h = await createTestRenderer({width: 110, height: 30, useThread: false})
  const box = new BoxRenderable(h.renderer, {position: "absolute", left: 0, top: 0})
  h.renderer.root.add(box)
  const surface = createPasskeyPopupRenderer(h.renderer, noActions)
  surface.assign(box)
  try {
    surface.render({...view, remainingMs: 192_000}, {width: 110, height: 30})
    await h.renderOnce()
    assert.match(h.captureCharFrame(), /3:12 left/)
    surface.render({...view, open: false, remainingMs: 192_000}, {width: 110, height: 30})
    await h.renderOnce()
    assert.match(h.captureCharFrame(), /3:12 left/)
  } finally {h.renderer.destroy()}
})

test("MP-08/MP-11 a countdown tick preserves the request scroll surface", async () => {
  const {ScrollBoxRenderable} = await import("@opentui/core")
  const h = await createTestRenderer({width: 90, height: 30, useThread: false})
  const box = new BoxRenderable(h.renderer, {position: "absolute", left: 0, top: 0})
  h.renderer.root.add(box)
  const surface = createPasskeyPopupRenderer(h.renderer, noActions)
  surface.assign(box)
  const findBody = (node: BoxRenderable): InstanceType<typeof ScrollBoxRenderable> | undefined => {
    if (node instanceof ScrollBoxRenderable) return node
    for (const child of node.getChildren()) if (child instanceof BoxRenderable) {
      const found = findBody(child)
      if (found) return found
    }
  }
  try {
    surface.render({...view, remainingMs: 192_000}, {width: 90, height: 30})
    await h.renderOnce()
    const body = findBody(box)
    assert.ok(body)
    surface.render({...view, remainingMs: 191_000}, {width: 90, height: 30})
    await h.renderOnce()
    assert.equal(findBody(box), body)
    assert.match(h.captureCharFrame(), /3:11 left/)
  } finally {h.renderer.destroy()}
})

// MP-08 / MP-10 / MP-11: normal sudo decisions stay readable at terminal sizes.
test("MP-08/MP-11 typed sudo request collapses technical facts and full task", async () => {
  const h = await createTestRenderer({width: 100, height: 40, useThread: false})
  const box = new BoxRenderable(h.renderer, {position: "absolute", left: 0, top: 0})
  h.renderer.root.add(box)
  const surface = createPasskeyPopupRenderer(h.renderer, noActions)
  surface.assign(box)
  try {
    const p = Object.assign({...prompt, kind: "sudo" as const, title: "Claude Code wants to run one sudo task",
      message: "Sudo allows critical approvals until this task yields or is revoked."}, {sudo_request: {
        requester_name: "Claude Code", agent_name: "Reviewer", session_name: "Payments", task_summary: "Review the release",
        task: "Review the release\n" + "full task line\n".repeat(100), grant_id: "grant-handle", terminal_id: "terminal-handle",
      }})
    surface.render({...view, prompt: p}, {width: 100, height: 40})
    await h.renderOnce()
    const frame = h.captureCharFrame()
    assert.match(frame, /Reviewer/)
    assert.match(frame, /“Review the release”/)
    assert.match(frame, /Show details/)
    assert.doesNotMatch(frame, /full task line|grant-handle|session-1/)
  } finally {h.renderer.destroy()}
})

test("EA2-01 the access popup names the until-exit choice in words", async () => {
  const h = await createTestRenderer({ width: 100, height: 40, useThread: false })
  const box = new BoxRenderable(h.renderer, { position: "absolute", left: 0, top: 0 })
  h.renderer.root.add(box)
  const surface = createPasskeyPopupRenderer(h.renderer, noActions)
  surface.assign(box)
  try {
    const access = { ...prompt, kind: "access_grant" as const, session_id: "kernel-access", session_alias: null,
      title: "External agent access", message: "An external agent wants access to this kernel and your other kernels. Allow?", term: "until_holder_exits" as const, max_lifetime_minutes: 1440 }
    surface.render({ ...view, passkey: { length: 0, rememberMinutes: 0, lifetime: "until_exit" }, prompt: access }, { width: 100, height: 40 })
    await h.renderOnce()
    assert.match(h.captureCharFrame(), /Access for: until the agent exits/)
  } finally { h.renderer.destroy() }
})
