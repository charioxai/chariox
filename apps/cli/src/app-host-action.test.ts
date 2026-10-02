import assert from "node:assert/strict"
import test from "node:test"
import { acceptAppHostOffer, appHostOperationIds, visibleAppClipboardText } from "./app-host-action.js"
import { handleAppSlashCommand } from "./app-command-handler.js"

for (const copied of [true, false]) {
  test(`explicit accept copies the kernel payload through the asynchronous clipboard helper and renders fallback (${copied})`, async () => {
    const notices: string[] = []
    const text = "a\nb\x1b]52;c;evil\u202e"
    await acceptAppHostOffer("s", "op", async request => {
      assert.deepEqual(request, { AcceptAppHostAction: { session_id: "s", operation_id: "op" } })
      return { AppHostActionAccepted: { operation_id: "op", action: { kind: "clipboard_write", text } } }
    }, message => notices.push(message), { copy: async value => { assert.equal(value, text); if (!copied) throw new Error("unsupported") }, openLink: async () => assert.fail() })
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
      { copy: async () => assert.fail(), openLink: async value => { assert.equal(value, url); return opened } })
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
      { copy: async () => assert.fail(), openLink: async () => assert.fail() }))
  }
})
test("typed host command accepts empty text and requires an attached session before settlement", async () => {
  let copies = 0
  const deps = { currentAppSessionId: () => "s", sendAppRequest: async () => ({ AppHostActionAccepted: { operation_id: "op", action: { kind: "clipboard_write", text: "" } } }),
    appHostTerminal: { copy: async () => { copies++ }, openLink: async () => assert.fail() }, appendNotice: () => {}, flashFooter: () => assert.fail() }
  await handleAppSlashCommand(deps, { kind: "app", raw: "/app host accept op", args: ["host", "accept", "op"] })
  assert.equal(copies, 1)
  await assert.rejects(handleAppSlashCommand(deps, { kind: "app", raw: "/app host accept op extra", args: ["host", "accept", "op", "extra"] }), /usage/)
  await assert.rejects(handleAppSlashCommand({ ...deps, currentAppSessionId: () => undefined }, { kind: "app", raw: "/app host accept op", args: ["host", "accept", "op"] }), /Attach/)
  assert.equal(copies, 1)
})

test("no-ID acceptance resolves only a sole trusted host interaction in the attached session", async () => {
  const operation = "0123456789abcdef0123456789abcdef"
  const interaction = { id: `app_host_${operation}`, kernel_operation_id: `host_action:${operation}`, kind: "permission", level: "warning", requested_at_ms: 1, title: "Copy text", message: "text", choices: [{ id: "decline", label: "Decline", reply: "deny" }] }
  const ids = appHostOperationIds({ id: "s", agents: [], active_interactions: [interaction, { ...interaction, id: "wrong" }, { ...interaction, agent_id: "agent" }, { ...interaction, kernel_operation_id: "file_export:other" }] } as any)
  assert.deepEqual(ids, [operation])
  let sent = 0
  const command = { kind: "app" as const, raw: "/app host accept", args: ["host", "accept"] }
  const deps = { currentAppSessionId: () => "s", currentAppHostOperationIds: () => ids, lastViewedAppHostOperationId: () => operation,
    sendAppRequest: async (request: Record<string, unknown>) => { sent++; assert.deepEqual(request, { AcceptAppHostAction: { session_id: "s", operation_id: operation } }); return { AppHostActionAccepted: { operation_id: operation, action: { kind: "clipboard_write", text: "copy" } } } },
    appHostTerminal: { copy: async () => {}, openLink: async () => assert.fail() }, appendNotice: () => {}, flashFooter: () => assert.fail() }
  await handleAppSlashCommand(deps, command)
  for (const candidates of [[], [operation, "other"]]) {
    await assert.rejects(handleAppSlashCommand({ ...deps, currentAppHostOperationIds: () => candidates }, command), /pending/)
  }
  await assert.rejects(handleAppSlashCommand({ ...deps, currentAppSessionId: () => undefined }, command), /Attach/)
  for (const viewed of [undefined, "expired-offer"]) {
    await assert.rejects(handleAppSlashCommand({ ...deps, lastViewedAppHostOperationId: () => viewed }, command), /review the App host offer/)
  }
  assert.equal(sent, 1)
})

test("userinfo in a returned link never invokes the browser shim", async () => {
  await assert.rejects(acceptAppHostOffer("s", "op", async () => ({ AppHostActionAccepted: { operation_id: "op", action: { kind: "open_link", url: "https://trusted.example@evil.example/" } } }), () => assert.fail(),
    { copy: async () => assert.fail(), openLink: async () => assert.fail() }), /Invalid App link/)
})
