import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import test from "node:test"
import { access, copyFile, mkdir, mkdtemp, realpath, rm, symlink } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { pathToFileURL } from "node:url"
import { capturePath1CloudReimage, resolveCaptureOutput } from "./path1-cloud-reimage-capture.mjs"

function fixture() {
  const binding = {
    environmentId: "environment-1", operationId: "operation-1", generation: 2,
    releaseDigest: `sha256:${"a".repeat(64)}`, sourceCommit: "b".repeat(40), sourceTree: "c".repeat(40),
  }
  const release = { runtimeReleaseDigest: binding.releaseDigest, runtimeSourceCommit: binding.sourceCommit, runtimeSourceTree: binding.sourceTree }
  const receipt = {
    ...binding, previousGeneration: 1, status: "fresh_equivalent", freshEquivalent: true,
    receiptId: "receipt-1", receiptDigest: `sha256:${"d".repeat(64)}`,
    runtimeReleaseDigest: binding.releaseDigest, sourceEvidence: release,
    runtimeEvidence: {
      oldKernelIdentityBaseline: { source: "cloud_retained_old_kernel_identity", environmentId: "environment-1", generation: 1,
        machineId: "old-MachineId", kernelId: "old-KernelId", linuxBootId: "11111111-1111-1111-1111-111111111111", osMachineId: "1".repeat(32),
        runtimeReleaseDigest: `sha256:${"e".repeat(64)}`, runtimeSourceCommit: "f".repeat(40), runtimeSourceTree: "0".repeat(40),
        observedAt: "2026-09-26T04:59:00.000Z" },
      freshnessEvidence: { ...release, linuxBootId: "22222222-2222-2222-2222-222222222222", osMachineId: "2".repeat(32) },
    },
    providerServerId: "1234", providerImageId: "5678", resourceObservation: { rebuildActionId: "88" },
    cleanupState: { oldGenerationRetired: true, newGenerationEnrolled: true },
    residueChecks: Object.fromEntries(["oldServicesAbsent", "oldProcessesAbsent", "oldStateAbsent",
      "cloudOldMachineRevoked", "cloudOldCredentialsRevoked", "cloudOldTargetsRevoked",
      "cloudOldHeartbeatsAbsent", "cloudOldRelayRealmDisabled", "cloudOldGenerationRetired"].map((key) => [key, true])),
    requestedAt: "2026-09-26T05:00:00.000Z", completedAt: "2026-09-26T05:01:00.000Z",
  }
  for (const kind of ["MachineId", "KernelId", "RelayRealmId", "RelayTargetId", "BootstrapGrantId"]) {
    receipt[`old${kind}`] = `old-${kind}`
    receipt[`new${kind}`] = `new-${kind}`
  }
  const requests = []
  const requestContract = {
    getManagedEnvironmentReimageReceiptRequest(environmentId) {
      return { GetManagedEnvironmentReimageReceipt: { environmentId } }
    },
    minimumProtocolVersion: 345,
  }
  return { binding, receipt, requests, now: () => new Date("2026-09-26T05:02:00.000Z"),
    requestContract,
    client: { async send(request) { requests.push(request); return { ManagedEnvironmentReimageReceipt: { receipt } } } } }
}

test("captures the exact finalized Cloud operation through one read-only kernel request", async () => {
  const input = fixture()
  input.receipt.failureMessage = "DO-NOT-RETAIN-SECRET"
  input.receipt.runtimeEvidence.extraProviderOutput = "DO-NOT-RETAIN-SECRET"
  const capture = await capturePath1CloudReimage(input)
  assert.deepEqual(input.requests, [{ GetManagedEnvironmentReimageReceipt: { environmentId: "environment-1" } }])
  assert.equal(capture.minimumProtocolVersion, 345)
  assert.equal(capture.rebuildActionId, "88")
  assert.equal(capture.release.sourceCommit, input.binding.sourceCommit)
  assert.deepEqual(capture.before.release, {
    digest: input.receipt.runtimeEvidence.oldKernelIdentityBaseline.runtimeReleaseDigest,
    sourceCommit: "f".repeat(40), sourceTree: "0".repeat(40),
  })
  assert.equal(capture.before.observedAt, "2026-09-26T04:59:00.000Z")
  assert.ok(!JSON.stringify(capture).includes("DO-NOT-RETAIN-SECRET"))
  assert.equal(capture.status, undefined, "a capture is not a full acceptance verdict")
})

