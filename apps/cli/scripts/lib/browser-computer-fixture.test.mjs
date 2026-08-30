import assert from "node:assert/strict"
import test from "node:test"
import { startBrowserComputerFixture } from "./browser-computer-fixture.mjs"

test("fixture exercises persisted browser state and difficult browser structures", async () => {
  const fixture = await startBrowserComputerFixture({ password: "fixture-test-password" })
  try {
    const health = await fetch(`${fixture.origin}/health`).then((response) => response.json())
    assert.deepEqual(health, { ok: true })

    const state = await fetch(`${fixture.origin}/state/seed?cookie=cookie-marker&local=local-marker&idb=idb-marker&cache=cache-marker`).then((response) => response.text())
    assert.match(state, /document\.cookie/)
    assert.match(state, /localStorage\.setItem/)
    assert.match(state, /indexedDB\.open/)
    assert.match(state, /caches\.open/)
    assert.match(state, /serviceWorker\.register/)
    assert.match(state, /cookie-marker/)

    const escapedState = await fetch(`${fixture.origin}/state/seed?cookie=%3C%2Fscript%3E`).then((response) => response.text())
    assert.doesNotMatch(escapedState, /const markers = \{"cookie":"<\/script>/)
    assert.ok(escapedState.includes("\\u003c/script>"))

    const check = await fetch(`${fixture.origin}/state/check`).then((response) => response.text())
    assert.match(check, /chariox_fixture_state/)
    assert.match(check, /chariox_fixture_local/)
    assert.match(check, /chariox_fixture_state/)
    assert.match(check, /serviceWorker/)

    const worker = await fetch(`${fixture.origin}/service-worker.js`).then((response) => response.text())
    assert.match(worker, /CHARIOX|chariox-fixture-worker-v1/)
    assert.match(worker, /offline-marker/)

    const interactions = await fetch(`${fixture.origin}/interactions`).then((response) => response.text())
    assert.match(interactions, /attachShadow/)
    assert.match(interactions, /\/frames\/outer/)
    assert.match(interactions, /confirm\(/)
    assert.match(interactions, /prompt\(/)
    assert.match(interactions, /target="_blank"/)
    assert.match(interactions, /\/downloads\/sample\.txt/)
    assert.match(interactions, /type="file"/)

    const outer = await fetch(`${fixture.origin}/frames/outer`).then((response) => response.text())
    const inner = await fetch(`${fixture.origin}/frames/inner`).then((response) => response.text())
    assert.match(outer, /CHARIOX_FIXTURE_OUTER_FRAME/)
    assert.match(outer, /\/frames\/inner/)
    assert.match(inner, /CHARIOX_FIXTURE_INNER_FRAME/)
  } finally {
    await fixture.close()
  }
})

test("fixture provides authenticated mail without exposing its password", async () => {
  const password = "fixture-test-password"
  const fixture = await startBrowserComputerFixture({ password })
  try {
    const invalid = await fetch(`${fixture.origin}/mail/login`, {
      method: "POST",
      body: new URLSearchParams({ email: fixture.account, password: "wrong" }),
      redirect: "manual",
    })
    assert.equal(invalid.status, 401)

    const login = await fetch(`${fixture.origin}/mail/login`, {
      method: "POST",
      body: new URLSearchParams({ email: fixture.account, password }),
      redirect: "manual",
    })
    assert.equal(login.status, 303)
    assert.equal(login.headers.get("location"), "/mail/inbox")
    const cookie = login.headers.get("set-cookie")
    assert.match(cookie, /^chariox_fixture_session=/)
    assert.doesNotMatch(cookie, new RegExp(password))

    const inbox = await fetch(`${fixture.origin}/mail/inbox`, { headers: { cookie } })
    assert.equal(inbox.status, 200)
    assert.match(await inbox.text(), /CHARIOX_FIXTURE_INBOX/)

    const sent = await fetch(`${fixture.origin}/mail/send`, {
      method: "POST",
      headers: { cookie },
      body: new URLSearchParams({
        to: "recipient@chariox.test",
        subject: "Fixture subject",
        body: "Fixture body",
      }),
    })
    assert.equal(sent.status, 200)
    assert.match(await sent.text(), /CHARIOX_FIXTURE_MESSAGE_SENT/)

    const messages = await fetch(`${fixture.origin}/api/messages`).then((response) => response.json())
    assert.equal(messages.messages.length, 1)
    assert.deepEqual(messages.messages[0], {
      id: "message-1",
      from: fixture.account,
      to: "recipient@chariox.test",
      subject: "Fixture subject",
      body: "Fixture body",
      sentAt: messages.messages[0].sentAt,
    })
    assert.equal(fixture.messages.length, 1)
  } finally {
    await fixture.close()
  }
})

test("fixture validates required credentials and releases its listener", async () => {
  await assert.rejects(() => startBrowserComputerFixture(), /password is required/)
  const fixture = await startBrowserComputerFixture({ password: "fixture-test-password" })
  const origin = fixture.origin
  await fixture.close()
  await assert.rejects(() => fetch(`${origin}/health`), /fetch failed/)
})
