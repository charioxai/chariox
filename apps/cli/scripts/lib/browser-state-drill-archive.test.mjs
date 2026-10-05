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
  response = { archivePresent: true, quarantineCount: 0 }
  await fixture.verifyRejectedArchive(state)
  assert.match("backup `backup-1` archive integrity check failed: managed saved home archive metadata is invalid",
    fixture.corruptionRejection)
  assert.match("backup `backup-1` managed archive integrity check failed; broker-owned archive was left unchanged",
    fixture.corruptionRejection)
  response = { archivePresent: false, quarantineCount: 1 }
  await assert.rejects(fixture.verifyRejectedArchive(state))
  response = { private: false }
  await assert.rejects(fixture.verify(state))
})

test("MP-08/MP-10: unprotected archive rejection retains quarantine expectations", async t => {
  const {mkdtemp, writeFile, rm, rename} = await import("node:fs/promises")
  const path = await import("node:path")
  const root = await mkdtemp(path.join(process.env.CHARIOX_HOME ?? process.env.HOME, ".chariox-quarantine-test-"))
  t.after(() => rm(root, {recursive: true, force: true}))
  const state = {home_archive_path: path.join(root, "home.tar.zst")}
  await writeFile(state.home_archive_path, "synthetic archive")
  const fixture = createBrowserStateArchiveFixture({localDev: false})
  await assert.rejects(fixture.verifyRejectedArchive(state))
  await rename(state.home_archive_path, `${state.home_archive_path}.corrupt-fixture`)
  await fixture.verifyRejectedArchive(state)
  assert.match("archive integrity check failed; corrupt archive quarantined", fixture.corruptionRejection)
  assert.doesNotMatch("archive integrity check failed: managed saved home archive metadata is invalid",
    fixture.corruptionRejection)
})

test("MP-03/MP-10: operator refusal cannot fall back to unprivileged archive access", async () => {
  const fixture = createBrowserStateArchiveFixture({ helper: "/owned/fixture", localDev: true,
    runCommand: async () => ({ code: 1, stdout: "", stderr: "private tool detail" }) })
  await assert.rejects(fixture.verify({}), error => !error.message.includes("private tool detail"))
})

test("MP-03/MP-10: corruption waits only for the root operator pin, then executes once", async () => {
  const calls = [], waits = []
  const fixture = createBrowserStateArchiveFixture({helper: "/owned/fixture", localDev: true,
    wait: async ms => { waits.push(ms) },
    runCommand: async (...args) => {
      calls.push(args)
      return calls.length === 1 ? {code: 2, stdout: ""} : {code: 0, stdout: '{"corrupted":true}'}
    }})
  await fixture.corrupt({id: "backup-1"})
  assert.equal(calls.length, 2)
  assert.deepEqual(waits, [1000])
})

test("MP-03/MP-10: missing corruption pin has a bounded wait and never falls back", async () => {
  const fixture = createBrowserStateArchiveFixture({helper: "/owned/fixture", localDev: true,
    authorizationWaitMs: 0, runCommand: async () => ({code: 2, stdout: ""})})
  await assert.rejects(fixture.corrupt({}), /authorization/)
})
