import assert from "node:assert/strict"
import test from "node:test"
import { createHash } from "node:crypto"
import { mkdtemp, readFile, realpath, rm, stat, symlink, writeFile } from "node:fs/promises"
import { execFile as execFileCallback } from "node:child_process"
import { promisify } from "node:util"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { correlatePath1HostCloud, readCorrelationCapture } from "./path1-host-cloud-correlation.mjs"
import { offlineCorrelationScript } from "./path1-offline-correlation-test-fixture.mjs"

function fixture() {
  const release = digit => ({ digest: `sha256:${digit.repeat(64)}`, sourceCommit: digit.repeat(40), sourceTree: digit.repeat(40) })
  const old = { bootId: "11111111-1111-1111-1111-111111111111", machineId: "1".repeat(32), release: release("a") }
  const fresh = { bootId: "22222222-2222-2222-2222-222222222222", machineId: "2".repeat(32), release: release("b") }
  const host = (identity, digit, unit, capturedAt) => ({
    schema: "chariox.path1-host-identity-capture/v1", capturedAt,
    identity: { ...identity,
      serviceInstances: [{ unit, invocationId: digit.repeat(32) }],
      processes: [`${identity.bootId}:321:98765`],
      stateInstances: [ { path: "/var/lib/chariox", instanceId: "stat:2049:101" },
        { path: "/home/chariox", instanceId: "stat:2049:102" } ],
    },
    observations: { ignored: "DO-NOT-RETAIN" },
  })
  return {
    before: host(old, "a", "chariox-managed-bootstrap.service", "2026-09-26T04:59:00.000Z"),
    after: host(fresh, "b", "chariox-path1-managed-bootstrap.service", "2026-09-26T05:02:00.000Z"),
    cloud: { schema: "chariox.path1-cloud-reimage-capture/v1", environmentId: "env-1", operationId: "op-1",
      receiptId: "receipt-1", generation: 2, previousGeneration: 1, providerServerId: "1234", rebuildActionId: "88",
      before: { ...old, observedAt: "2026-09-26T04:58:00.000Z" }, after: fresh, release: fresh.release,
      requestedAt: "2026-09-26T05:00:00.000Z", completedAt: "2026-09-26T05:01:00.000Z",
      capturedAt: "2026-09-26T05:02:00.000Z" },
    now: () => new Date("2026-09-26T05:03:00.000Z"),
  }
}

test("correlates host identities with retained Cloud bindings without an acceptance verdict", () => {
  const input = fixture()
  const result = correlatePath1HostCloud(input)
  assert.equal(result.after.release.digest, input.cloud.release.digest)
  assert.equal(result.before.release.sourceCommit, input.cloud.before.release.sourceCommit)
  assert.equal(result.operationId, "op-1")
  assert.equal(result.status, undefined)
  assert.ok(!JSON.stringify(result).includes("DO-NOT-RETAIN"))
  // Rebuilt filesystems may reuse device/inode numbers. Correlation must not
  // confuse those numbers with standalone proof of residue-free state.
  assert.deepEqual(result.before.stateInstances, result.after.stateInstances)
})

