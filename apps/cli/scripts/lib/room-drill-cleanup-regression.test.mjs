// MP-08/MP-10/MP-11: refused group signals must not abandon independent cleanup.
import assert from "node:assert/strict"
import { access, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { runInNewContext } from "node:vm"
import { createOwnedDrillProcessGroup } from "./drill-owned-process-group.mjs"
import { roomCleanupComplete } from "./room-provider-retention.mjs"

const source = await readFile(new URL("../live-room-environment-pointer-click-drill.mjs", import.meta.url), "utf8")
function section(start, end) {
  assert.ok(source.includes(start) && source.includes(end), "Room drill function boundaries changed")
  return source.slice(source.indexOf(start), source.indexOf(end, source.indexOf(start)))
}
// Execute the real cleanup and termination functions without launching the drill.
const cleanupSource = section("async function cleanup() {", "async function evidenceArtifacts() {")
  + section("async function terminateChild(child) {", "async function waitFor(operation,")

async function fixture(t, mode, { retain = false, originalFailure = null } = {}) {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-room-cleanup-test-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  const tempRoot = path.join(root, "runtime")
  const workspace = path.join(root, "workspace")
  const evidenceRoot = path.join(root, "evidence")
  await Promise.all([tempRoot, workspace, evidenceRoot].map(directory => mkdir(directory)))
  await writeFile(path.join(tempRoot, "marker"), "disposable runtime")
  await writeFile(path.join(workspace, "marker"), "disposable workspace")
  const calls = []
  const signals = []
  const rootRow = { pid: 21000, ppid: process.pid, pgid: 21000, started: "original-root" }
  const refused = { pid: rootRow.pid, exitCode: null, signalCode: null }
  let snapshots = 0
  refused.ownedProcessGroup = createOwnedDrillProcessGroup(refused, {
    snapshot: () => {
      snapshots++
      if (snapshots === 1) return [rootRow]
      if (mode === "signal snapshot failure" || (mode === "exists snapshot failure" && snapshots > 2)) {
        throw new Error("snapshot unavailable: SYNTHETIC_REDACTION_CANARY")
      }
      if (mode === "PID reuse") return [{ ...rootRow, started: "reused-root" }]
      if (mode === "unverified reparented descendant") {
        refused.exitCode = 0
        return [{ pid: 21001, ppid: 1, pgid: rootRow.pid, started: "new-descendant" }]
      }
      return [rootRow]
    },
    signal: (...args) => { signals.push(args); refused.signalCode = args[1] },
  })
  const context = {
    tempRootPromise: Promise.resolve(tempRoot), evidenceRoot, publicEvidenceRoot: path.join(root, "public-evidence"),
    retainedProviderRoot: retain ? tempRoot : null,
    localForwarding: null, localAutomation: null, remoteAutomation: null,
    client: null, observerClient: null, workerClient: null, requests: null,
    slice: null, sessionId: null, fixture: null,
    fixtureWorkspaceLease: { kind: "direct", workspace },
    children: [{ pid: 21010, exitCode: null, signalCode: null },
      { pid: 21011, exitCode: null, signalCode: null }, refused],
    failure: originalFailure, result: {}, resources: [], sensitiveValues: [],
    containerName: "synthetic-owned-container", homeVolume: "synthetic-owned-volume",
    relayPort: 21020, kernelPort: 21021, tuiOutput: { local: "", remote: "" },
    path, writeFile, rm, mkdir, access, setTimeout, clearTimeout,
    sleep: async () => {},
    signalOwnedDrillChild: (child, signal) => {
      calls.push(["child", child.pid, signal])
      child.signalCode = signal
    },
    docker: async args => { calls.push(["docker", ...args]) },
    runCommand: async (_command, args) => ({ code: retain && args[0] === "volume" ? 0 : 1 }),
    portIsAvailable: async () => true,
    assertNoPlaintextSecretInTree: async () => {},
    assertNoPlaintextSecretInDrillState: async () => {},
    removeRoomDirectDockerWorkspaceFixture: async lease => {
      calls.push(["workspace"])
      await rm(lease.workspace, { recursive: true, force: true })
    },
    closeFixtureServer: async () => {},
    captureRoomKernelDiagnostics: async () => ({ status: "unavailable" }),
    resourceSnapshot: async label => ({ label }), roomCleanupComplete,
    redactDrillSecrets: text => text.replaceAll("SYNTHETIC_REDACTION_CANARY", "[redacted]"),
  }
  await runInNewContext(`${cleanupSource}; cleanup()`, context)
  return { context, signals, calls, tempRoot, workspace, evidenceRoot, refused }
}

for (const mode of ["signal snapshot failure", "exists snapshot failure", "PID reuse", "unverified reparented descendant"]) {
  test(`MP-08/MP-10/MP-11 ${mode} preserves refusal and finishes other resource cleanup`, async t => {
    const f = await fixture(t, mode)
    // An exists failure happens after the initially verified TERM. Never escalate.
    assert.deepEqual(f.signals, mode === "exists snapshot failure" ? [[-21000, "SIGTERM"]] : [])
    assert.deepEqual(f.calls, [
      ["child", 21011, "SIGTERM"], ["child", 21010, "SIGTERM"],
      ["docker", "rm", "-f", "synthetic-owned-container"],
      ["docker", "volume", "rm", "-f", "synthetic-owned-volume"], ["workspace"],
    ])
    await assert.rejects(access(f.tempRoot), { code: "ENOENT" })
    await assert.rejects(access(f.workspace), { code: "ENOENT" })
    const receipt = JSON.parse(await readFile(path.join(f.evidenceRoot, "cleanup.json"), "utf8"))
    assert.equal(receipt.containerGone && receipt.volumeGone && receipt.tempRootRemoved && receipt.fixtureWorkspaceRemoved, true)
    assert.equal(receipt.childCleanupFailures.length, 1)
    assert.equal(receipt.childCleanupFailures[0].pid, f.refused.pid)
    assert.equal(receipt.childCleanupFailures[0].message, f.context.redactDrillSecrets(f.context.failure.message))
    assert.equal(JSON.stringify(receipt).includes("SYNTHETIC_REDACTION_CANARY"), false)
    assert.equal(roomCleanupComplete(receipt, false), false)
    await assert.rejects(access(path.join(f.evidenceRoot, "result.json")), { code: "ENOENT" })
    assert.match(await readFile(path.join(f.evidenceRoot, "failure.txt"), "utf8"), /snapshot unavailable|reused|foreign/)
  })
}

test("MP-08/MP-10/MP-11 cleanup refusal retains protected provider state and the original failure", async t => {
  const originalFailure = new Error("original drill failure")
  const f = await fixture(t, "signal snapshot failure", { retain: true, originalFailure })
  assert.equal(f.context.failure, originalFailure)
  assert.deepEqual(f.signals, [])
  assert.equal(f.calls.filter(call => call[0] === "child").length, 2)
  assert.equal(f.calls.some(call => call[0] === "docker" && call[1] === "rm"), true)
  assert.equal(f.calls.some(call => call[0] === "docker" && call[1] === "volume"), false)
  await access(f.tempRoot)
  await assert.rejects(access(f.workspace), { code: "ENOENT" })
  const receipt = JSON.parse(await readFile(path.join(f.evidenceRoot, "cleanup.json"), "utf8"))
  assert.equal(receipt.providerStateRetained && receipt.homeVolumeRetained, true)
  assert.equal(receipt.childCleanupFailures.length, 1)
  assert.match(receipt.childCleanupFailures[0].message, /snapshot unavailable/)
  assert.equal(roomCleanupComplete(receipt, true), false)
  assert.match(await readFile(path.join(f.evidenceRoot, "failure.txt"), "utf8"), /original drill failure/)
})
