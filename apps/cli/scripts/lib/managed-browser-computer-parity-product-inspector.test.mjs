import assert from "node:assert/strict"
import test from "node:test"
import { createManagedParityPhysicalLedger, createManagedParityInspectorFromAuthorities } from "./managed-browser-computer-parity-product-inspector.mjs"

function snapshot(overrides = {}) {
  return { schema: "chariox.managed_parity.host_observation.v1", bootId: "boot-1", engineId: "engine-1",
    mountNamespace: "mnt:[1]", hostRssBytes: 100,
    containers: [], volumes: [], processes: [], profiles: [], retainedProfiles: [], listeners: [], paths: [],
    hostContainerIds: [], hostVolumeNames: [], retainedContainers: [], retainedVolumes: [],
    retainedProcesses: [], retainedPaths: [],
    mounts: [{ path: "/tmp/run", device: "1", inode: "2", mountId: "3", parentMountId: "4",
      mountDevice: "8:1", mountPoint: "/", freeBytes: 1000 }], ...overrides }
}
const receipt = { sliceId: "slice-1", slice: { id: "slice-1", owner_kernel_id: "home", owner_machine_id: "home-machine" } }

test("physical ledger retains both immutable generations and checks them after logical deletion", async () => {
  const calls = []
  const observations = [snapshot(), snapshot({ containers: [{ id: "first" }] }),
    snapshot({ containers: [{ id: "second" }] }), snapshot({ retainedContainers: ["first"] })]
  const ledger = createManagedParityPhysicalLedger({ host: { paths: ["/tmp/run"] },
    async observe(input) { calls.push(structuredClone(input)); return observations.shift() } })
  await ledger.begin()
  await ledger.observeCreated(receipt)
  await ledger.observeCreated(receipt)
  const result = await ledger.inspect()
  assert.deepEqual(calls.at(-1).retained.containerIds, ["first", "second"])
  assert.equal(result.containers, 1)
})

test("physical ledger rejects preexisting ownership and never forgets a failed census", async () => {
  let index = 0
  const ledger = createManagedParityPhysicalLedger({ host: { paths: ["/tmp/run"] }, async observe() {
    return index++ === 0 ? snapshot({ hostContainerIds: ["existing"] })
      : index === 2 ? snapshot({ containers: [{ id: "existing" }] }) : snapshot()
  } })
  await ledger.begin()
  await assert.rejects(ledger.observeCreated(receipt), /preexisting container/)
  await ledger.beforeRetire({})
  await assert.rejects(ledger.inspect(), /preexisting container/)
})

test("physical ledger refuses a changed boot, mount or incomplete census", async () => {
  for (const changed of [snapshot({ bootId: "rebooted" }), snapshot({ mounts: [] }),
    snapshot({ retainedProcesses: undefined }), snapshot({ engineId: "wrong-engine" })]) {
    let first = true
    const ledger = createManagedParityPhysicalLedger({ host: { paths: ["/tmp/run"] }, async observe() {
      if (first) { first = false; return snapshot() }
      return changed
    } })
    await ledger.begin()
    await assert.rejects(ledger.inspect(), /identity changed|mount identity|retainedProcesses/)
  }
})

