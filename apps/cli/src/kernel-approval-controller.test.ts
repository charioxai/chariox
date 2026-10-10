import assert from "node:assert/strict"
import test from "node:test"
import type { RuntimeInteraction, RuntimeSession } from "./cli-types.js"
import { createKernelApprovalController, kernelApprovals, type KernelApprovalKey } from "./kernel-approval-controller.js"
import { createGlobalKeyboardShortcutController } from "./global-keyboard-shortcut-controller.js"
import { createCliStdinKeyController, type CliStdinKeyControllerDeps } from "./cli-stdin-key-controller.js"

export const approvalFixture: RuntimeInteraction = {
  id: "approval-1", kernel_operation_id: "install-1", kind: "permission", level: "warning",
  title: "Install Linear", message: "Allow this App to use the capabilities listed in the installation?",
  choices: [{ id: "deny", label: "Deny", reply: "deny" }, { id: "allow", label: "Allow", reply: "allow" }],
  requested_at_ms: 1,
}
const session = (id = "session-1", items: RuntimeInteraction[] = [approvalFixture]) => ({
  id, agents: [], active_interactions: items,
}) as unknown as RuntimeSession
const key = (name: string, extra: Partial<KernelApprovalKey> = {}): KernelApprovalKey => ({
  name, preventDefault() { this.defaultPrevented = true }, stopPropagation() {}, ...extra,
})
function harness(initial = session(), popupHas = true) {
  let current = initial
  let connected = true
  let resolve!: (value: RuntimeSession) => void
  let reject!: (error: Error) => void
  const requests: unknown[][] = []
  const popups: string[][] = []
  const focus: string[] = []
  const controller = createKernelApprovalController({
    getSession: () => current, connected: () => connected, onView() {},
    onOpen: () => focus.push("blur"), onClose: () => focus.push("restore"), scroll() {},
    respond: (...args) => { requests.push(args); return new Promise((yes, no) => { resolve = yes; reject = no }) },
    applySession: (value) => { current = value },
    showPasskeyPrompt: (...args) => { popups.push(args); return popupHas },
  })
  controller.sync()
  return { controller, requests, popups, focus, resolve: (value: RuntimeSession) => resolve(value),
    reject: (message = "untrusted detail") => reject(new Error(message)),
    current: () => current,
    setSession(value: RuntimeSession) { current = value; controller.sync() },
    setConnected(value: boolean) { connected = value; controller.sync() },
  }
}

test("zero-agent kernel approvals require explicit opening and selection before Enter", async () => {
  const h = harness()
  assert.equal(h.controller.view().count, 1)
  assert.equal(h.controller.handleKey(key("return")), false)
  h.controller.handleKey(key("f8"))
  h.controller.handleKey(key("return"))
  h.controller.handleKey(key("1"))
  assert.equal(h.requests.length, 0)
  h.controller.handleKey(key("down"))
  h.controller.handleKey(key("return", { eventType: "repeat" }))
  assert.equal(h.requests.length, 0)
  h.controller.handleKey(key("return"))
  assert.deepEqual(h.requests, [["session-1", "approval-1", "deny"]])
  h.controller.handleKey(key("return"))
  assert.equal(h.requests.length, 1)
  h.resolve(session("session-1", []))
  await new Promise<void>((resolve) => queueMicrotask(resolve))
  assert.equal(h.controller.view().open, false)
  assert.deepEqual(h.focus, ["blur", "restore"])
})

test("dismissal, disconnect and switching pending approvals never reuse a selected choice", async () => {
  const h = harness()
  h.setSession(session("session-1", [approvalFixture, { ...approvalFixture, id: "approval-2" }]))
  h.controller.show()
  h.controller.handleKey(key("down"))
  h.controller.handleKey(key("right"))
  h.controller.handleKey(key("return"))
  assert.equal(h.requests.length, 0)
  h.controller.handleKey(key("escape"))
  assert.equal(h.controller.view().count, 2)
  h.controller.show()
  h.setConnected(false)
  await h.controller.choose("approval-1", "allow")
  assert.equal(h.requests.length, 0)
  h.setConnected(true)
  await h.controller.choose("approval-1", "undeclared")
  assert.equal(h.requests.length, 0)
})

test("late response cannot reapply the previous session, even after switching back", async () => {
  const h = harness()
  h.controller.show()
  const response = h.controller.choose("approval-1", "allow")
  h.setSession(session("other"))
  h.setSession(session())
  h.resolve(session("session-1", []))
  await response
  assert.equal(h.current().active_interactions?.length, 1)
  assert.equal(h.controller.isOpen(), false)
})

