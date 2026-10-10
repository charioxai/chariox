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

test("only well-formed passkey prompts reach the popup", () => {
  const malformed = [
    { ...prompt("a"), kind: "unknown" },
    { ...prompt("b"), interaction_id: " " },
    { ...prompt("c"), refuse_choice_id: "approve" },
    { ...prompt("d"), expires_at_ms: "soon" },
    null,
  ]
  assert.deepEqual(passkeyPromptsFromEvent([...malformed, prompt("ok")]), [prompt("ok")])
  assert.deepEqual(passkeyPromptsFromEvent(undefined), [])
})

test("a new prompt needs explicit focus and does not take composer keys", () => {
  const h = harness()
  h.popup.apply([prompt("p1")])
  assert.equal(h.popup.view().open, false)
  assert.deepEqual(h.focus, [])
  assert.equal(h.popup.ownsInput(), false)
  assert.equal(h.popup.handleKey(key("x")), false)
  assert.equal(h.popup.handleKey(key("return")), false)
  assert.deepEqual(h.requests, [])
  assert.equal(h.popup.handleKey(key("f8")), true)
  assert.deepEqual(h.focus, ["open"])
  h.popup.apply([])
  assert.equal(h.popup.view().open, false)
  assert.equal(h.popup.view().count, 0)
  assert.deepEqual(h.focus, ["open", "close"])
})

test("a typed and pasted passkey is kept exactly and sent with the approve choice", async () => {
  const h = harness()
  h.popup.apply([prompt("p1")])
  h.popup.show()
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
  h.popup.show()
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
  h.popup.show()
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
  h.popup.show()
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
  h.popup.show()
  type(h, "half")
  h.popup.handleKey(key("r", { ctrl: true, sequence: "\u0012" }))
  assert.deepEqual(h.requests, [["p1", "deny"]])
  assert.equal(h.popup.view().passkey.length, 0)
})

test("Esc hides the popup and only explicit focus brings it back", () => {
  const h = harness()
  h.popup.apply([prompt("p1")])
  h.popup.show()
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
  assert.equal(h.popup.view().open, false)
  assert.equal(h.popup.view().prompt?.interaction_id, "p1")
  h.popup.handleKey(key("escape"))
  assert.equal(h.popup.show("session-1", "p2"), true)
  assert.equal(h.popup.view().prompt?.interaction_id, "p2")
  assert.equal(h.popup.show("session-1", "unknown"), false)
})

test("this terminal's remember window approves without retyping until the kernel refuses", async () => {
  const h = harness()
  h.popup.apply([prompt("p1"), prompt("p2")])
  h.popup.show()
  type(h, "x")
  h.popup.handleKey(key("tab"))
  assert.equal(h.popup.view().passkey.rememberMinutes, 5)
  h.popup.handleKey(key("return"))
  assert.deepEqual(h.requests[0], ["p1", "approve", { passkey: "x", rememberMinutes: 5 }])
  h.resolve()
  await settle()
  assert.equal(h.popup.view().prompt?.interaction_id, "p2")
  assert.equal(h.popup.view().open, false)
  h.popup.handleKey(key("return"))
  assert.equal(h.requests.length, 1, "remembered presence cannot consume a composer Enter")
  h.popup.show()
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
  h.popup.show()
  h.setConnected(false)
  type(h, "secret")
  h.popup.handleKey(key("return"))
  h.popup.handleKey(key("r", { ctrl: true }))
  assert.equal(h.requests.length, 0)
  assert.equal(h.popup.view().connected, false)
})

test("a passkey typed for one request never answers another that takes its place", async () => {
  const h = harness()
  h.popup.apply([prompt("p1"), prompt("p2")])
  h.popup.show()
  type(h, "for-p1")
  h.popup.handleKey(key("tab"))
  assert.deepEqual(h.popup.view().passkey, { length: 6, rememberMinutes: 5 })
  // Another terminal answered p1: the kernel's set keeps only p2, at index 0.
  h.popup.apply([prompt("p2")])
  assert.equal(h.popup.view().prompt?.interaction_id, "p2")
  assert.deepEqual(h.popup.view().passkey, { length: 0, rememberMinutes: 0 })
  h.popup.handleKey(key("return"))
  assert.deepEqual(h.requests, [], "nothing typed for p2, so nothing is sent")
  // The same explicitly focused request keeps what was typed for it.
  h.popup.show()
  type(h, "for-p2")
  h.popup.apply([prompt("p2"), prompt("p3")])
  assert.equal(h.popup.view().passkey.length, 6)
  // An earlier request leaving moves the shown one, never the entry.
  h.popup.apply([prompt("p0"), prompt("p2"), prompt("p3")])
  assert.equal(h.popup.view().prompt?.interaction_id, "p2")
  assert.equal(h.popup.view().passkey.length, 6)
  h.popup.apply([prompt("p2"), prompt("p3")])
  assert.equal(h.popup.view().prompt?.interaction_id, "p2")
  assert.equal(h.popup.view().passkey.length, 6)
})