function authorities({ owned = false, incompleteCloud = false } = {}) {
  const events = []
  const config = { runId: "run-1", expected: { kernelId: "kernel-1", machineId: "machine-1", roomId: "room-1" },
    inspector: { identity: "fixture", sha256: "fixture", observations: { machineOwnership: {
      kind: owned ? "run_owned" : "preexisting", machineId: "machine-1",
      ...(owned ? { creationReceipt: { runId: "run-1", machineId: "machine-1", kernelId: "kernel-1",
        managedEnvironmentId: "managed-1", providerServerId: "123", createOperationId: "create-1" } } : {}),
    } } } }
  const ownershipReceipt = { runId: "run-1", machineId: "machine-1", ownedTargetIds: ["target-1"],
    ownedHeartbeatIds: ["heartbeat-1"], ownedKernelIds: ["child-1"] }
  let providerReads = 0
  const cloud = { authority: { kind: "read-only-cloud-inventory" },
    async begin() { return ownershipReceipt }, async observeCreated() { return ownershipReceipt }, async beforeRetire() { return ownershipReceipt },
    async inspect() { return { authority: "read-only-cloud-inventory", complete: true, runId: "run-1", machineId: "machine-1",
      queriedTargetIds: incompleteCloud ? [] : ["target-1"], queriedHeartbeatIds: ["heartbeat-1"], activeTargets: [],
      retainedTargetRows: [], retainedHeartbeatRows: [], retainedEvidenceLeakCount: 0 } },
    async inspectProviderMachine() { events.push("provider"); return { authority: "hetzner-api", providerServerId: "123",
      httpStatus: providerReads++ === 0 ? 200 : 404, observedAt: new Date().toISOString(),
      creationOperationId: "create-1", managedEnvironmentId: "managed-1" } },
  }
  const physical = { async begin() {}, async observeCreated() {}, async beforeRetire() {}, async inspect() {
    events.push("physical"); return { bootId: "boot-1", ownedSliceIds: ["slice-1"], volumes: 0, containers: 0,
      processes: 0, profiles: 0, listeners: 0, temporaryFiles: 0, resources: { rssDeltaBytes: 4, diskDeltaBytes: 8 } }
  } }
  const requests = {
    getManagedEnvironmentRequest: () => ({ GetManagedEnvironment: {} }),
    requestManagedEnvironmentLifecycleRequest: (input) => ({ RequestManagedEnvironmentLifecycle: input }),
    listSessionsRequest: () => ({ ListSessions: null }), listSlicesRequest: () => ({ ListSlices: null }),
  }
  const home = { async close() {}, async send(request) {
    const kind = Object.keys(request)[0]; events.push(kind)
    if (kind === "GetManagedEnvironment") return { ManagedEnvironment: { environment: {
      environmentId: "managed-1", runtimeMachineId: "machine-1", runtimeKernelId: "kernel-1" } } }
    if (kind === "RequestManagedEnvironmentLifecycle") return { ManagedEnvironmentLifecycleRequested: {
      result: { environment: { environmentId: "managed-1" }, operation: { kind: "delete" } } } }
    if (kind === "QueryFreshRemoteMachineKernels") return { FreshRemoteMachineKernelsObserved: {
      machine_ref: "machine-1", query_started_at_ms: 1, query_completed_at_ms: 2, kernels: [] } }
    if (kind === "ListSessions") return { SessionsListed: { sessions: [] } }
    if (kind === "ListSlices") return { SlicesListed: { slices: [] } }
    throw new Error("unexpected request")
  } }
  return { config, cloud, physical, home, requests, events,
    async inspectEvidence() { return { source: "physical-evidence-root", enumerationComplete: true, enumeratedFileCount: 1,
      files: [{ relativePath: "allowed.json", scan: { completed: true, forbiddenMatches: 0 } }], allowedEvidencePaths: ["allowed.json"] } } }
}

test("preexisting machine scope is explicit and never requests managed machine deletion", async () => {
  const fixture = authorities()
  const inspector = createManagedParityInspectorFromAuthorities(fixture)
  await inspector.begin({})
  const result = await inspector.run("cleanup.inspect")
  assert.equal(result.managedMachines, 0)
  assert.equal(result.machineOwnership, "preexisting")
  assert.equal(fixture.events.includes("RequestManagedEnvironmentLifecycle"), false)
  assert.equal(result.retirement, null)
})

test("run-owned deletion retains actual before-delete telemetry and separately proves provider absence", async () => {
  const fixture = authorities({ owned: true })
  const inspector = createManagedParityInspectorFromAuthorities(fixture)
  inspector.setBeforeMachineDelete(async () => { fixture.events.push("final-sample"); return { phase: "before-delete", capturedAt: "actual-time" } })
  await inspector.begin({})
  const result = await inspector.run("cleanup.inspect")
  assert.ok(fixture.events.indexOf("final-sample") < fixture.events.indexOf("RequestManagedEnvironmentLifecycle"))
  assert.equal(result.retirement.providerAbsence.httpStatus, 404)
  assert.equal(result.retirement.finalBeforeDeleteSample.phase, "before-delete")
  assert.deepEqual(result.retirement.afterDeletionTelemetry, { available: false, reason: "machine_deleted" })
  assert.deepEqual(result.resources, { rssDeltaBytes: 4, diskDeltaBytes: 8 })
})

