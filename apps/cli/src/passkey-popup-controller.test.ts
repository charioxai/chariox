import assert from "node:assert/strict"
import test from "node:test"
import type { PasskeyPrompt } from "@chariox/kernel-client/kernel-types"
import { createPasskeyPopupController, passkeyPromptsFromEvent, type PasskeyPopupKey } from "./passkey-popup-controller.js"

const prompt = (id: string, sessionId = "session-1"): PasskeyPrompt => ({
  kind: "critical_approval", session_id: sessionId, interaction_id: id,
  title: "Approve App action", message: "An App asks to perform a protected action.",
  approve_choice_id: "approve", refuse_choice_id: "deny", requested_at_ms: 1, expires_at_ms: 300_001,
})
const key = (name: string, extra: Partial<PasskeyPopupKey> = {}): PasskeyPopupKey => ({
  name, preventDefault() { this.defaultPrevented = true }, stopPropagation() {}, ...extra,
})
const paste = (text: string, rawText: string | null = null) => {
  const events: string[] = []
  return { events, event: { text, rawText,
    preventDefault: () => { events.push("prevent") }, stopPropagation: () => { events.push("stop") } } }
}
const settle = () => new Promise<void>((resolve) => setTimeout(resolve, 0))
const type = (h: ReturnType<typeof harness>, text: string) => {
  for (const sequence of Array.from(text)) h.popup.handleKey(key(sequence === " " ? "space" : sequence, { sequence }))
}

function harness() {
  let connected = true
  let resolve!: () => void
  let reject!: (error: Error) => void
  const requests: unknown[][] = []
  const notices: string[] = []
  const focus: string[] = []
  const popup = createPasskeyPopupController({
    connected: () => connected, onView() {},
    onOpen: () => focus.push("open"), onClose: () => focus.push("close"), scroll() {},
    respond: (prompt, choiceId, proof) => {
      requests.push(proof ? [prompt.interaction_id, choiceId, proof] : [prompt.interaction_id, choiceId])
      return new Promise<void>((yes, no) => { resolve = yes; reject = no })
    },
    notify: (message) => notices.push(message),
  })
  return { popup, requests, notices, focus,
    resolve: () => resolve(),
    reject: (message: string) => reject(new Error(message)),
    setConnected(value: boolean) { connected = value },
  }
}

test("only well-formed critical approval prompts reach the popup", () => {
  const malformed = [
    { ...prompt("a"), kind: "sudo" },
    { ...prompt("b"), interaction_id: " " },
    { ...prompt("c"), refuse_choice_id: "approve" },
    { ...prompt("d"), expires_at_ms: "soon" },
    null,
  ]
  assert.deepEqual(passkeyPromptsFromEvent([...malformed, prompt("ok")]), [prompt("ok")])
  assert.deepEqual(passkeyPromptsFromEvent(undefined), [])
})

test("a prompt opens the popup on its own, and the kernel closing it closes the popup", () => {
  const h = harness()
  h.popup.apply([prompt("p1")])
  assert.equal(h.popup.view().open, true)
  assert.deepEqual(h.focus, ["open"])
  assert.equal(h.popup.ownsInput(), true)
  // Answered on another terminal: the kernel's set no longer has it.
  h.popup.apply([])
  assert.equal(h.popup.view().open, false)
  assert.equal(h.popup.view().count, 0)
  assert.deepEqual(h.focus, ["open", "close"])
})

test("a typed and pasted passkey is kept exactly and sent with the approve choice", async () => {
  const h = harness()
  h.popup.apply([prompt("p1")])
  h.popup.handleKey(key("x", { shift: true, sequence: "X" }))
  h.popup.handleKey(key("\u{1D11E}", { sequence: "\u{1D11E}" }))
  h.popup.handleKey(key("1", { shift: true, sequence: "!" }))
  h.popup.handleKey(key("backspace"))
  h.popup.handleKey(key("backspace"))
  h.popup.handleKey(key("\u{1D11E}", { sequence: "\u{1D11E}" }))
  assert.equal(h.popup.handlePaste(paste("Pa$te Ünï\u{1F511} \n").event), true)
  // OpenTUI strips the escape sequence from `text`; the raw paste keeps it.
  h.popup.handlePaste(paste("AbCd", "Ab\u001b[31mCd").event)
  assert.match(h.popup.view().error ?? "", /^Paste not added/)
  h.popup.handlePaste(paste("first\nsecond").event)
  h.popup.handlePaste(paste("tab\there").event)
  h.popup.handlePaste(paste("y".repeat(512)).event)
  assert.deepEqual(h.popup.view().passkey, { length: 13, rememberMinutes: 0 })
  assert.equal(JSON.stringify(h.popup.view()).includes("Pa$te"), false, "the view holds only the length")
  h.popup.handleKey(key("return"))
  assert.deepEqual(h.requests, [["p1", "approve", { passkey: "X\u{1D11E}Pa$te Ünï\u{1F511} ", rememberMinutes: null }]])
  assert.equal(h.popup.view().passkey.length, 0, "the passkey is dropped once sent")
  h.resolve()
  await settle()
  assert.equal(h.popup.view().open, false, "answered here, it closes before the kernel's next set")
  h.popup.apply([prompt("p1")])
  assert.equal(h.popup.view().open, false, "a set sent before the answer cannot reopen it")
})

