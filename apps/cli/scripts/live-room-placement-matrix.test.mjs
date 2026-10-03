import assert from "node:assert/strict"
import test from "node:test"

import {
  ROOM_PLACEMENT_ROWS,
  evaluateRoomPlacementRow,
  probeForeignRoomDenial,
  validateRoomPlacementMatrixConfig,
} from "./live-room-placement-matrix.mjs"

const homeKernel = { url: "/tmp/home.sock", kernelId: "home-kernel", machineId: "home-machine" }

test("matrix config requires the six exact placement roles without substitution", () => {
  const config = validConfig()
  assert.equal(validateRoomPlacementMatrixConfig(config), config)
  assert.equal(config.rows.length, 6)

  const sameWorkerSubstitution = structuredClone(config)
  sameWorkerSubstitution.rows[2].agentPlacement = { kind: "home_kernel" }
  assertRejects(() => validateRoomPlacementMatrixConfig(sameWorkerSubstitution), /remote-worker agent/)

  const wrongEnvironmentWorker = structuredClone(config)
  wrongEnvironmentWorker.rows[4].agentWorkerKernelId = "other-worker"
  assertRejects(() => validateRoomPlacementMatrixConfig(wrongEnvironmentWorker), /agent kernel identity must equal the Environment worker/)

  const missingRow = structuredClone(config)
  missingRow.rows.pop()
  assertRejects(() => validateRoomPlacementMatrixConfig(missingRow), /each of the six Room placement rows/)

  const remoteHome = structuredClone(config)
  remoteHome.homeKernel.url = "wss://kernel.example.invalid"
  assertRejects(() => validateRoomPlacementMatrixConfig(remoteHome), /local Unix socket or loopback kernel/)

  const rowSixHomeKernel = structuredClone(config)
  rowSixHomeKernel.rows[5].agentPlacement = {
    kind: "kernel_ref",
    kernelRef: rowSixHomeKernel.homeKernel.kernelId,
  }
  rowSixHomeKernel.rows[5].agentWorkerKernelId = rowSixHomeKernel.homeKernel.kernelId
  rowSixHomeKernel.rows[5].agentWorkerMachineId = rowSixHomeKernel.homeKernel.machineId
  assertRejects(() => validateRoomPlacementMatrixConfig(rowSixHomeKernel), /must select a non-home remote worker/)
})

test("local-only config requires the selected row and a distinct foreign Room", () => {
  const config = validConfig()
  config.selectionMode = "local_only"
  config.foreignRoomId = "foreign-room-probe"
  config.rows = [config.rows[1]]
  assert.equal(validateRoomPlacementMatrixConfig(config), config)

  const evidence = validEvidence(config.rows[0])
  evidence.selectionMode = "local_only"
  evidence.foreignRoomId = config.foreignRoomId
  evidence.foreignRoomDenial = {
    requestedRoomId: config.foreignRoomId,
    attachmentRoomId: config.rows[0].roomId,
    denialCode: "attachment_not_in_session",
  }
  const proof = evaluateRoomPlacementRow({ row: config.rows[0], homeKernel, evidence })
  assert.equal(proof.ok, true)
  assert.equal(proof.foreignRoomDenial, true)
  assert.equal(Object.hasOwn(proof.unexecutedGates, "foreignRoomDenial"), false)
  for (const gate of ["takeover", "ordering", "reconnect", "forgedLeaseDenial",
    "directCrossSliceFileProcess", "cloudWebViewRendering"]) {
    assert.match(proof.unexecutedGates[gate], /^unexecuted:/)
  }

  evidence.foreignRoomDenial.denialCode = "room_has_no_environment"
  const wrongDenial = evaluateRoomPlacementRow({ row: config.rows[0], homeKernel, evidence })
  assert.equal(wrongDenial.ok, false)
  assert.ok(wrongDenial.violations.includes("foreign_room_denial_not_proven"))

  const missingForeignRoom = structuredClone(config)
  delete missingForeignRoom.foreignRoomId
  assertRejects(() => validateRoomPlacementMatrixConfig(missingForeignRoom), /foreignRoomId/)
  const substituted = structuredClone(config)
  substituted.rows = [validConfig().rows[0]]
  assertRejects(() => validateRoomPlacementMatrixConfig(substituted), /local-only placement row/)
})