function sharedControllerFixture() {
  const input = fixture()
  input.binding.sharedControllerTargetId = "controller-target"
  input.receipt.oldRelayRealmId = "shared-realm"
  input.receipt.newRelayRealmId = "shared-realm"
  input.receipt.residueChecks.cloudOldRelayRealmDisabled = false
  const source = {
    sourceTargetId: "controller-target", relayRealmId: "shared-realm",
    machineId: "controller-machine", kernelId: "controller-kernel", keyThumbprint: "9".repeat(64),
  }
  const environment = {
    environmentId: input.binding.environmentId, accountId: "account-1",
    runtimeGeneration: input.binding.generation, runtimeReleaseDigest: input.binding.releaseDigest,
    runtimeMachineId: input.receipt.newMachineId, runtimeKernelId: input.receipt.newKernelId,
    runtimeRelayRealmId: "shared-realm", contextPlan: { source },
  }
  const status = { configured: true, connected: true, machine_id: source.machineId, daemon_id: source.kernelId }
  const profile = { account_id: "account-1", realm_id: "shared-realm", machine_id: source.machineId,
    machine_credential: "NEVER-RETAIN-CREDENTIAL", cloud_session_token: "NEVER-RETAIN-TOKEN" }
  Object.assign(input.requestContract, {
    getManagedEnvironmentRequest: (environmentId) => ({ GetManagedEnvironment: { environmentId } }),
    relayStatusRequest: () => ({ RelayStatus: null }),
    cloudRelayStatusRequest: () => ({ CloudRelayStatus: null }),
  })
  input.client.send = async (request) => {
    input.requests.push(request)
    if ("GetManagedEnvironment" in request) return { ManagedEnvironment: { environment } }
    if ("RelayStatus" in request) return { RelayStatus: { status } }
    if ("CloudRelayStatus" in request) return { CloudRelayStatus: { profile } }
    return { ManagedEnvironmentReimageReceipt: { receipt: input.receipt } }
  }
  return { ...input, environment, source, status, profile }
}

test("captures a retained shared controller realm only with explicit current product bindings", async () => {
  const input = sharedControllerFixture()
  const capture = await capturePath1CloudReimage(input)
  assert.deepEqual(capture.retainedController, {
    sourceTargetId: "controller-target", machineId: "controller-machine", kernelId: "controller-kernel",
    relayRealmId: "shared-realm", accountId: "account-1",
  })
  assert.equal(capture.residueChecks.cloudOldRelayRealmDisabled, false)
  assert.equal(capture.residueChecks.cloudOldGenerationRetired, true)
  assert.equal(input.requests.length, 5)
  assert.ok(!JSON.stringify(capture).includes("NEVER-RETAIN"))
})