test("a terminal control sequence typed into the passkey clears it and the rest of that input", async () => {
  const h = harness()
  h.popup.apply([prompt("p1")])
  // An unbracketed paste of styled text, as OpenTUI parses it into keys.
  h.popup.handleKey(key("a", { sequence: "a" }))
  h.popup.handleKey(key("", { sequence: "\u001b[31m", ctrl: true, meta: true, alt: true }))
  h.popup.handleKey(key("b", { sequence: "b" }))
  h.popup.handleKey(key("return", { sequence: "\r" }))
  assert.equal(h.requests.length, 0)
  assert.equal(h.popup.view().passkey.length, 0)
  assert.match(h.popup.view().error ?? "", /control sequence/)
  await settle()
  type(h, "o")
  h.popup.handleKey(key("return"))
  assert.deepEqual(h.requests, [["p1", "approve", { passkey: "o", rememberMinutes: null }]])
})

test("a wrong passkey keeps the popup open and asks again", async () => {
  const h = harness()
  h.popup.apply([prompt("p1")])
  type(h, "guess")
  h.popup.handleKey(key("return"))
  h.reject("local transport `critical approval` failed: PASSKEY_REJECTED: the passkey is not correct")
  await settle()
  assert.equal(h.popup.view().open, true)
  assert.equal(h.popup.view().error, "That passkey is not correct.")
  assert.equal(h.popup.view().passkey.length, 0)
  h.popup.handleKey(key("return"))
  assert.equal(h.requests.length, 1, "an empty passkey is not sent")
  assert.equal(h.popup.view().error, "Enter your Chariox passkey to approve.")
})

test("a later answer is told it was already answered and the popup closes", async () => {
  const h = harness()
  h.popup.apply([prompt("p1"), prompt("p2")])
  type(h, "right")
  h.popup.handleKey(key("return"))
  h.reject("PASSKEY_ALREADY_ANSWERED: already answered")
  await settle()
  assert.deepEqual(h.notices, ["This passkey request was already answered."])
  assert.equal(h.popup.view().prompt?.interaction_id, "p2")
  assert.equal(h.popup.view().error, null)
})

test("Ctrl+R refuses with the refuse choice and no passkey", async () => {
  const h = harness()
  h.popup.apply([prompt("p1")])
  type(h, "half")
  h.popup.handleKey(key("r", { ctrl: true, sequence: "\u0012" }))
  assert.deepEqual(h.requests, [["p1", "deny"]])
  assert.equal(h.popup.view().passkey.length, 0)
})

test("Esc hides the popup, F8 and a new prompt bring it back", () => {
  const h = harness()
  h.popup.apply([prompt("p1")])
  type(h, "secret")
  h.popup.handleKey(key("escape"))
  assert.equal(h.popup.view().open, false)
  assert.equal(h.popup.view().count, 1, "the prompt stays pending in the kernel")
  assert.equal(h.popup.view().passkey.length, 0, "hiding drops the typed passkey")
  assert.equal(h.popup.handleKey(key("x")), false, "a hidden popup takes no keys")
  assert.equal(h.popup.handleKey(key("f8")), true)
  assert.equal(h.popup.view().open, true)
  h.popup.handleKey(key("escape"))
  h.popup.apply([prompt("p1")])
  assert.equal(h.popup.view().open, false, "the same set does not reopen it")
  h.popup.apply([prompt("p1"), prompt("p2")])
  assert.equal(h.popup.view().open, true)
  assert.equal(h.popup.view().prompt?.interaction_id, "p1")
  h.popup.handleKey(key("escape"))
  assert.equal(h.popup.show("session-1", "p2"), true)
  assert.equal(h.popup.view().prompt?.interaction_id, "p2")
  assert.equal(h.popup.show("session-1", "unknown"), false)
})

test("this terminal's remember window approves without retyping until the kernel refuses", async () => {
  const h = harness()
  h.popup.apply([prompt("p1"), prompt("p2")])
  type(h, "x")
  h.popup.handleKey(key("tab"))
  assert.equal(h.popup.view().passkey.rememberMinutes, 5)
  h.popup.handleKey(key("return"))
  assert.deepEqual(h.requests[0], ["p1", "approve", { passkey: "x", rememberMinutes: 5 }])
  h.resolve()
  await settle()
  assert.equal(h.popup.view().prompt?.interaction_id, "p2")
  h.popup.handleKey(key("return"))
  assert.deepEqual(h.requests[1], ["p2", "approve"])
  h.reject("PASSKEY_REQUIRED: approving this critical action needs your Chariox passkey")
  await settle()
  assert.equal(h.popup.view().error, "Enter your Chariox passkey to approve.")
  h.popup.handleKey(key("return"))
  assert.equal(h.requests.length, 2)
})

test("a disconnected terminal sends nothing", () => {
  const h = harness()
  h.popup.apply([prompt("p1")])
  h.setConnected(false)
  type(h, "secret")
  h.popup.handleKey(key("return"))
  h.popup.handleKey(key("r", { ctrl: true }))
  assert.equal(h.requests.length, 0)
  assert.equal(h.popup.view().connected, false)
})
