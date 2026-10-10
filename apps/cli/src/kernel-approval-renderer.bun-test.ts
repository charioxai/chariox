import assert from "node:assert/strict"
import test from "node:test"
import { createRoot } from "solid-js"
import { createCliKernelApprovalComposition } from "./cli-kernel-approval-composition.js"
import { BoxRenderable, TextRenderable, TextareaRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import { createKernelApprovalRenderer } from "./kernel-approval-renderer.js"
import { createKernelApprovalController, type KernelApprovalView } from "./kernel-approval-controller.js"
import type { RuntimeInteraction, RuntimeSession } from "./cli-types.js"
import type { PasskeyPrompt } from "@chariox/kernel-client/kernel-types"

const view: KernelApprovalView = {
  open: false, count: 1, criticalCount: 0, index: 0, selected: null, pending: false, connected: true, error: null,
  interaction: {
    id: "approval-1", kernel_operation_id: "install-1", kind: "permission", level: "warning",
    title: "Install Linear", message: "Allow this App to use the capabilities listed in the installation?",
    choices: [{ id: "deny", label: "Deny", reply: "deny" }, { id: "allow", label: "Allow", reply: "allow" }],
    requested_at_ms: 1,
  },
}

for (const kind of ["access_grant", "access_extension"] as const) {
  for (const sessionId of ["legacy-session", "kernel-access"]) {
    for (const approve of [false, true]) {
      test(`access popup ${kind} ${approve ? "approval" : "refusal"} preserves ${sessionId} contract`, async () => {
        const h = await createTestRenderer({ width: 100, height: 36, useThread: false })
        let listener!: (event: unknown) => void
        const requests: Record<string, any>[] = []
        const notices: string[] = []
        const client = {
          onKernelEvent(callback: typeof listener) { listener = callback; return () => {} },
          async send(request: Record<string, any>) {
            requests.push(request)
            return sessionId === "kernel-access"
              ? { KernelAccessDecisionResponded: { interaction_id: "access-test" } }
              : { InteractionResponded: { interaction_id: "access-test", session: { id: sessionId, agents: [] } } }
          },
        }
        let dispose!: () => void
        const composition = createRoot(cleanup => {
          dispose = cleanup
          return createCliKernelApprovalComposition({
            client: client as never, renderer: h.renderer,
            session: () => ({ id: sessionId, agents: [] }) as unknown as RuntimeSession,
            connected: () => true, attached: () => true, kernelConnected: () => true, attachmentId: () => null,
            flashFooter() {}, dimensions: () => ({ width: 100, height: 36 }), themeRevision: () => 0,
            currentFocus: () => null, promptFocus: () => null, closeOtherDialog() {}, applySession() {},
            notify(message) { notices.push(message) },
          })
        })
        try {
          const prompt: PasskeyPrompt = {
            kind, session_id: sessionId, interaction_id: "access-test", title: "External access", message: "Review access",
            approve_choice_id: "approve", refuse_choice_id: "refuse", requested_at_ms: 1, expires_at_ms: 300_001,
          }
          listener({ event: "passkey_prompts_changed", prompts: [prompt] })
          const key = (name: string, extra = {}) => composition.handleKey({ name, preventDefault() {}, stopPropagation() {}, ...extra })
          key("f8")
          if (approve) { for (const sequence of "test-proof") key(sequence, { sequence }); key("return") }
          else key("r", { ctrl: true })
          await new Promise(resolve => setTimeout(resolve, 0))
          assert.equal(requests.length, 1)
          assert.equal(requests[0]?.RespondToInteraction.session_id, sessionId)
          assert.equal(requests[0]?.RespondToInteraction.choice_id, approve ? "approve" : "refuse")
          assert.deepEqual(notices, [])
        } finally { dispose(); h.renderer.destroy() }
      })
    }
  }
}

const shortcutCases = [
  ["darwin", "F8 (fn+F8 on Mac) or Ctrl+G"],
  ["linux", "F8 or Ctrl+G"],
] as const

for (const [platform, shortcutLabel] of [
  ...shortcutCases,
  [undefined, process.platform === "darwin" ? shortcutCases[0][1] : shortcutCases[1][1]],
] as const) {
  test(`global approvals render over a zero-agent workspace and preserve the sole prompt (${platform ?? "host default"})`, async () => {
    const harness = await createTestRenderer({ width: 80, height: 24, useThread: false })
    const box = new BoxRenderable(harness.renderer, { position: "absolute", left: 0, top: 0 })
    harness.renderer.root.add(new TextRenderable(harness.renderer, { content: "App workspace · no focus agent\nPrompt > draft kept", top: 22 }))
    harness.renderer.root.add(box)
    const surface = createKernelApprovalRenderer(harness.renderer, { show() {}, choose() {} }, platform)
    surface.assign(box)
    try {
      surface.render(view, { width: 80, height: 24 })
      await harness.renderOnce()
      let frame = harness.captureCharFrame()
      assert.equal(frame.split("\n")[0]?.trim(), `Chariox · 1 approval · ${shortcutLabel}`)
      assert.doesNotMatch(frame, /Install Linear/)
      surface.render({ ...view, open: true }, { width: 80, height: 24 })
      await harness.renderOnce()
      frame = harness.captureCharFrame()
      assert.match(frame, /Install Linear/)
      assert.match(frame, /Enter confirm/)
      assert.match(frame, /Prompt > draft kept/)
      assert.doesNotMatch(frame, /› Allow|› Deny/)
      surface.render({ ...view, open: true, selected: 1, pending: true }, { width: 80, height: 24 })
      await harness.renderOnce()
      assert.match(harness.captureCharFrame(), /Waiting for kernel confirmation/)
      surface.render({ ...view, open: true, connected: false }, { width: 80, height: 24 })
      await harness.renderOnce()
      assert.match(harness.captureCharFrame(), /Disconnected · reconnect to respond/)
    } finally { harness.renderer.destroy() }
  })
}

test("approval title, text and choice controls fit narrow terminals without horizontal clipping", async () => {
  const harness = await createTestRenderer({ width: 48, height: 18, useThread: false })
  const box = new BoxRenderable(harness.renderer, { position: "absolute", left: 0, top: 0 })
  harness.renderer.root.add(box)
  const surface = createKernelApprovalRenderer(harness.renderer, { show() {}, choose() {} })
  surface.assign(box)
  try {
    surface.render({ ...view, open: true, selected: 0 }, { width: 48, height: 18 })
    await harness.renderOnce()
    const frame = harness.captureCharFrame()
    assert.match(frame, /Install Linear/)
    assert.match(frame, /› Deny/)
    assert.match(frame, /Enter confirm/)
    assert.match(frame, /scroll/)
  } finally { harness.renderer.destroy() }
})

test("actual OpenTUI keyboard delivery isolates approval choices from the focused textarea", async () => {
  const harness = await createTestRenderer({ width: 80, height: 24, useThread: false })
  let submitted = 0
  const requests: string[] = []
  const prompt = new TextareaRenderable(harness.renderer, {
    initialValue: "draft kept", keyBindings: [{ name: "return", action: "submit" }],
    onSubmit: () => { if (!controller.ownsInput()) submitted += 1 },
  })
  harness.renderer.root.add(prompt)
  const controller = createKernelApprovalController({
    getSession: () => ({ id: "session-1", agents: [], active_interactions: [view.interaction!] }) as unknown as RuntimeSession,
    connected: () => true, onView() {}, scroll() {},
    onOpen: () => prompt.blur(), onClose: () => prompt.focus(),
    respond: async (_session, _interaction, choice) => { requests.push(choice); return new Promise(() => {}) },
    applySession() {}, showPasskeyPrompt: () => false,
  })
  harness.renderer.keyInput.on("keypress", controller.handleKey)
  try {
    controller.sync()
    prompt.focus()
    await harness.mockInput.pressKeys(["RETURN"])
    assert.equal(submitted, 1)
    assert.equal(requests.length, 0)
    await harness.mockInput.pressKeys(["F8", "RETURN", "1"])
    assert.equal(prompt.focused, false)
    assert.equal(requests.length, 0)
    assert.equal(submitted, 1)
    assert.equal(prompt.plainText, "draft kept")
    await harness.mockInput.pressKeys(["ARROW_DOWN", "RETURN", "RETURN"])
    assert.deepEqual(requests, ["deny"])
    await harness.mockInput.pressKeys(["ESCAPE"])
    // A bare ESC is held briefly to distinguish it from a terminal sequence.
    await new Promise((resolve) => setTimeout(resolve, 50))
    assert.equal(prompt.focused, true)
    assert.equal(prompt.plainText, "draft kept")
  } finally {
    harness.renderer.keyInput.off("keypress", controller.handleKey)
    controller.dispose()
    harness.renderer.destroy()
  }
})

test("actual mouse clicks require the primary button and a connected, nonpending approval", async () => {
  const harness = await createTestRenderer({ width: 80, height: 24, useThread: false })
  const box = new BoxRenderable(harness.renderer, { position: "absolute", left: 0, top: 0 })
  harness.renderer.root.add(box)
  const choices: string[] = []
  const surface = createKernelApprovalRenderer(harness.renderer, { show() {}, choose: (interactionId, id) => { assert.equal(interactionId, "approval-1"); choices.push(id) } })
  surface.assign(box)
  try {
    surface.render({ ...view, open: true }, { width: 80, height: 24 })
    await harness.renderOnce()
    const lines = harness.captureCharFrame().split("\n")
    const y = lines.findIndex((line) => line.includes("Deny"))
    const x = lines[y]!.indexOf("Deny")
    assert.ok(x >= 0 && y >= 0)
    await harness.mockMouse.click(x, y, 2)
    assert.deepEqual(choices, [])
    await harness.mockMouse.click(x, y, 0)
    assert.deepEqual(choices, ["deny"])
    for (const state of [{ pending: true }, { connected: false }]) {
      surface.render({ ...view, open: true, ...state }, { width: 80, height: 24 })
      await harness.renderOnce()
      await harness.mockMouse.click(x, y, 0)
    }
    assert.deepEqual(choices, ["deny"])
    surface.render({ ...view, open: true, selected: 1,
      interaction: { ...view.interaction!, message: "Long permission details\n".repeat(100) },
    }, { width: 80, height: 24 })
    await harness.renderOnce()
    assert.match(harness.captureCharFrame(), /Selected: Allow/)
  } finally { harness.renderer.destroy() }
})

test("App clipboard and link offers show the payload, explicit typed acceptance and Decline in the trusted TUI", async () => {
  const harness = await createTestRenderer({ width: 100, height: 26, useThread: false })
  const box = new BoxRenderable(harness.renderer, { position: "absolute", left: 0, top: 0 })
  harness.renderer.root.add(box)
  const surface = createKernelApprovalRenderer(harness.renderer, { show() {}, choose() {} })
  surface.assign(box)
  try {
    for (const [title, payload] of [["Open a link from an App", "Exact URL: https://example.org/a?x=%20"], ["Copy text from an App", 'Text (11 UTF-8 bytes): "copy\\ntext"']] as const) {
      surface.render({ ...view, open: true, interaction: { id: "app_host_0123456789abcdef0123456789abcdef", kernel_operation_id: "host_action:0123456789abcdef0123456789abcdef", kind: "permission", level: "warning", requested_at_ms: 1, title,
        message: `${payload}\nOnly alice can answer.\n/app host accept`, choices: [{ id: "decline", label: "Decline", reply: "deny" }] } }, { width: 100, height: 26 })
      await harness.renderOnce()
      const frame = harness.captureCharFrame()
      assert.ok(frame.includes(title!))
      assert.ok(frame.includes(payload!))
      assert.match(frame, /\/app host accept/)
      assert.match(frame, /Decline/)
      assert.doesNotMatch(frame, /› Decline/)
    }
  } finally { harness.renderer.destroy() }
})

for (const [platform, shortcutLabel] of shortcutCases) {
  for (const width of [80, 48]) {
    for (const [name, count, criticalCount] of [["none", 0, 0], ["one", 1, 0], ["several", 3, 0], ["critical", 3, 1]] as const) {
      test(`prompt approval banner renders ${name} at ${width} columns without covering the draft (${platform})`, async () => {
        const h = await createTestRenderer({ width, height: 24, useThread: false })
        const layout = new BoxRenderable(h.renderer, { width, height: 24, flexDirection: "column" })
        const response = new BoxRenderable(h.renderer, { flexGrow: 1 })
        response.add(new TextRenderable(h.renderer, { content: "App workspace · no focus agent" }))
        const banner = new BoxRenderable(h.renderer, { flexDirection: "column", flexShrink: 0 })
        const prompt = new TextareaRenderable(h.renderer, { initialValue: "Prompt > draft kept\nsecond draft line", flexShrink: 0 })
        layout.add(response)
        layout.add(banner)
        layout.add(prompt)
        h.renderer.root.add(layout)
        let opened = 0
        const surface = createKernelApprovalRenderer(h.renderer, { show() { opened += 1 }, choose() {} }, platform)
        surface.assignBanner(banner)
        try {
          prompt.focus()
          surface.render({ ...view, count, criticalCount }, { width, height: 24 })
          await h.renderOnce()
          const frame = h.captureCharFrame()
          assert.match(frame, /Prompt > draft kept *\n.*second draft line/)
          assert.equal(prompt.focused, true)
          assert.equal(opened, 0)
          const lines = frame.split("\n").map(line => line.trimEnd())
          const bannerStart = lines.findIndex(line => line.includes("Action needed"))
          if (!count) {
            assert.equal(bannerStart, -1)
            assert.equal(banner.visible, false)
          } else {
            assert.match(frame, new RegExp(`${count} approval${count === 1 ? "" : "s"} waiting:`))
            const bannerEnd = lines.findIndex(line => line.includes("Prompt >"))
            assert.ok(bannerStart < bannerEnd)
            const contents = lines.slice(bannerStart, bannerEnd).join("\n").replace(/\s+/g, " ")
            const expectedHint = `Open: ${shortcutLabel} or /approvals`
            assert.equal((banner.getChildren().at(-1) as TextRenderable).plainText, expectedHint)
            // Soft wrapping can split /approvals between its slash and letters.
            assert.ok(contents.replace(/\s/g, "").includes(expectedHint.replace(/\s/g, "")), contents)
            if (criticalCount) assert.match(contents, /Critical — passkey needed.*Open to review and approve with your passkey/)
            else assert.doesNotMatch(contents, /passkey needed/)
            await h.mockMouse.click(2, bannerStart, 0)
            assert.equal(opened, 1)
          }
          if (process.env.APPROVAL_EVIDENCE_DIR && width === 80) {
            const { writeFileSync } = await import("node:fs")
            writeFileSync(`${process.env.APPROVAL_EVIDENCE_DIR}/after-${platform}-${name}.txt`, frame)
          }
          surface.render({ ...view, count: 0, criticalCount: 0 }, { width, height: 24 })
          await h.renderOnce()
          assert.doesNotMatch(h.captureCharFrame(), /Action needed/)
        } finally { h.renderer.destroy() }
      })
    }
  }
}

test("OpenTUI Ctrl+G terminal bytes open approvals and preserve focused draft", async () => {
  const h = await createTestRenderer({ width: 80, height: 24, useThread: false })
  const prompt = new TextareaRenderable(h.renderer, { initialValue: "draft kept" })
  h.renderer.root.add(prompt)
  const controller = createKernelApprovalController({
    getSession: () => ({ id: "session", agents: [], active_interactions: [view.interaction!] }) as unknown as RuntimeSession,
    connected: () => true, onView() {}, scroll() {},
    onOpen: () => prompt.blur(), onClose: () => prompt.focus(),
    respond: async () => { assert.fail("shortcut approved an action") }, applySession() {}, showPasskeyPrompt() { return false },
  })
  h.renderer.keyInput.on("keypress", controller.handleKey)
  try {
    controller.sync()
    prompt.focus()
    h.mockInput.pressKey("g", { ctrl: true })
    assert.equal(controller.isOpen(), true)
    assert.equal(prompt.focused, false)
    assert.equal(prompt.plainText, "draft kept")
    h.mockInput.pressKey("g", { ctrl: true })
    assert.equal(controller.isOpen(), false)
    assert.equal(prompt.focused, true)
    assert.equal(prompt.plainText, "draft kept")
  } finally {
    h.renderer.keyInput.off("keypress", controller.handleKey)
    controller.dispose()
    h.renderer.destroy()
  }
})

test("shared approval command opener handles waiting room, empty session and pending approvals", async () => {
  const h = await createTestRenderer({ width: 80, height: 24, useThread: false })
  const prompt = new TextareaRenderable(h.renderer, { initialValue: "draft kept" })
  h.renderer.root.add(prompt)
  let attached = false
  let interactions: RuntimeInteraction[] = []
  const flashes: string[] = []
  let dispose!: () => void
  const approvals = createRoot(cleanup => {
    dispose = cleanup
    return createCliKernelApprovalComposition({
      client: { onKernelEvent: () => () => {} } as never, renderer: h.renderer,
      session: () => ({ id: "session", agents: [], active_interactions: interactions }) as unknown as RuntimeSession,
      connected: () => true, attached: () => attached, kernelConnected: () => true, notify() {}, attachmentId: () => null,
      flashFooter: (message, tone) => flashes.push(`${tone}:${message}`),
      dimensions: () => ({ width: 80, height: 24 }), themeRevision: () => 0,
      currentFocus: () => prompt, promptFocus: () => prompt,
      closeOtherDialog() {}, applySession() { assert.fail("UI opener sent a response") },
    })
  })
  try {
    prompt.focus()
    approvals.openFromCommand()
    assert.deepEqual(flashes, ["info:start or join a session to view approvals"])
    assert.equal(prompt.focused, true)
    attached = true
    approvals.openFromCommand()
    assert.deepEqual(flashes, ["info:start or join a session to view approvals", "info:No pending approvals"])
    assert.equal(prompt.focused, true)
    interactions = [view.interaction!]
    approvals.openFromCommand()
    assert.equal(approvals.isOpen(), true)
    assert.equal(approvals.view().selected, null)
    assert.equal(prompt.focused, false)
    assert.equal(prompt.plainText, "draft kept")
  } finally { dispose(); h.renderer.destroy() }
})


test("MP-08 / MP-10 / MP-11 general interaction panel renders structured requester metadata", async () => {
  const h = await createTestRenderer({ width: 100, height: 40, useThread: false })
  const box = new BoxRenderable(h.renderer, { position: "absolute", left: 0, top: 0 })
  h.renderer.root.add(box)
  const surface = createKernelApprovalRenderer(h.renderer, { show() {}, choose() {} })
  surface.assign(box)
  try {
    surface.render({ ...view, open: true, interaction: { ...view.interaction!,
      message: "Requester: forged display label",
      requester: { executable: "/opt/outside\nagent", pid: 43, process_start_id: "9876543210123456789", process_exec_version: 0 },
    } }, { width: 100, height: 40 })
    await h.renderOnce()
    const frame = h.captureCharFrame()
    assert.match(frame, /Requester: External program · PID 43/)
    assert.ok(frame.includes('Executable: "/opt/outside\\nagent"'))
    assert.match(frame, /Process start: 9876543210123456789/)
  } finally { h.renderer.destroy() }
})