for (const [name, mutate] of [
  ["implicit shared realm", (f) => { delete f.binding.sharedControllerTargetId }],
  ["unselected controller target", (f) => { f.source.sourceTargetId = "other-target" }],
  ["foreign account", (f) => { f.profile.account_id = "other-account" }],
  ["foreign realm", (f) => { f.profile.realm_id = "other-realm" }],
  ["disconnected controller", (f) => { f.status.connected = false }],
  ["different local kernel", (f) => { f.status.daemon_id = "different-kernel" }],
  ["retired controller identity", (f) => { f.source.machineId = f.receipt.oldMachineId }],
  ["replacement as controller", (f) => { f.source.machineId = f.receipt.newMachineId }],
  ["stale environment generation", (f) => { f.environment.runtimeGeneration++ }],
  ["stale environment release", (f) => { f.environment.runtimeReleaseDigest = `sha256:${"f".repeat(64)}` }],
  ["unretired old credentials", (f) => { f.receipt.residueChecks.cloudOldCredentialsRevoked = false }],
  ["unretired old targets", (f) => { f.receipt.residueChecks.cloudOldTargetsRevoked = false }],
  ["unretired old generation", (f) => { f.receipt.residueChecks.cloudOldGenerationRetired = false }],
  ["missing disabled-realm assertion", (f) => { delete f.receipt.residueChecks.cloudOldRelayRealmDisabled }],
]) {
  test(`rejects shared-controller capture with ${name}`, async () => {
    const input = sharedControllerFixture()
    mutate(input)
    await assert.rejects(capturePath1CloudReimage(input))
  })
}

test("rejects controller identity changes between the bracketing reads", async () => {
  const input = sharedControllerFixture()
  const send = input.client.send
  let statusReads = 0
  input.client.send = async (request) => {
    const response = await send(request)
    if ("RelayStatus" in request && ++statusReads === 2) {
      return { RelayStatus: { status: { ...input.status, daemon_id: "replacement-connection" } } }
    }
    return response
  }
  await assert.rejects(capturePath1CloudReimage(input), /connected local kernel/)
  assert.equal(statusReads, 2)
})

test("bounds all shared-controller reads by the capture deadline", async () => {
  const input = sharedControllerFixture()
  const send = input.client.send
  input.client.send = (request) => "CloudRelayStatus" in request ? new Promise(() => {}) : send(request)
  await assert.rejects(capturePath1CloudReimage({ ...input, timeoutMs: 20 }), /timed out/)
  assert.equal(input.requests.filter((request) => "RelayStatus" in request).length, 1)
})

for (const [name, mutate, message] of [
  ["wrong operation", (r) => { r.operationId = "operation-other" }, /selected reimage/],
  ["wrong environment", (r) => { r.environmentId = "environment-other" }, /selected reimage/],
  ["wrong generation", (r) => { r.generation = 3 }, /selected reimage/],
  ["pending receipt", (r) => { r.status = "pending" }, /not finalized/],
  ["unbound release", (r) => { r.sourceEvidence.runtimeSourceCommit = "e".repeat(40) }, /reviewed release/],
  ["unchanged boot", (r) => { r.runtimeEvidence.freshnessEvidence.linuxBootId = r.runtimeEvidence.oldKernelIdentityBaseline.linuxBootId }, /rotated host/],
  ["wrong retained environment", (r) => { r.runtimeEvidence.oldKernelIdentityBaseline.environmentId = "another-environment" }, /rotated host/],
  ["wrong retained generation", (r) => { r.runtimeEvidence.oldKernelIdentityBaseline.generation = 2 }, /rotated host/],
  ["wrong retained machine", (r) => { r.runtimeEvidence.oldKernelIdentityBaseline.machineId = "another-machine" }, /rotated host/],
  ["missing retained release", (r) => { delete r.runtimeEvidence.oldKernelIdentityBaseline.runtimeReleaseDigest }, /retained release/],
  ["malformed retained source", (r) => { r.runtimeEvidence.oldKernelIdentityBaseline.runtimeSourceCommit = "unreviewed" }, /retained release/],
  ["missing retained observation time", (r) => { delete r.runtimeEvidence.oldKernelIdentityBaseline.observedAt }, /baseline time/],
  ["post-rebuild baseline", (r) => { r.runtimeEvidence.oldKernelIdentityBaseline.observedAt = r.completedAt }, /baseline time/],
  ["noncanonical retained time", (r) => { r.runtimeEvidence.oldKernelIdentityBaseline.observedAt = "2026-09-26 04:59" }, /baseline time/],
  ["unchanged control identity", (r) => { r.newMachineId = r.oldMachineId }, /rotated control/],
  ["missing rebuild", (r) => { delete r.resourceObservation.rebuildActionId }, /provider rebuild/],
  ["incomplete retirement", (r) => { r.residueChecks.cloudOldHeartbeatsAbsent = false }, /retirement/],
  ["nonshared realm not retired", (r) => { r.residueChecks.cloudOldRelayRealmDisabled = false }, /realm retirement/],
  ["future completion", (r) => { r.completedAt = "2026-09-27T05:00:00.000Z" }, /completion times/],
]) {
  test(`rejects ${name}`, async () => {
    const input = fixture()
    mutate(input.receipt)
    await assert.rejects(capturePath1CloudReimage(input), message)
  })
}

