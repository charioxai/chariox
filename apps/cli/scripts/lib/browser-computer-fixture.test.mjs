import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { once } from "node:events"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { startBrowserComputerFixture } from "./browser-computer-fixture.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const fixtureEntrypoint = path.resolve(scriptDir, "..", "live-browser-computer-fixture.mjs")

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

    const unauthenticatedMessages = await fetch(`${fixture.origin}/api/messages`)
    assert.equal(unauthenticatedMessages.status, 403)
    const unauthenticatedUpload = await fetch(`${fixture.origin}/uploads`, { method: "POST", body: "untrusted" })
    assert.equal(unauthenticatedUpload.status, 403)

    const messages = await fetch(`${fixture.origin}/api/messages`, { headers: { cookie } }).then((response) => response.json())
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

    const upload = await fetch(`${fixture.origin}/uploads`, {
      method: "POST",
      headers: { cookie, "content-type": "text/plain" },
      body: "fixture upload",
    })
    assert.equal(upload.status, 200)
    assert.match(await upload.text(), /CHARIOX_FIXTURE_UPLOAD 14/)
    assert.deepEqual(fixture.uploads, [{ contentType: "text/plain", sizeBytes: 14 }])
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

test("fixture entrypoint rejects invalid configuration and stops on SIGTERM", async () => {
  const env = { ...process.env }
  delete env.CHARIOX_BROWSER_FIXTURE_PASSWORD

  const missingPassword = await runEntrypoint(env)
  assert.equal(missingPassword.code, 1)
  assert.match(missingPassword.stderr, /CHARIOX_BROWSER_FIXTURE_PASSWORD is required/)

  const invalidPort = await runEntrypoint({
    ...env,
    CHARIOX_BROWSER_FIXTURE_PASSWORD: "fixture-entrypoint-password",
    CHARIOX_BROWSER_FIXTURE_PORT: "65536",
  })
  assert.notEqual(invalidPort.code, 0)
  assert.match(invalidPort.stderr, /CHARIOX_BROWSER_FIXTURE_PORT must be a valid TCP port/)

  const password = "fixture-entrypoint-password"
  const child = spawn(process.execPath, [fixtureEntrypoint], {
    env: {
      ...env,
      CHARIOX_BROWSER_FIXTURE_PASSWORD: password,
      CHARIOX_BROWSER_FIXTURE_PORT: "0",
    },
    stdio: ["ignore", "pipe", "pipe"],
  })
  let stdout = ""
  let stderr = ""
  child.stdout.on("data", (chunk) => { stdout += chunk })
  child.stderr.on("data", (chunk) => { stderr += chunk })
  try {
    const readyLine = await waitForReadyLine(child, () => stdout)
    const ready = JSON.parse(readyLine.slice(readyLine.indexOf("{")).trim())
    assert.deepEqual(await fetch(`${ready.origin}/health`).then((response) => response.json()), { ok: true })
    assert.doesNotMatch(`${stdout}\n${stderr}`, new RegExp(password))

    child.kill("SIGTERM")
    const [code, signal] = await waitForClose(child)
    assert.equal(code, 0)
    assert.equal(signal, null)
    await assert.rejects(() => fetch(`${ready.origin}/health`), /fetch failed/)
  } finally {
    if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL")
  }
})

async function runEntrypoint(env) {
  const child = spawn(process.execPath, [fixtureEntrypoint], {
    env,
    stdio: ["ignore", "pipe", "pipe"],
  })
  let stdout = ""
  let stderr = ""
  child.stdout.on("data", (chunk) => { stdout += chunk })
  child.stderr.on("data", (chunk) => { stderr += chunk })
  const [code, signal] = await waitForClose(child)
  return { code, signal, stdout, stderr }
}

async function waitForReadyLine(child, readOutput) {
  const timeout = AbortSignal.timeout(5_000)
  while (child.exitCode === null && child.signalCode === null) {
    const nextData = once(child.stdout, "data", { signal: timeout })
    const line = readOutput().split("\n").find((value) => value.startsWith("CHARIOX_BROWSER_COMPUTER_FIXTURE_READY "))
    if (line) return line
    await nextData
  }
  throw new Error(`fixture exited before readiness with code ${child.exitCode}`)
}

async function waitForClose(child) {
  if (child.exitCode !== null || child.signalCode !== null) return [child.exitCode, child.signalCode]
  return await once(child, "close", { signal: AbortSignal.timeout(5_000) })
}