test("an answered earlier request leaves the shown one in view", async () => {
  const h = harness()
  h.popup.apply([prompt("p1"), prompt("p2"), prompt("p3")])
  h.popup.show()
  h.popup.handleKey(key("r", { ctrl: true }))
  assert.deepEqual(h.requests, [["p1", "deny"]])
  // The approval panel shows another request while the refusal is pending.
  assert.equal(h.popup.show("session-1", "p3"), true)
  h.resolve()
  await settle()
  assert.equal(h.popup.view().prompt?.interaction_id, "p3")
  assert.equal(h.popup.view().count, 2)
})

test("access grants and extensions require a fresh passkey and let the owner choose a bounded term", async () => {
  const h = harness()
  h.popup.apply([prompt("critical")])
  h.popup.show()
  type(h, "correct")
  h.popup.handleKey(key("tab"))
  h.popup.handleKey(key("return"))
  h.resolve(); await settle()
  for (const kind of ["access_grant", "access_extension"] as const) {
    const access = { ...prompt(kind), kind, lifetime_minutes: 30, max_lifetime_minutes: 45 }
    h.popup.apply([access])
    h.popup.show()
    const count = h.requests.length
    h.popup.handleKey(key("return"))
    await settle()
    assert.equal(h.requests.length, count, "remembered critical presence cannot grant access")
    assert.match(h.popup.view().error!, /Enter your Chariox passkey/)
    h.popup.handleKey(key("tab"))
    assert.equal(h.popup.view().passkey.rememberMinutes, 0)
    assert.equal(h.popup.view().passkey.accessLifetimeMinutes, 45)
    type(h, "fresh")
    h.popup.handleKey(key("return"))
    assert.deepEqual(h.requests.at(-1), [kind, "approve", { passkey: "fresh", rememberMinutes: null, accessLifetimeMinutes: 45 }])
    h.resolve(); await settle()
  }
})

test("MP-08/MP-10/MP-11 A04 sudo asks for a fresh passkey and offers a 1-8 hour window", async () => {
  const h = harness()
  h.popup.apply([prompt("critical")])
  h.popup.show()
  type(h, "correct")
  h.popup.handleKey(key("tab"))
  h.popup.handleKey(key("return"))
  h.resolve(); await settle()
  const sudo = { ...prompt("sudo"), kind: "sudo" as const, lifetime_minutes: 60, max_lifetime_minutes: 480 }
  assert.deepEqual(passkeyPromptsFromEvent([sudo]), [sudo])
  h.popup.apply([sudo])
  h.popup.show()
  const count = h.requests.length
  h.popup.handleKey(key("return"))
  await settle()
  assert.equal(h.requests.length, count, "the remembered critical passkey never mints sudo")
  assert.match(h.popup.view().error!, /Enter your Chariox passkey/)
  assert.equal(h.popup.view().passkey.accessLifetimeMinutes, 60)
  const offered = [60]
  for (let i = 0; i < 4; i++) {
    h.popup.handleKey(key("tab"))
    offered.push(h.popup.view().passkey.accessLifetimeMinutes!)
  }
  assert.deepEqual(offered, [60, 120, 240, 480, 60])
  assert.equal(h.popup.view().passkey.rememberMinutes, 0)
  h.popup.handleKey(key("tab"))
  type(h, "fresh")
  h.popup.handleKey(key("return"))
  assert.deepEqual(h.requests.at(-1), ["sudo", "approve", { passkey: "fresh", rememberMinutes: null, accessLifetimeMinutes: 120 }])
  h.resolve(); await settle()
})