test("foreign-Room display probe accepts only an attachment-scope denial", async () => {
  const row = validConfig().rows[1]
  const requests = []
  const requestApi = {
    getSliceDisplayEndpointRequest(sliceRef, scope) {
      return { sliceRef, scope }
    },
  }
  const args = {
    requestApi, row, foreignRoomId: "foreign-room-probe",
    attachmentId: "attached-to-placement-room", viewerPublicKey: "viewer-key",
  }
  const accepted = await probeForeignRoomDenial({
    ...args,
    client: {
      async send(request) {
        requests.push(request)
        throw Object.assign(new Error("denied"), { code: "attachment_not_in_session" })
      },
    },
  })
  assert.deepEqual(requests, [{ sliceRef: row.environmentSliceRef, scope: {
    sessionId: "foreign-room-probe",
    attachmentId: "attached-to-placement-room",
    viewerPublicKey: "viewer-key",
  } }])
  assert.deepEqual(accepted, {
    requestedRoomId: "foreign-room-probe",
    attachmentRoomId: row.roomId,
    denialCode: "attachment_not_in_session",
  })
  await assert.rejects(probeForeignRoomDenial({
    ...args,
    client: { async send() { throw Object.assign(new Error("missing"), { code: "room_has_no_environment" }) } },
  }), /reason other than attachment scope/)
  await assert.rejects(probeForeignRoomDenial({
    ...args,
    client: { async send() { return { endpoint: "wrongly allowed" } } },
  }), /was not denied/)
})

test("all six placement proofs validate while acceptance-only gates remain explicitly unexecuted", () => {
  const config = validConfig()
  validateRoomPlacementMatrixConfig(config)
  for (const row of config.rows) {
    const result = evaluateRoomPlacementRow({ row, homeKernel, evidence: validEvidence(row) })
    assert.deepEqual(result.violations, [], row.id)
    assert.equal(result.ok, true, row.id)
    assert.equal(result.sameRoomEnvironment, true, row.id)
    assert.equal(result.sameAgent, true, row.id)
    assert.equal(result.sameStableTab, true, row.id)
    assert.equal(result.fullAcceptance, "not_proven", row.id)
    for (const gate of ["takeover", "ordering", "reconnect", "foreignRoomDenial", "forgedLeaseDenial"]) {
      assert.match(result.unexecutedGates[gate], /^unexecuted:/, `${row.id}:${gate}`)
    }
  }
})

test("remote-agent row fails closed if silently substituted with the home worker", () => {
  const row = validConfig().rows[2]
  const evidence = validEvidence(row)
  evidence.agent.workerKernelId = homeKernel.kernelId
  evidence.agent.workerMachineId = homeKernel.machineId
  evidence.agent.remoteExecution = null

  const result = evaluateRoomPlacementRow({ row, homeKernel, evidence })
  assert.equal(result.ok, false)
  assert.ok(result.violations.includes("agent_worker_mismatch"))
  assert.ok(result.violations.includes("remote_agent_lease_binding_mismatch"))
})

test("Browser and Computer actions must have the same attributed Room agent", () => {
  const row = validConfig().rows[4]
  const evidence = validEvidence(row)
  evidence.computerAction.actorId = "agent:other-agent"
  evidence.actionHistory[1].actor_id = "agent:other-agent"
  evidence.environment.actions[1].actor_id = "agent:other-agent"

  const result = evaluateRoomPlacementRow({ row, homeKernel, evidence })
  assert.equal(result.ok, false)
  assert.ok(result.violations.includes("browser_computer_actor_mismatch"))
  assert.equal(result.sameAgent, false)

  const foreignActor = validEvidence(row)
  foreignActor.environment.actors = []
  const missingActor = evaluateRoomPlacementRow({ row, homeKernel, evidence: foreignActor })
  assert.equal(missingActor.ok, false)
  assert.ok(missingActor.violations.includes("room_agent_actor_missing"))
})

