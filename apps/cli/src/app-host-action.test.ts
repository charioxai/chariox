import assert from "node:assert/strict"
import test from "node:test"
import { acceptAppHostOffer, visibleAppClipboardText } from "./app-host-action.js"
import { handleAppSlashCommand } from "./app-command-handler.js"

for (const copied of [true, false]) {
  test(`explicit accept copies the kernel payload through OSC 52 and renders fallback (${copied})`, async () => {
    const notices: string[] = []
    const text = "a\nb\x1b]52;c;evil\u202e"
    await acceptAppHostOffer("s", "op", async request => {
      assert.deepEqual(request, { AcceptAppHostAction: { session_id: "s", operation_id: "op" } })
      return { AppHostActionAccepted: { operation_id: "op", action: { kind: "clipboard_write", text } } }
    }, message => notices.push(message), { copyOSC52: value => { assert.equal(value, text); return copied }, openLink: async () => assert.fail() })
    assert.equal(notices.length, 1)
    assert.match(notices[0]!, /this text:/)
    assert.equal(notices[0]!.includes("\x1b"), false)
    assert.equal(notices[0]!.includes("\u202e"), false)
    assert.equal(JSON.parse(visibleAppClipboardText(text)), text)
  })
}
for (const opened of [true, false]) {
  test(`explicit accept opens the exact http URL or prints it (${opened})`, async () => {
    const url = "https://example.org/a?x=%20&y=2#z"
    const notices: string[] = []
    await acceptAppHostOffer("s", "op", async () => ({ AppHostActionAccepted: { operation_id: "op", action: { kind: "open_link", url } } }), message => notices.push(message),
      { copyOSC52: () => assert.fail(), openLink: async value => { assert.equal(value, url); return opened } })
    assert.equal(notices[0]!.endsWith(url), true)
  })
}
test("declined, expired, foreign and malformed offers never act", async () => {
  for (const response of [
    { AppRequestFailed: { code: "not_found" } },
    { AppHostActionAccepted: { operation_id: "other", action: { kind: "clipboard_write", text: "no" } } },
    { AppHostActionAccepted: { operation_id: "op", action: { kind: "open_link", url: "file:///tmp/no" } } },
  ]) {
    await assert.rejects(acceptAppHostOffer("s", "op", async () => response, () => assert.fail(),
      { copyOSC52: () => assert.fail(), openLink: async () => assert.fail() }))
  }
})
test("typed host command requires an operation and attached session before settlement", async () => {
  let copies = 0
  const deps = { currentAppSessionId: () => "s", sendAppRequest: async () => ({ AppHostActionAccepted: { operation_id: "op", action: { kind: "clipboard_write", text: "" } } }),
    appHostTerminal: { copyOSC52: () => { copies++; return false }, openLink: async () => assert.fail() }, appendNotice: () => {}, flashFooter: () => assert.fail() }
  await handleAppSlashCommand(deps, { kind: "app", raw: "/app host accept op", args: ["host", "accept", "op"] })
  assert.equal(copies, 1)
  await assert.rejects(handleAppSlashCommand(deps, { kind: "app", raw: "/app host accept op extra", args: ["host", "accept", "op", "extra"] }), /usage/)
  await assert.rejects(handleAppSlashCommand({ ...deps, currentAppSessionId: () => undefined }, { kind: "app", raw: "/app host accept op", args: ["host", "accept", "op"] }), /Attach/)
  assert.equal(copies, 1)
})
