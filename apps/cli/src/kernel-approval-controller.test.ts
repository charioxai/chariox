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
function harness() {
  let current = session()
  let connected = true
  let resolve!: (value: RuntimeSession) => void
  let reject!: (error: Error) => void
  const requests: string[][] = []
  const focus: string[] = []
  const controller = createKernelApprovalController({
    getSession: () => current, connected: () => connected, onView() {},
    onOpen: () => focus.push("blur"), onClose: () => focus.push("restore"), scroll() {},
    respond: (...args) => { requests.push(args); return new Promise((yes, no) => { resolve = yes; reject = no }) },
    applySession: (value) => { current = value },
  })
  controller.sync()
  return { controller, requests, focus, resolve: (value: RuntimeSession) => resolve(value),
    reject: () => reject(new Error("untrusted detail")),
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
    dialogOverlayOpen: () => false, closeActiveDialogOverlay: forbidden,
    requestExit: forbidden, requestPromptStop: forbidden, hasActiveTurnWork: () => true,
  })
  let rawKey = "return"
  const raw = createCliStdinKeyController({
    parseKeypress: () => key(rawKey), kernelApprovalOwnsInput: h.controller.ownsInput,
    dialogOverlayOpen: () => false, handleSessionBrowserKey: forbidden,
  } as unknown as CliStdinKeyControllerDeps)
  assert.equal(global.handleKey(key("f8")), true)
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