test("public history and Web View must retain one Environment and stable Tab", () => {
  const row = validConfig().rows[3]
  const changedEnvironment = validEvidence(row)
  changedEnvironment.webView.environmentId = "other-environment"
  let result = evaluateRoomPlacementRow({ row, homeKernel, evidence: changedEnvironment })
  assert.equal(result.ok, false)
  assert.ok(result.violations.includes("web_view_room_environment_tab_mismatch"))

  const changedTab = validEvidence(row)
  changedTab.browserAction.targetTabId = "other-tab"
  changedTab.actionHistory[0].targets = [{ kind: "browser_tab", id: "other-tab" }]
  result = evaluateRoomPlacementRow({ row, homeKernel, evidence: changedTab })
  assert.equal(result.ok, false)
  assert.ok(result.violations.includes("browser_action_tab_mismatch"))

  const changedHistory = validEvidence(row)
  changedHistory.actionHistory[0].targets = [{ kind: "browser_tab", id: "other-tab" }]
  result = evaluateRoomPlacementRow({ row, homeKernel, evidence: changedHistory })
  assert.equal(result.ok, false)
  assert.ok(result.violations.includes("public_action_history_mismatch"))
})

test("action history, Room state ledger, and Web View visibility are all required", () => {
  const row = validConfig().rows[0]
  const evidence = validEvidence(row)
  evidence.actionHistory.pop()
  evidence.webView.videoStarted = false

  const result = evaluateRoomPlacementRow({ row, homeKernel, evidence })
  assert.equal(result.ok, false)
  assert.ok(result.violations.includes("public_action_history_mismatch"))
  assert.ok(result.violations.includes("public_web_view_not_visible"))
})

test("one Environment runtime generation must span both actions and the Web View", () => {
  const row = validConfig().rows[1]
  const evidence = validEvidence(row)
  evidence.computerAction.runtimeGeneration = 2
  evidence.actionHistory[1].runtime_generation = 2
  evidence.environment.actions[1].runtime_generation = 2

  const result = evaluateRoomPlacementRow({ row, homeKernel, evidence })
  assert.equal(result.ok, false)
  assert.ok(result.violations.includes("provider_action_environment_generation_mismatch"))
})

function validConfig() {
  return {
    schema: "chariox.room_placement_matrix.config.v1",
    homeKernel,
    provider: { provider: "opencode", model: "configured-model", accountProfile: "default", effort: "low" },
    rows: ROOM_PLACEMENT_ROWS.map((spec, index) => {
      const roomId = `room-${index + 1}`
      const environmentSliceRef = `environment-slice-${index + 1}`
      const environmentRemote = spec.environment === "remote"
      const environmentWorkerKernelId = environmentRemote ? `environment-worker-${index + 1}` : `home-slice-worker-${index + 1}`
      const environmentWorkerMachineId = environmentRemote ? `environment-machine-${index + 1}` : homeKernel.machineId
      let agentPlacement
      let agentWorkerKernelId
      let agentWorkerMachineId
      let importFirst = false
      if (spec.agent === "home") {
        agentPlacement = { kind: "home_kernel" }
        agentWorkerKernelId = homeKernel.kernelId
        agentWorkerMachineId = homeKernel.machineId
      } else if (spec.agent === "other_local_slice") {
        agentPlacement = { kind: "slice_ref", sliceRef: `other-slice-${index + 1}` }
        agentWorkerKernelId = homeKernel.kernelId
        agentWorkerMachineId = homeKernel.machineId
        importFirst = true
      } else if (spec.agent === "remote_worker") {
        agentWorkerKernelId = `agent-worker-${index + 1}`
        agentWorkerMachineId = `agent-machine-${index + 1}`
        agentPlacement = { kind: "kernel_ref", kernelRef: agentWorkerKernelId }
      } else if (spec.agent === "environment_worker") {
        agentWorkerKernelId = environmentWorkerKernelId
        agentWorkerMachineId = environmentWorkerMachineId
        agentPlacement = { kind: "kernel_ref", kernelRef: agentWorkerKernelId }
      } else {
        agentWorkerKernelId = `different-worker-${index + 1}`
        agentWorkerMachineId = `different-machine-${index + 1}`
        agentPlacement = { kind: "kernel_ref", kernelRef: agentWorkerKernelId }
      }
      return {
        id: spec.id,
        roomId,
        environmentSliceRef,
        environmentId: `environment-${index + 1}`,
        tabId: `tab-${index + 1}`,
        environmentWorkerKernelId,
        environmentWorkerMachineId,
        agentWorkerKernelId,
        agentWorkerMachineId,
        agentPlacement,
        importFirst,
      }
    }),
  }
}