for (const [name, mutate, error] of [
  ["wrong old host", input => { input.before.identity.bootId = input.after.identity.bootId }, /does not match Cloud/],
  ["wrong new machine", input => { input.after.identity.machineId = "3".repeat(32) }, /does not match Cloud/],
  ["wrong old release", input => { input.before.identity.release = { ...input.before.identity.release, sourceTree: "c".repeat(40) } }, /release/],
  ["wrong new release", input => { input.after.identity.release = { ...input.after.identity.release, digest: `sha256:${"c".repeat(64)}` } }, /release/],
  ["late before capture", input => { input.before.capturedAt = input.after.capturedAt }, /bracket/],
  ["stale after capture", input => { input.after.capturedAt = input.before.capturedAt }, /bracket/],
  ["future Cloud capture", input => { input.cloud.capturedAt = "2026-09-27T00:00:00.000Z" }, /bracket/],
  ["shared-host service", input => { input.after.identity.serviceInstances[0].unit = "chariox-managed-bootstrap.service" }, /topology/],
  ["reused invocation", input => { input.after.identity.serviceInstances[0].invocationId = "a".repeat(32) }, /old service/],
  ["duplicate invocation", input => { input.after.identity.serviceInstances.push({ unit: "chariox-rootless-docker.service", invocationId: "b".repeat(32) }) }, /ambiguous/],
  ["wrong state root", input => { input.after.identity.stateInstances[0].path = "/tmp" }, /state-root/],
  ["wrong process boot", input => { input.after.identity.processes = [...input.before.identity.processes] }, /bound to this boot/],
  ["unknown capture schema", input => { input.after.schema = "synthetic" }, /unsupported host/],
  ["unknown service", input => { input.before.identity.serviceInstances[0].unit = "arbitrary.service" }, /ambiguous/],
  ["missing Cloud operation", input => { delete input.cloud.operationId }, /operation binding/],
  ["wrong Cloud generation", input => { input.cloud.previousGeneration = 3 }, /operation binding/],
]) {
  test(`rejects ${name}`, () => {
    const input = fixture()
    mutate(input)
    assert.throws(() => correlatePath1HostCloud(input), error)
  })
}

test("capture files bind exact retained bytes and reject symlink and oversized inputs", async t => {
  const root = await mkdtemp(join(tmpdir(), "chariox-path1-correlation-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  const file = join(root, "capture.json")
  const bytes = Buffer.from(`${JSON.stringify(fixture().before)}\n`)
  await writeFile(file, bytes)
  const result = await readCorrelationCapture(file)
  assert.equal(result.capture.identity.bootId, fixture().before.identity.bootId)
  assert.equal(result.evidence.sha256, `sha256:${createHash("sha256").update(bytes).digest("hex")}`)
  await symlink(file, join(root, "alias.json"))
  await assert.rejects(readCorrelationCapture(join(root, "alias.json")), /bounded regular/)
  await writeFile(file, Buffer.alloc(1024 * 1024 + 1))
  await assert.rejects(readCorrelationCapture(file), /bounded regular/)
})

test("CLI writes a private external correlation and never overwrites evidence", async t => {
  const root = await realpath(await mkdtemp(join(tmpdir(), "chariox-path1-correlation-cli-")))
  t.after(() => rm(root, { recursive: true, force: true }))
  const input = fixture()
  const instant = Date.now()
  const at = offset => new Date(instant + offset).toISOString()
  input.cloud.before.observedAt = at(-4000)
  input.before.capturedAt = at(-3000)
  input.cloud.requestedAt = at(-2000)
  input.cloud.completedAt = at(-1000)
  input.cloud.capturedAt = at(-500)
  input.after.capturedAt = at(-500)
  const args = []
  for (const kind of ["before", "after", "cloud"]) {
    const file = join(root, `${kind}.json`)
    await writeFile(file, `${JSON.stringify(input[kind])}\n`)
    args.push(`--${kind}`, file)
  }
  const output = join(root, "correlation.json")
  args.push("--output", output)
  const execFile = promisify(execFileCallback)
  const script = await offlineCorrelationScript(root, "path1-host-cloud-correlation.mjs")
  const result = await execFile(process.execPath, [script, ...args], { timeout: 5000 })
  assert.match(result.stdout, /Full MP-10 evidence remains required/)
  const bytes = await readFile(output)
  const correlation = JSON.parse(bytes)
  assert.equal(correlation.status, undefined)
  assert.equal(Object.keys(correlation.evidence).length, 3)
  assert.equal((await stat(output)).mode & 0o777, 0o600)
  await assert.rejects(execFile(process.execPath, [script, ...args], { timeout: 5000 }), error => {
    assert.match(error.stderr, /no acceptance verdict/)
    return error.code === 1
  })
  assert.deepEqual(await readFile(output), bytes)
})