test("missing retained Cloud identity coverage fails even with no current relay registration", async () => {
  const inspector = createManagedParityInspectorFromAuthorities(authorities({ incompleteCloud: true }))
  await inspector.begin({})
  await assert.rejects(inspector.run("cleanup.inspect"), /every retained identity/)
})

test("started browser needs physical profile coverage and retained inode is not inferred away", async () => {
  const missing = createManagedParityPhysicalLedger({ host: { paths: ["/tmp/run"] }, async observe() { return snapshot() } })
  await missing.begin()
  await assert.rejects(missing.observeCreated({ ...receipt, kind: "slice.start" }), /profile physical coverage/)
  await assert.rejects(missing.inspect(), /profile physical coverage/)
  const profile = { path: "/volume/profile", device: "7", inode: "9" }
  const observations = [snapshot(), snapshot({ profiles: [profile] }), snapshot({ retainedProfiles: [profile] })]
  const ledger = createManagedParityPhysicalLedger({ host: { paths: ["/tmp/run"] }, async observe() { return observations.shift() } })
  await ledger.begin()
  await ledger.observeCreated({ ...receipt, kind: "slice.start" })
  const result = await ledger.inspect()
  assert.equal(result.volumes, 0)
  assert.equal(result.profiles, 1)
})

test("evidence leak count comes from classified physical files and completed scans", async () => {
  const fixture = authorities()
  fixture.inspectEvidence = async () => ({ source: "physical-evidence-root", enumerationComplete: true,
    enumeratedFileCount: 2, allowedEvidencePaths: ["allowed.json"], files: [
      { relativePath: "allowed.json", scan: { completed: true, forbiddenMatches: 2 } },
      { relativePath: "unexpected.json", scan: { completed: true, forbiddenMatches: 0 } },
    ] })
  const inspector = createManagedParityInspectorFromAuthorities(fixture)
  await inspector.begin({})
  assert.equal((await inspector.run("cleanup.inspect")).retainedEvidenceLeakCount, 3)
})

test("interrupted kernel inspection waits for pending send and socket close settlement", async () => {
  const fixture = authorities()
  const aborter = new AbortController()
  let rejectSend, finishClose, enteredSend
  const entered = new Promise((resolve) => { enteredSend = resolve })
  fixture.home.send = () => { enteredSend(); return new Promise((_resolve, reject) => { rejectSend = reject }) }
  fixture.home.close = () => { rejectSend(new Error("request lifetime retired")); return new Promise((resolve) => { finishClose = resolve }) }
  const inspector = createManagedParityInspectorFromAuthorities(fixture)
  await inspector.begin({})
  let settled = false
  const result = inspector.run("cleanup.inspect", {}, { signal: aborter.signal }).finally(() => { settled = true })
  await entered
  aborter.abort()
  await new Promise((resolve) => setImmediate(resolve))
  assert.equal(settled, false)
  finishClose()
  await assert.rejects(result, /interrupted/)
})

test("lost deletion receipt never retries and reconciles exact provider absence without declaring success", async () => {
  const fixture = authorities({ owned: true })
  const send = fixture.home.send
  let deletes = 0
  fixture.home.send = async (request) => {
    if (request.RequestManagedEnvironmentLifecycle) { deletes++; throw new Error("receipt lost") }
    return send(request)
  }
  const inspector = createManagedParityInspectorFromAuthorities(fixture)
  inspector.setBeforeMachineDelete(async () => ({ phase: "before-delete" }))
  await inspector.begin({})
  await assert.rejects(inspector.run("cleanup.inspect"), /receipt lost/)
  assert.equal(deletes, 1)
  assert.equal(inspector.getMachineRetirementEvidence().providerAbsence.httpStatus, 404)
})