function validEvidence(row) {
  const agentId = `agent-${row.id}`
  const actorId = `agent:${agentId}`
  const remote = row.agentPlacement.kind === "kernel_ref"
    || (row.agentPlacement.kind === "slice_ref" && row.agentWorkerKernelId !== homeKernel.kernelId)
  const remoteExecution = remote ? {
    worker_kernel_id: row.agentWorkerKernelId,
    worker_machine_id: row.agentWorkerMachineId,
    execution_lease_id: `lease-${row.id}`,
    leased_agent_id: `leased-${agentId}`,
  } : null
  const browserAction = {
    actionId: `browser-${row.id}`,
    actorId,
    roomId: row.roomId,
    mode: "browser",
    kind: "click",
    state: "completed",
    sequence: 11,
    runtimeGeneration: 1,
    targetTabId: row.tabId,
    desktopTarget: false,
  }
  const computerAction = {
    actionId: `computer-${row.id}`,
    actorId,
    roomId: row.roomId,
    mode: "computer",
    kind: "pointer_click",
    state: "completed",
    sequence: 12,
    runtimeGeneration: 1,
    targetTabId: null,
    desktopTarget: true,
  }
  const actionTargets = (action) => [
    ...(action.desktopTarget ? [{ kind: "desktop" }] : []),
    ...(action.targetTabId ? [{ kind: "browser_tab", id: action.targetTabId }] : []),
  ]
  const historyEntry = (action) => ({
    roomId: row.roomId,
    action_id: action.actionId,
    actor_id: action.actorId,
    mode: action.mode,
    kind: action.kind,
    state: action.state,
    sequence: action.sequence,
    runtime_generation: action.runtimeGeneration,
    targets: actionTargets(action),
  })
  const ledgerEntry = ({ actionId, actorId: actionActor, mode, kind, state, sequence, runtimeGeneration, targetTabId, desktopTarget }) => ({
    action_id: actionId,
    actor_id: actionActor,
    mode,
    kind,
    state,
    sequence,
    runtime_generation: runtimeGeneration,
    targets: [
      ...(desktopTarget ? [{ kind: "desktop" }] : []),
      ...(targetTabId ? [{ kind: "browser_tab", id: targetTabId }] : []),
    ],
  })
  const snapshot = () => ({
    roomId: row.roomId,
    environmentId: row.environmentId,
    runtimeGeneration: 1,
    focusedTabId: row.tabId,
    focusedTabPresent: true,
  })
  return {
    baselineSequence: 10,
    environment: {
      roomId: row.roomId,
      environmentId: row.environmentId,
      runtimeGeneration: 1,
      sliceRef: row.environmentSliceRef,
      workerKernelId: row.environmentWorkerKernelId,
      workerMachineId: row.environmentWorkerMachineId,
      focusedTabId: row.tabId,
      focusedTabPresent: true,
      actors: [{ actorId, kind: "agent" }],
      actions: [ledgerEntry(browserAction), ledgerEntry(computerAction)],
    },
    agent: {
      id: agentId,
      roomId: row.roomId,
      placementKind: row.agentPlacement.kind,
      workerKernelId: row.agentWorkerKernelId,
      workerMachineId: row.agentWorkerMachineId,
      remoteExecution,
      membershipSliceIds: row.agentPlacement.kind === "slice_ref" ? [row.agentPlacement.sliceRef] : [],
      sliceRoomIds: row.agentPlacement.kind === "slice_ref" ? [row.roomId] : [],
    },
    browserAction,
    computerAction,
    actionHistory: [historyEntry(browserAction), historyEntry(computerAction)],
    snapshots: [snapshot(), snapshot(), snapshot(), snapshot()],
    webView: {
      source: "public-selkies-display",
      sliceRef: row.environmentSliceRef,
      kind: "selkies",
      streamId: `stream-${row.id}`,
      videoStarted: true,
      frameRecordType: 4,
      frameByteLength: 1024,
      roomId: row.roomId,
      environmentId: row.environmentId,
      runtimeGeneration: 1,
      tabId: row.tabId,
    },
  }
}

function assertRejects(run, pattern) {
  assert.throws(run, pattern)
}