test("ambiguous responses keep approval pending and never expose transport payloads", async () => {
  const h = harness()
  h.controller.show()
  const response = h.controller.choose("approval-1", "allow")
  h.reject()
  await response
  assert.equal(h.controller.view().count, 1)
  assert.match(h.controller.view().error ?? "", /did not confirm/)
  assert.doesNotMatch(h.controller.view().error ?? "", /untrusted detail/)
})

test("only exact kernel permission subjects appear in the independent surface", () => {
  const malformed = [
    { ...approvalFixture, agent_id: "agent-1" },
    { ...approvalFixture, kernel_operation_id: " " },
    { ...approvalFixture, kind: "choice" },
    { ...approvalFixture, default_on_timeout: "allow" },
    { ...approvalFixture, custom_choice: {} },
  ] as unknown as RuntimeInteraction[]
  assert.deepEqual(kernelApprovals(session("session-1", malformed)), [])
})

test("global dialog keys and duplicate raw bytes cannot reach agent or prompt shortcuts", async () => {
  const h = harness()
  const forbidden = () => { throw new Error("reached an agent shortcut") }
  const global = createGlobalKeyboardShortcutController({
    handleKernelApprovalKey: h.controller.handleKey, handleHotkeysToggleShortcut: forbidden,
    dialogOverlayOpen: () => false,
    requestExit: forbidden, requestPromptStop: forbidden, hasActiveTurnWork: () => true,
  })
  let rawKey = "return"
  let rawCtrl = false
  const raw = createCliStdinKeyController({
    createStdinParser: () => ({ push: () => {}, drain: (onEvent: (event: { type: string; key: unknown }) => void) => onEvent({ type: "key", key: key(rawKey, { ctrl: rawCtrl }) }) }),
    kernelApprovalOwnsInput: h.controller.ownsInput,
    dialogOverlayOpen: () => false, handleSessionBrowserKey: forbidden,
  } as unknown as CliStdinKeyControllerDeps)
  rawKey = "g"; rawCtrl = true
  assert.equal(raw.handleData("\x07"), true) // Raw bytes never reach other shortcut handlers.
  assert.equal(global.handleKey(key("g", { ctrl: true })), true)
  rawCtrl = false
  for (const name of ["return", "tab", "1", "e", "c"]) {
    global.handleKey(key(name, { ctrl: true }))
    rawKey = name
    assert.equal(raw.handleData(name), true)
  }
  global.handleKey(key("escape"))
  rawKey = "escape"
  assert.equal(raw.handleData("escape"), true)
  assert.equal(h.requests.length, 0)
  await new Promise<void>((resolve) => queueMicrotask(resolve))
  assert.equal(h.controller.ownsInput(), false)
})

test("a stale rendered choice cannot authorize the next pending interaction", async () => {
  const h = harness()
  h.controller.show()
  h.setSession(session("session-1", [{ ...approvalFixture, id: "replacement" }]))
  await h.controller.choose("approval-1", "allow")
  assert.equal(h.requests.length, 0)
})

const critical = (id: string): RuntimeInteraction => ({
  id, kernel_operation_id: `validation:${id}`, kind: "permission", level: "warning",
  title: "Approve App action", message: "An App asks to perform a protected action.",
  choices: [{ id: "deny", label: "Deny", reply: "deny" },
    { id: "approve", label: "Approve", reply: "allow", requires_passkey: true }],
  requested_at_ms: 1,
})
const selectApprove = (h: ReturnType<typeof harness>) => {
  h.controller.handleKey(key("down"))
  h.controller.handleKey(key("down"))
  h.controller.handleKey(key("return"))
}

test("a critical approval is approved only in the passkey popup", () => {
  const h = harness(session("session-1", [critical("c1")]))
  h.controller.handleKey(key("f8"))
  selectApprove(h)
  assert.deepEqual(h.popups, [["session-1", "c1"]])
  assert.equal(h.requests.length, 0)
  assert.equal(h.controller.view().error, null)
  assert.equal(h.controller.view().open, true)
})

test("another user's critical approval says where it is answered and sends nothing", () => {
  const h = harness(session("session-1", [critical("c1")]), false)
  h.controller.handleKey(key("f8"))
  selectApprove(h)
  assert.equal(h.requests.length, 0)
  assert.match(h.controller.view().error ?? "", /passkey popup/)
})

