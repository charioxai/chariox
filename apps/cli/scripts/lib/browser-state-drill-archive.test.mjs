import assert from "node:assert/strict"
import test from "node:test"
import { createBrowserStateArchiveFixture } from "./browser-state-drill-archive.mjs"

test("MP-03/MP-10: operator archive fixture requires explicit local DEV and absolute helper", () => {
  assert.throws(() => createBrowserStateArchiveFixture({ helper: "/owned/fixture", localDev: false }), /local DEV/)
  assert.throws(() => createBrowserStateArchiveFixture({ helper: "relative", localDev: true }), /absolute/)
})

test("MP-03/MP-10: operator operations use stdin metadata and retain fail-closed receipts", async () => {
  const calls = []
  let response = { sizeBytes: 123, sha256: "a".repeat(64), private: true, readableArchive: true }
  const fixture = createBrowserStateArchiveFixture({ helper: "/owned/fixture", localDev: true,
    runCommand: async (...args) => { calls.push(args); return { code: 0, stdout: JSON.stringify(response) } } })
  const state = { id: "state-1" }
  assert.equal((await fixture.verify(state)).sizeBytes, 123)
  assert.deepEqual(calls[0].slice(0, 2), ["sudo", ["-n", "/usr/bin/python3", "/owned/fixture", "verify"]])
  assert.deepEqual(JSON.parse(calls[0][2].stdin), state)
  response = { corrupted: true }
  await fixture.corrupt(state)
  response = { archivePresent: false, quarantineCount: 1 }
  await fixture.verifyQuarantine(state)
  response = { archivePresent: true, quarantineCount: 0 }
  await assert.rejects(fixture.verifyQuarantine(state))
  response = { private: false }
  await assert.rejects(fixture.verify(state))
})

test("MP-03/MP-10: operator refusal cannot fall back to unprivileged archive access", async () => {
  const fixture = createBrowserStateArchiveFixture({ helper: "/owned/fixture", localDev: true,
    runCommand: async () => ({ code: 1, stdout: "", stderr: "private tool detail" }) })
  await assert.rejects(fixture.verify({}), error => !error.message.includes("private tool detail"))
})