test("invalid binding never sends a kernel request", async () => {
  const input = fixture()
  delete input.binding.environmentId
  await assert.rejects(capturePath1CloudReimage(input), /invalid reviewed/)
  assert.deepEqual(input.requests, [])
})

test("a hung read has a bounded timeout", async () => {
  const input = fixture()
  input.client.send = () => new Promise(() => {})
  await assert.rejects(capturePath1CloudReimage({ ...input, timeoutMs: 20 }), /timed out/)
})

test("injected capture imports without generated client dist and CLI reports the build prerequisite", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "path1-capture-source-"))
  const evidence = await mkdtemp(join(tmpdir(), "path1-capture-evidence-"))
  t.after(async () => {
    await rm(root, { recursive: true, force: true })
    await rm(evidence, { recursive: true, force: true })
  })
  const copiedScript = join(root, "apps/cli/scripts/path1-cloud-reimage-capture.mjs")
  await mkdir(join(root, "apps/cli/scripts"), { recursive: true })
  await copyFile(new URL("./path1-cloud-reimage-capture.mjs", import.meta.url), copiedScript)
  const script = await realpath(copiedScript)
  await assert.rejects(access(join(root, "packages/kernel-client/dist/ipc.js")), { code: "ENOENT" })
  await assert.rejects(access(join(root, "packages/kernel-client/dist/ipc-managed-environment-requests.js")), { code: "ENOENT" })
  const isolated = await import(`${pathToFileURL(script).href}?no-generated-dist=${Date.now()}`)
  const input = fixture()
  const capture = await isolated.capturePath1CloudReimage(input)
  assert.equal(capture.minimumProtocolVersion, 345)
  assert.deepEqual(input.requests, [{ GetManagedEnvironmentReimageReceipt: { environmentId: "environment-1" } }])

  const output = join(evidence, "capture.json")
  const result = spawnSync(process.execPath, [script,
    "--kernel", "ws://127.0.0.1:1", "--environment", input.binding.environmentId,
    "--operation", input.binding.operationId, "--generation", String(input.binding.generation),
    "--release", input.binding.releaseDigest, "--commit", input.binding.sourceCommit,
    "--tree", input.binding.sourceTree, "--output", output,
  ], { encoding: "utf8", timeout: 10_000 })
  assert.equal(result.status, 1)
  assert.equal(result.stdout, "")
  assert.equal(result.stderr,
    "Path-1 Cloud capture requires built kernel-client artifacts; run pnpm --workspace-root run build:kernel-client.\n")
  await assert.rejects(access(output), { code: "ENOENT" })
})

test("output resolution rejects symlinked repository parents before any capture", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-cloud-capture-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  const repo = join(root, "repo")
  const evidence = join(root, "evidence")
  await mkdir(repo)
  await mkdir(evidence)
  await symlink(repo, join(evidence, "repository-alias"))
  await assert.rejects(resolveCaptureOutput(join(repo, "receipt.json"), repo), /outside the repository/)
  await assert.rejects(resolveCaptureOutput(join(evidence, "repository-alias", "receipt.json"), repo), /outside the repository/)
  await assert.rejects(resolveCaptureOutput("receipt.json", repo), /absolute external/)
  assert.equal(await resolveCaptureOutput(join(evidence, "receipt.json"), repo),
    join(await realpath(evidence), "receipt.json"))
})