test("deny and routine approvals are answered from the panel without the passkey", async () => {
  const h = harness(session("session-1", [critical("c1")]))
  h.controller.handleKey(key("f8"))
  h.controller.handleKey(key("down"))
  h.controller.handleKey(key("return"))
  assert.deepEqual(h.requests, [["session-1", "c1", "deny"]])
  assert.deepEqual(h.popups, [])
})

test("an open panel takes every paste, so none reaches the prompt", () => {
  const h = harness(session("session-1", [critical("c1")]))
  const events: string[] = []
  const paste = { preventDefault: () => { events.push("prevent") }, stopPropagation: () => { events.push("stop") } }
  assert.equal(h.controller.handlePaste(paste), false, "a closed panel leaves pastes to the prompt")
  h.controller.handleKey(key("f8"))
  assert.equal(h.controller.handlePaste(paste), true)
  assert.deepEqual(events, ["prevent", "stop"])
})

test("no-ID host acceptance is bound to the offer displayed before dismissal", () => {
  const h = harness()
  const host = (operation: string): RuntimeInteraction => ({ ...approvalFixture, id: `app_host_${operation}`, kernel_operation_id: `host_action:${operation}`, choices: [{ id: "decline", label: "Decline", reply: "deny" }] })
  const a = host("0123456789abcdef0123456789abcdef")
  const b = host("fedcba9876543210fedcba9876543210")
  h.setSession(session("session-1", [a]))
  assert.equal(h.controller.lastViewedAppHostOperationId(), undefined)
  h.controller.show()
  h.controller.close()
  assert.equal(h.controller.lastViewedAppHostOperationId(), "0123456789abcdef0123456789abcdef")
  h.setSession(session("session-1", [{ ...a, message: "changed payload" }]))
  assert.equal(h.controller.lastViewedAppHostOperationId(), undefined)
  h.setSession(session("session-1", [a]))
  // A expires while the human types; the queued B is projected, panel closed.
  h.setSession(session("session-1", [b]))
  assert.equal(h.controller.lastViewedAppHostOperationId(), undefined)
  assert.equal(h.controller.isOpen(), false)
  h.controller.show()
  h.controller.close()
  assert.equal(h.controller.lastViewedAppHostOperationId(), "fedcba9876543210fedcba9876543210")
  h.setSession(session("other", [b]))
  assert.equal(h.controller.lastViewedAppHostOperationId(), undefined)
  h.setSession(session("session-1", [b]))
  assert.equal(h.controller.lastViewedAppHostOperationId(), undefined)
})

test("Ctrl+G and F8 explicitly open and dismiss approvals without accepting or changing the draft", () => {
  for (const shortcut of [key("g", { ctrl: true }), key("f8")]) {
    const h = harness()
    assert.deepEqual(h.focus, [])
    assert.equal(h.controller.handleKey(shortcut), true)
    assert.equal(h.controller.isOpen(), true)
    assert.equal(h.controller.view().selected, null)
    assert.deepEqual(h.requests, [])
    assert.equal(h.controller.handleKey({ ...shortcut, defaultPrevented: false, eventType: "repeat" }), true)
    assert.equal(h.controller.isOpen(), true)
    h.controller.handleKey({ ...shortcut, defaultPrevented: false })
    assert.equal(h.controller.isOpen(), false)
    assert.deepEqual(h.focus, ["blur", "restore"])
    assert.deepEqual(h.requests, [])
  }
})

test("approval shortcuts ignore modifiers, repeats and releases and empty queues", () => {
  for (const event of [key("g"), key("g", { ctrl: true, alt: true }), key("g", { ctrl: true, shift: true }), key("t", { ctrl: true })]) {
    assert.equal(harness().controller.handleKey(event), false)
  }
  for (const eventType of ["repeat", "release"]) {
    const h = harness()
    h.controller.handleKey(key("g", { ctrl: true, eventType }))
    assert.equal(h.controller.isOpen(), false)
  }
  assert.equal(harness(session("session-1", [])).controller.handleKey(key("g", { ctrl: true })), false)
})

test("critical approvals anywhere in the queue are visible without stealing focus", () => {
  const h = harness(session("session-1", [approvalFixture, {
    ...approvalFixture, id: "critical", level: "critical", requested_at_ms: 2,
    choices: [{ id: "allow", label: "Approve", reply: "allow", requires_passkey: true }],
  }]))
  assert.equal(h.controller.view().criticalCount, 1)
  assert.equal(h.controller.view().interaction?.id, "approval-1")
  assert.deepEqual(h.focus, [])
})
