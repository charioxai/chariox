#!/usr/bin/env node

import assert from "node:assert/strict"
import { randomUUID } from "node:crypto"
import { readFile, stat } from "node:fs/promises"
import path from "node:path"
import { pathToFileURL } from "node:url"

import { runRoomRealProviderAction } from "./lib/live-room-real-provider.mjs"

const repoRoot = path.resolve(import.meta.dirname, "../../..")
const configSchema = "chariox.room_placement_matrix.config.v1"
const reportSchema = "chariox.room_placement_matrix.report.v1"
// Config supplies homeKernel {url,kernelId,machineId}, provider
// {provider,model,accountProfile,effort}, and the six ordered rows. Each row
// identifies its Room, headed Environment slice/id/Tab, both worker
// kernel/machine identities, explicit agentPlacement, and importFirst flag.
const unexecutedGates = Object.freeze({
  takeover: "unexecuted: requires an explicit human Web takeover drill",
  ordering: "unexecuted: this run checks the two provider actions' sequence only, not takeover or race ordering",
  reconnect: "unexecuted: requires a real disconnect/reconnect drill",
  foreignRoomDenial: "unexecuted: requires an authenticated foreign-Room denial drill",
  forgedLeaseDenial: "unexecuted: requires a forged worker lease denial drill",
})

export const ROOM_PLACEMENT_ROWS = Object.freeze([
  Object.freeze({ id: "home_environment_home_agent", environment: "home", agent: "home" }),
  Object.freeze({ id: "home_environment_other_local_slice_agent", environment: "home", agent: "other_local_slice" }),
  Object.freeze({ id: "home_environment_remote_worker_agent", environment: "home", agent: "remote_worker" }),
  Object.freeze({ id: "remote_environment_home_agent", environment: "remote", agent: "home" }),
  Object.freeze({ id: "remote_environment_worker_agent", environment: "remote", agent: "environment_worker" }),
  Object.freeze({ id: "remote_environment_different_placement_agent", environment: "remote", agent: "different_slice_or_worker" }),
])

export function validateRoomPlacementMatrixConfig(config) {
  assert.ok(config && typeof config === "object" && !Array.isArray(config), "matrix config must be an object")
  assert.equal(config.schema, configSchema, "unsupported Room placement matrix config schema")
  const home = config.homeKernel
  const homeKernelUrl = requireText(home?.url, "homeKernel.url")
  assert.ok(isLocalKernelEndpoint(homeKernelUrl), "homeKernel.url must identify a local Unix socket or loopback kernel")
  requireText(home?.kernelId, "homeKernel.kernelId")
  requireText(home?.machineId, "homeKernel.machineId")
  const provider = config.provider
  assert.ok(provider && typeof provider === "object", "provider selection is required")
  assert.ok(["codex", "claude", "opencode"].includes(provider.provider), "select an official provider")
  requireText(provider.model, "provider.model")
  requireText(provider.accountProfile, "provider.accountProfile")
  requireText(provider.effort, "provider.effort")
  assert.ok(Array.isArray(config.rows) && config.rows.length === ROOM_PLACEMENT_ROWS.length,
    "config must contain each of the six Room placement rows exactly once")

  const ids = config.rows.map((row) => row?.id)
  assert.deepEqual(ids, ROOM_PLACEMENT_ROWS.map((row) => row.id),
    "rows must use the six canonical placement identities in order")
  for (let index = 0; index < ROOM_PLACEMENT_ROWS.length; index += 1) {
    validateRowConfig(config.rows[index], ROOM_PLACEMENT_ROWS[index], home)
  }
  return config
}

function validateRowConfig(row, spec, home) {
  requireText(row.roomId, `${spec.id}.roomId`)
  requireText(row.environmentSliceRef, `${spec.id}.environmentSliceRef`)
  requireText(row.environmentId, `${spec.id}.environmentId`)
  requireText(row.tabId, `${spec.id}.tabId`)
  requireText(row.environmentWorkerKernelId, `${spec.id}.environmentWorkerKernelId`)
  requireText(row.environmentWorkerMachineId, `${spec.id}.environmentWorkerMachineId`)
  requireText(row.agentWorkerKernelId, `${spec.id}.agentWorkerKernelId`)
  requireText(row.agentWorkerMachineId, `${spec.id}.agentWorkerMachineId`)
  assert.equal(typeof row.importFirst, "boolean", `${spec.id}.importFirst must be explicit`)
  const placement = row.agentPlacement
  assert.ok(placement && typeof placement === "object" && !Array.isArray(placement),
    `${spec.id}.agentPlacement is required`)

  if (spec.environment === "home") {
    assert.equal(row.environmentWorkerMachineId, home.machineId, `${spec.id} must use the home worker machine`)
  } else {
    assert.ok(row.environmentWorkerKernelId !== home.kernelId
      && row.environmentWorkerMachineId !== home.machineId,
    `${spec.id} must use a remote Environment worker`)
  }

  if (spec.agent === "home") {
    assert.deepEqual(placement, { kind: "home_kernel" }, `${spec.id} must select a home-kernel agent`)
    assert.equal(row.agentWorkerKernelId, home.kernelId)
    assert.equal(row.agentWorkerMachineId, home.machineId)
    assert.equal(row.importFirst, false, "home-kernel account import is unsupported by this runner")
  } else if (spec.agent === "other_local_slice") {
    assert.equal(placement.kind, "slice_ref", `${spec.id} must select a slice agent`)
    requireText(placement.sliceRef, `${spec.id}.agentPlacement.sliceRef`)
    assert.notEqual(placement.sliceRef, row.environmentSliceRef, `${spec.id} must use a different slice`)
    assert.equal(row.agentWorkerMachineId, home.machineId, `${spec.id} agent slice must be on the home machine`)
  } else if (spec.agent === "remote_worker") {
    assert.equal(placement.kind, "kernel_ref", `${spec.id} must select a remote-worker agent`)
    assert.equal(placement.kernelRef, row.agentWorkerKernelId)
    assert.ok(row.agentWorkerKernelId !== home.kernelId && row.agentWorkerMachineId !== home.machineId,
      `${spec.id} agent must be on a remote worker`)
    assert.equal(row.importFirst, false, "remote-worker account import is unsupported by this runner")
  } else if (spec.agent === "environment_worker") {
    assert.equal(placement.kind, "kernel_ref", `${spec.id} must select an Environment-worker agent`)
    assert.equal(placement.kernelRef, row.environmentWorkerKernelId,
      `${spec.id} agent kernel_ref must equal the Environment worker`)
    assert.equal(row.agentWorkerKernelId, row.environmentWorkerKernelId,
      `${spec.id} agent kernel identity must equal the Environment worker`)
    assert.equal(row.agentWorkerMachineId, row.environmentWorkerMachineId,
      `${spec.id} agent machine identity must equal the Environment worker`)
    assert.equal(row.importFirst, false, "remote-worker account import is unsupported by this runner")
  } else {
    assert.ok(placement.kind === "slice_ref" || placement.kind === "kernel_ref",
      `${spec.id} must select a different slice or remote worker`)
    if (placement.kind === "slice_ref") {
      requireText(placement.sliceRef, `${spec.id}.agentPlacement.sliceRef`)
      assert.notEqual(placement.sliceRef, row.environmentSliceRef, `${spec.id} must use a different slice`)
    } else {
      assert.equal(placement.kernelRef, row.agentWorkerKernelId)
      assert.notEqual(row.agentWorkerKernelId, row.environmentWorkerKernelId,
        `${spec.id} remote agent must differ from the Environment worker`)
      assert.notEqual(row.agentWorkerKernelId, home.kernelId,
        `${spec.id} must select a non-home remote worker`)
    }
  }
  if (placement.kind !== "slice_ref") {
    assert.equal(row.importFirst, false, `${spec.id} account import is unsupported for this placement`)
  }
}

/** Pure evidence validator. Its result is not live evidence unless the runner obtains the inputs below from public kernel/provider calls. */
export function evaluateRoomPlacementRow({ row, homeKernel, evidence } = {}) {
  const violations = []
  const require = (condition, code) => { if (!condition) violations.push(code) }
  const spec = ROOM_PLACEMENT_ROWS.find((item) => item.id === row?.id)
  if (!spec || !evidence || !homeKernel || !row) {
    return { ok: false, violations: ["row_or_evidence_missing"], fullAcceptance: "not_proven", unexecutedGates }
  }
  try {
    validateRowConfig(row, spec, homeKernel)
  } catch {
    return { ok: false, violations: ["row_configuration_invalid"], fullAcceptance: "not_proven", unexecutedGates }
  }

  const env = evidence.environment
  const snapshots = Array.isArray(evidence.snapshots) ? evidence.snapshots : []
  require(env?.roomId === row.roomId, "environment_room_mismatch")
  require(env?.environmentId === row.environmentId, "environment_id_mismatch")
  require(env?.sliceRef === row.environmentSliceRef, "environment_slice_mismatch")
  require(env?.workerKernelId === row.environmentWorkerKernelId
    && env?.workerMachineId === row.environmentWorkerMachineId, "environment_worker_mismatch")
  require(Number.isSafeInteger(env?.runtimeGeneration), "environment_runtime_generation_missing")
  require(env?.focusedTabId === row.tabId && env?.focusedTabPresent === true, "environment_stable_tab_missing")
  if (spec.environment === "home") {
    require(env?.workerMachineId === homeKernel.machineId,
      "home_environment_was_substituted")
  } else {
    require(env?.workerKernelId !== homeKernel.kernelId && env?.workerMachineId !== homeKernel.machineId,
      "remote_environment_was_substituted")
  }

  const agent = evidence.agent
  const placement = row.agentPlacement
  require(agent?.id && agent.roomId === row.roomId, "agent_room_mismatch")
  require(agent?.placementKind === placement.kind, "agent_placement_kind_mismatch")
  require(agent?.workerKernelId === row.agentWorkerKernelId
    && agent?.workerMachineId === row.agentWorkerMachineId, "agent_worker_mismatch")
  const membershipIds = agent?.membershipSliceIds
  require(Array.isArray(membershipIds), "agent_slice_membership_unavailable")
  if (placement.kind === "home_kernel") {
    require(agent?.remoteExecution === null && membershipIds?.length === 0,
      "home_agent_binding_or_duplicate_slice")
    require(agent?.workerKernelId === homeKernel.kernelId && agent?.workerMachineId === homeKernel.machineId,
      "home_agent_was_substituted")
  } else if (placement.kind === "kernel_ref") {
    require(membershipIds?.length === 0, "remote_agent_duplicated_on_slice")
    require(agent?.remoteExecution?.worker_kernel_id === placement.kernelRef
      && agent?.remoteExecution?.worker_machine_id === row.agentWorkerMachineId
      && nonempty(agent?.remoteExecution?.execution_lease_id)
      && nonempty(agent?.remoteExecution?.leased_agent_id), "remote_agent_lease_binding_mismatch")
  } else {
    require(membershipIds?.length === 1 && membershipIds[0] === placement.sliceRef,
      "agent_slice_membership_mismatch")
    require(Array.isArray(agent?.sliceRoomIds) && agent.sliceRoomIds.includes(row.roomId),
      "agent_slice_room_mismatch")
    if (agent?.remoteExecution) {
      require(agent.remoteExecution.worker_kernel_id === row.agentWorkerKernelId
        && agent.remoteExecution.worker_machine_id === row.agentWorkerMachineId
        && nonempty(agent.remoteExecution.execution_lease_id)
        && nonempty(agent.remoteExecution.leased_agent_id), "slice_agent_lease_binding_mismatch")
    } else {
      require(agent?.workerKernelId === homeKernel.kernelId
        && agent?.workerMachineId === homeKernel.machineId, "local_slice_agent_worker_mismatch")
    }
  }

  const browser = evidence.browserAction
  const computer = evidence.computerAction
  const actorId = `agent:${agent?.id ?? ""}`
  const sameRoomEnvironment = env?.roomId === row.roomId && env?.environmentId === row.environmentId
    && env?.sliceRef === row.environmentSliceRef
    && snapshots.length === 4
    && snapshots.every((snapshot) => snapshot?.runtimeGeneration === env?.runtimeGeneration)
    && snapshots.every((snapshot) => snapshot?.roomId === row.roomId
      && snapshot?.environmentId === row.environmentId)
    && evidence.webView?.roomId === row.roomId && evidence.webView?.environmentId === row.environmentId
  require(browser?.roomId === row.roomId && computer?.roomId === row.roomId, "action_room_mismatch")
  require(browser?.actorId === actorId && computer?.actorId === actorId, "browser_computer_actor_mismatch")
  require(Array.isArray(env?.actors)
    && env.actors.some((actor) => actor?.actorId === actorId && actor.kind === "agent"),
  "room_agent_actor_missing")
  require(browser?.actionId && computer?.actionId && browser.actionId !== computer.actionId,
    "browser_computer_action_identity_invalid")
  require(browser?.mode === "browser" && browser?.kind === "click" && browser?.state === "completed",
    "browser_action_not_completed")
  require(computer?.mode === "computer" && computer?.kind === "pointer_click" && computer?.state === "completed",
    "computer_action_not_completed")
  require(browser?.runtimeGeneration === env?.runtimeGeneration
    && computer?.runtimeGeneration === env?.runtimeGeneration,
  "provider_action_environment_generation_mismatch")
  require(Number.isSafeInteger(evidence.baselineSequence)
    && Number.isSafeInteger(browser?.sequence) && browser.sequence > evidence.baselineSequence
    && Number.isSafeInteger(computer?.sequence) && computer.sequence > browser.sequence,
  "provider_action_sequence_mismatch")
  require(browser?.targetTabId === row.tabId, "browser_action_tab_mismatch")
  require(browser?.desktopTarget === false && computer?.desktopTarget === true,
    "browser_computer_target_mismatch")
  require(computer?.targetTabId === null || computer?.targetTabId === row.tabId,
    "computer_action_tab_mismatch")
  require(historyContains(evidence.actionHistory, row.roomId, browser)
    && historyContains(evidence.actionHistory, row.roomId, computer), "public_action_history_mismatch")
  require(environmentLedgerContains(env?.actions, browser)
    && environmentLedgerContains(env?.actions, computer), "room_environment_action_ledger_mismatch")

  require(Array.isArray(snapshots) && snapshots.length === 4, "public_room_snapshots_incomplete")
  for (const snapshot of snapshots) {
    require(snapshot?.roomId === row.roomId && snapshot?.environmentId === row.environmentId,
      "snapshot_room_environment_changed")
    require(snapshot?.runtimeGeneration === env?.runtimeGeneration,
      "snapshot_environment_generation_changed")
    require(snapshot?.focusedTabId === row.tabId && snapshot?.focusedTabPresent === true,
      "snapshot_stable_tab_changed")
  }

  const view = evidence.webView
  require(view?.source === "public-selkies-display" && view?.sliceRef === row.environmentSliceRef
    && view?.kind === "selkies" && nonempty(view?.streamId)
    && view?.videoStarted === true && view?.frameRecordType === 4
    && Number.isSafeInteger(view?.frameByteLength) && view.frameByteLength > 10,
  "public_web_view_not_visible")
  require(view?.roomId === row.roomId && view?.environmentId === row.environmentId
    && view?.runtimeGeneration === env?.runtimeGeneration
    && view?.tabId === row.tabId, "web_view_room_environment_tab_mismatch")
  const sameAgent = agent?.id && agent.roomId === row.roomId
    && browser?.actorId === actorId && computer?.actorId === actorId
  const sameStableTab = env?.focusedTabId === row.tabId && env?.focusedTabPresent === true
    && snapshots.length === 4
    && snapshots.every((snapshot) => snapshot?.focusedTabId === row.tabId
      && snapshot?.focusedTabPresent === true)
    && browser?.targetTabId === row.tabId
    && view?.tabId === row.tabId

  return {
    ok: violations.length === 0,
    violations,
    sameRoomEnvironment,
    sameAgent: Boolean(sameAgent),
    sameStableTab,
    fullAcceptance: "not_proven",
    unexecutedGates,
  }
}

function historyContains(history, roomId, expected) {
  if (!Array.isArray(history)) return false
  const matches = history.filter((entry) => entry?.action_id === expected.actionId)
  if (matches.length !== 1) return false
  const targets = Array.isArray(matches[0].targets) ? matches[0].targets : []
  const browserTargets = targets.filter((target) => target?.kind === "browser_tab")
  const targetTabId = browserTargets.length === 1 ? browserTargets[0].id : null
  const desktopTarget = targets.filter((target) => target?.kind === "desktop").length === 1
  return matches[0].roomId === roomId
    && matches[0].actor_id === expected.actorId
    && matches[0].mode === expected.mode && matches[0].kind === expected.kind
    && matches[0].state === expected.state && matches[0].sequence === expected.sequence
    && matches[0].runtime_generation === expected.runtimeGeneration
    && desktopTarget === expected.desktopTarget
    && targetTabId === expected.targetTabId
}

function environmentLedgerContains(actions, expected) {
  if (!Array.isArray(actions)) return false
  const matches = actions.filter((entry) => entry?.action_id === expected.actionId)
  if (matches.length !== 1) return false
  const targets = Array.isArray(matches[0].targets) ? matches[0].targets : []
  const browserTargets = targets.filter((target) => target?.kind === "browser_tab")
  const targetTabId = browserTargets.length === 1 ? browserTargets[0].id : null
  const desktopTarget = targets.filter((target) => target?.kind === "desktop").length === 1
  return matches[0].actor_id === expected.actorId
    && matches[0].mode === expected.mode && matches[0].kind === expected.kind
    && matches[0].state === expected.state && matches[0].sequence === expected.sequence
    && matches[0].runtime_generation === expected.runtimeGeneration
    && desktopTarget === expected.desktopTarget && targetTabId === expected.targetTabId
}

export async function runRoomPlacementMatrix(config) {
  validateRoomPlacementMatrixConfig(config)
  const [{ LocalIpcClient }, requestApi, { openSelkiesDisplayStream }] = await Promise.all([
    import("../../../packages/kernel-client/dist/ipc.js"),
    import("../../../packages/kernel-client/dist/ipc-requests.js"),
    import("../../../packages/kernel-client/dist/display-stream.js"),
  ])
  const client = new LocalIpcClient(config.homeKernel.url)
  const rows = []
  try {
    const homeStatus = variant(await client.send(requestApi.relayStatusRequest()), "RelayStatus").status
    assert.equal(homeStatus.daemon_id, config.homeKernel.kernelId, "connected home kernel identity mismatch")
    assert.equal(homeStatus.machine_id, config.homeKernel.machineId, "connected home machine identity mismatch")
    for (const row of config.rows) {
      let stage = "public_preflight"
      try {
        const result = await runConfiguredRow({ client, requestApi, openSelkiesDisplayStream, config, row,
          setStage: (value) => { stage = value } })
        rows.push(result)
      } catch {
        rows.push({ id: row.id, status: "failed", failureCode: "row_acceptance_failed", failedStage: stage })
        break
      }
    }
  } finally {
    await client.close?.().catch(() => {})
  }
  const complete = rows.length === ROOM_PLACEMENT_ROWS.length && rows.every((row) => row.status === "placement_proof_only")
  return {
    schema: reportSchema,
    status: complete ? "placement_proofs_collected" : "partial_failure",
    fullAcceptance: "not_proven",
    evidenceSource: "official-provider-run-and-public-kernel-requests",
    placementProofOnly: true,
    rows,
    unexecutedGates,
  }
}

async function runConfiguredRow({ client, requestApi, openSelkiesDisplayStream, config, row, setStage }) {
  const home = config.homeKernel
  const binding = variant(await client.send(requestApi.getRoomEnvironmentSliceRequest(row.roomId)),
    "RoomEnvironmentSlice").binding
  assert.ok(binding && binding.session_id === row.roomId
    && binding.slice_id === row.environmentSliceRef
    && binding.owner_kernel_id === home.kernelId, "Room Environment binding does not match configured home-owned slice")
  const environmentSlice = await readSlice(client, requestApi, row.environmentSliceRef)
  assert.equal(binding.worker_kernel_ref, environmentSlice.worker_kernel_ref,
    "Room Environment worker reference differs from its public slice")
  assert.equal(environmentSlice.owner_kernel_id, home.kernelId)
  assert.equal(environmentSlice.owner_machine_id, home.machineId)
  assert.equal(environmentSlice.display_mode, "headed")
  const environmentWorker = sliceWorker(environmentSlice)
  assert.equal(environmentWorker.kernelId, row.environmentWorkerKernelId)
  assert.equal(environmentWorker.machineId, row.environmentWorkerMachineId)

  const before = await readRoomSnapshot(client, requestApi, row)
  assert.equal(before.environmentId, row.environmentId)
  assert.equal(before.focusedTabId, row.tabId)
  const baselineHistory = await readActionHistory(client, requestApi, row.roomId)
  const baselineSequence = baselineHistory.reduce((latest, action) => Math.max(latest, action.sequence), 0)
  const placement = row.agentPlacement
  const baseInput = {
    client,
    requests: requestApi,
    sessionId: row.roomId,
    sliceId: row.environmentSliceRef,
    agentPlacement: placement,
    workspace: row.workspaceId,
    withTimeout,
    waitFor,
    checkpoint: async () => {},
    options: {
      provider: config.provider.provider,
      model: config.provider.model,
      accountProfile: config.provider.accountProfile,
      effort: config.provider.effort,
      importFirst: row.importFirst,
    },
  }

  setStage("official_provider_browser")
  const browserResult = await runRoomRealProviderAction({
    ...baseInput,
    options: { ...baseInput.options, mode: "browser", browserTask: "click" },
  })
  const afterBrowser = await readRoomSnapshot(client, requestApi, row)
  const afterBrowserAgent = await readSessionAgent(client, requestApi, row.roomId, browserResult.agentId)

  setStage("official_provider_computer")
  const computerResult = await runRoomRealProviderAction({
    ...baseInput,
    agent: afterBrowserAgent,
    options: { ...baseInput.options, importFirst: false, mode: "computer" },
  })
  assert.equal(computerResult.agentId, browserResult.agentId,
    "Browser and Computer provider actions must reuse one Room agent")
  assert.equal(computerResult.actorId, browserResult.actorId,
    "Browser and Computer provider actions must retain one action actor")
  const afterComputer = await readRoomSnapshot(client, requestApi, row)
  const finalAgent = await readSessionAgent(client, requestApi, row.roomId, browserResult.agentId)
  const allSlices = await readSlices(client, requestApi)
  const memberships = allSlices.filter((slice) => slice.agent_ids?.includes?.(finalAgent.id))
  const agentProof = await proveAgentPlacement({ client, requestApi, row, home, placement, agent: finalAgent, memberships })

  setStage("public_action_history")
  const actionHistory = (await readActionHistory(client, requestApi, row.roomId))
    .map((action) => ({ roomId: row.roomId, ...action }))
  const browserAction = historyAction(actionHistory, browserResult.actionId, browserResult)
  const computerAction = historyAction(actionHistory, computerResult.actionId, computerResult)

  setStage("public_web_view")
  const attachment = variant(await client.send(requestApi.attachToSessionRequest(
    row.roomId, `placement-matrix-${randomUUID()}`,
  )), "SessionAttached").attachment
  assert.ok(nonempty(attachment?.id), "Web View attachment identity is unavailable")
  let stream
  let viewSnapshot
  let frame
  try {
    stream = await openSelkiesDisplayStream({
      client,
      sliceId: row.environmentSliceRef,
      sessionId: row.roomId,
      attachmentId: attachment.id,
      connectTimeoutMs: 15_000,
    })
    await stream.sendControl("START_VIDEO")
    const started = await stream.receive({ timeoutMs: 15_000 })
    assert.equal(started.kind, "text")
    assert.equal(new TextDecoder().decode(started.data), "VIDEO_STARTED")
    frame = await stream.receive({ timeoutMs: 15_000 })
    viewSnapshot = await readRoomSnapshot(client, requestApi, row)
  } finally {
    await stream?.close().catch(() => {})
    await client.send(requestApi.detachFromSessionRequest(attachment.id)).catch(() => {})
  }
  const finalBinding = variant(await client.send(requestApi.getRoomEnvironmentSliceRequest(row.roomId)),
    "RoomEnvironmentSlice").binding
  assert.ok(finalBinding && finalBinding.slice_id === row.environmentSliceRef
    && finalBinding.session_id === row.roomId
    && finalBinding.owner_kernel_id === binding.owner_kernel_id
    && finalBinding.worker_kernel_ref === binding.worker_kernel_ref,
  "Room Environment binding changed during Web View observation")
  const finalEnvironmentSlice = await readSlice(client, requestApi, row.environmentSliceRef)
  assert.deepEqual(sliceWorker(finalEnvironmentSlice), environmentWorker,
    "Room Environment worker placement changed during provider actions or Web View observation")
  assert.equal(stream.endpoint.slice_id, row.environmentSliceRef)

  const evidence = {
    baselineSequence,
    environment: {
      roomId: before.roomId,
      environmentId: before.environmentId,
      runtimeGeneration: before.runtimeGeneration,
      sliceRef: environmentSlice.id,
      workerKernelId: environmentWorker.kernelId,
      workerMachineId: environmentWorker.machineId,
      focusedTabId: before.focusedTabId,
      focusedTabPresent: before.focusedTabPresent,
      actors: viewSnapshot.actors,
      actions: viewSnapshot.actions,
    },
    agent: agentProof,
    browserAction,
    computerAction,
    actionHistory,
    snapshots: [before, afterBrowser, afterComputer, viewSnapshot],
    webView: {
      source: "public-selkies-display",
      sliceRef: stream.endpoint.slice_id,
      kind: stream.endpoint.kind,
      streamId: stream.endpoint.stream_id,
      videoStarted: true,
      frameRecordType: frame?.kind === "binary" ? frame.data[0] : null,
      frameByteLength: frame?.data?.byteLength ?? 0,
      roomId: viewSnapshot.roomId,
      environmentId: viewSnapshot.environmentId,
      runtimeGeneration: viewSnapshot.runtimeGeneration,
      tabId: viewSnapshot.focusedTabId,
    },
  }
  const evaluation = evaluateRoomPlacementRow({ row, homeKernel: home, evidence })
  assert.ok(evaluation.ok, `public placement evidence rejected: ${evaluation.violations.join(",")}`)
  return {
    id: row.id,
    status: "placement_proof_only",
    roomId: row.roomId,
    environmentId: row.environmentId,
    environmentSliceRef: row.environmentSliceRef,
    environmentWorkerKernelId: environmentWorker.kernelId,
    environmentWorkerMachineId: environmentWorker.machineId,
    agentId: finalAgent.id,
    agentPlacement: placement,
    browserActionId: browserAction.actionId,
    computerActionId: computerAction.actionId,
    actorId: browserAction.actorId,
    tabId: row.tabId,
    actions: [browserAction, computerAction].map((action) => ({
      actionId: action.actionId,
      actorId: action.actorId,
      mode: action.mode,
      kind: action.kind,
      sequence: action.sequence,
      runtimeGeneration: action.runtimeGeneration,
    })),
    webView: { source: evidence.webView.source, kind: evidence.webView.kind, streamId: evidence.webView.streamId, visibleFrameBytes: evidence.webView.frameByteLength },
    checks: { sameRoomEnvironment: true, sameAgent: true, sameStableTab: true, actionAttribution: true, noDuplicateAgentPlacement: true },
    fullAcceptance: evaluation.fullAcceptance,
    unexecutedGates,
  }
}

async function proveAgentPlacement({ client, requestApi, row, home, placement, agent, memberships }) {
  assert.equal(agent.session_id, row.roomId)
  const location = placement.kind === "home_kernel"
    ? { kernelId: home.kernelId, machineId: home.machineId }
    : placement.kind === "kernel_ref"
      ? { kernelId: agent.remote_execution?.worker_kernel_id, machineId: agent.remote_execution?.worker_machine_id }
      : await proveSliceAgentPlacement(client, requestApi, placement.sliceRef, row, home)
  const membershipSliceIds = memberships.map((slice) => slice.id).sort()
  const sliceRoomIds = placement.kind === "slice_ref"
    ? roomIdsForSlice(memberships.find((slice) => slice.id === placement.sliceRef))
    : []
  return {
    id: agent.id,
    roomId: agent.session_id,
    placementKind: placement.kind,
    workerKernelId: location.kernelId,
    workerMachineId: location.machineId,
    remoteExecution: agent.remote_execution,
    membershipSliceIds,
    sliceRoomIds,
  }
}

async function proveSliceAgentPlacement(client, requestApi, sliceRef, row, home) {
  const slice = await readSlice(client, requestApi, sliceRef)
  assert.equal(slice.owner_kernel_id, home.kernelId)
  assert.equal(slice.owner_machine_id, home.machineId)
  assert.ok(roomIdsForSlice(slice).includes(row.roomId), "agent slice is not bound to the requested Room")
  return sliceWorker(slice)
}

async function readRoomSnapshot(client, requestApi, row) {
  const environment = variant(await client.send(requestApi.getRoomEnvironmentStateRequest(row.roomId)),
    "RoomEnvironmentState").environment
  assert.ok(environment && typeof environment === "object")
  const focusedTabId = environment.focused_tab_id
  return {
    roomId: environment.session_id,
    environmentId: environment.environment_id,
    runtimeGeneration: environment.runtime_generation,
    focusedTabId,
    focusedTabPresent: nonempty(focusedTabId)
      && Array.isArray(environment.tabs)
      && environment.tabs.some((tab) => tab?.tab_id === focusedTabId && tab.focused === true),
    actors: Array.isArray(environment.actors)
      ? environment.actors.map((actor) => ({ actorId: actor?.actor_id, kind: actor?.kind })) : [],
    actions: Array.isArray(environment.actions) ? environment.actions : [],
  }
}

async function readSessionAgent(client, requestApi, roomId, agentId) {
  const session = variant(await client.send(requestApi.getSessionStateRequest(roomId)), "SessionState").session
  assert.ok(session && session.id === roomId)
  const agents = session.agents?.filter((agent) => agent?.id === agentId) ?? []
  assert.equal(agents.length, 1, "authoritative Room agent is missing or duplicated")
  return agents[0]
}

async function readActionHistory(client, requestApi, roomId) {
  const page = variant(await client.send(requestApi.listRoomEnvironmentActionHistoryRequest(roomId, null, 100)),
    "RoomEnvironmentActionHistoryListed").page
  assert.ok(Array.isArray(page?.actions), "public Room action history is unavailable")
  assert.ok(page.actions.every((action) => Number.isSafeInteger(action?.sequence)),
    "public Room action history has an invalid sequence")
  return page.actions
}

function historyAction(history, actionId, result) {
  const matches = history.filter((action) => action.action_id === actionId)
  assert.equal(matches.length, 1, "official provider action is missing or duplicated in public history")
  const action = matches[0]
  assert.equal(action.actor_id, result.actorId)
  const targets = Array.isArray(action.targets) ? action.targets : []
  const browserTargets = targets.filter((target) => target?.kind === "browser_tab")
  return {
    actionId: action.action_id,
    actorId: action.actor_id,
    roomId: history.find((entry) => entry.action_id === actionId)?.roomId,
    mode: action.mode,
    kind: action.kind,
    state: action.state,
    sequence: action.sequence,
    runtimeGeneration: action.runtime_generation,
    targetTabId: browserTargets.length === 1 ? browserTargets[0].id : null,
    desktopTarget: targets.filter((target) => target?.kind === "desktop").length === 1,
  }
}

async function readSlice(client, requestApi, sliceRef) {
  const slice = variant(await client.send(requestApi.getSliceRequest(sliceRef)), "Slice").slice
  assert.ok(slice && slice.id === sliceRef && slice.status === "running", "configured slice is not running")
  return slice
}

async function readSlices(client, requestApi) {
  const slices = variant(await client.send(requestApi.listSlicesRequest()), "SlicesListed").slices
  assert.ok(Array.isArray(slices), "authoritative slice inventory is unavailable")
  return slices
}

function sliceWorker(slice) {
  return {
    kernelId: nonempty(slice?.worker_kernel_id) ? slice.worker_kernel_id : slice?.owner_kernel_id,
    machineId: nonempty(slice?.worker_machine_id) ? slice.worker_machine_id : slice?.owner_machine_id,
  }
}

function roomIdsForSlice(slice) {
  if (!slice) return []
  return [...new Set([slice.session_id, slice.environment_session_id, ...(slice.session_ids ?? [])]
    .filter((value) => typeof value === "string" && value.length > 0))].sort()
}

function variant(response, key) {
  assert.ok(response && Object.hasOwn(response, key), `public kernel response is missing ${key}`)
  return response[key]
}

function requireText(value, name) {
  assert.ok(nonempty(value), `${name} must be a nonempty string`)
  return value.trim()
}

function nonempty(value) {
  return typeof value === "string" && value.trim().length > 0
}

function isLocalKernelEndpoint(endpoint) {
  if (path.isAbsolute(endpoint)) return true
  let url
  try {
    url = new URL(endpoint)
  } catch {
    return false
  }
  return url.protocol === "ws:"
    && ["127.0.0.1", "[::1]"].includes(url.hostname)
    && url.username === "" && url.password === ""
    && url.pathname === "/" && url.search === "" && url.hash === ""
}

async function withTimeout(promise, timeoutMs, label) {
  let timer
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`${label} timed out`)), timeoutMs) }),
    ])
  } finally {
    clearTimeout(timer)
  }
}

async function waitFor(check, timeoutMs, label) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const result = await check()
    if (result) return result
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
  throw new Error(`${label} timed out`)
}

async function readPrivateConfig(filePath) {
  assert.ok(path.isAbsolute(filePath), "config path must be absolute")
  const resolved = path.resolve(filePath)
  const relative = path.relative(repoRoot, resolved)
  assert.ok(relative === ".." || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative),
    "config must be stored outside the repository")
  const info = await stat(resolved)
  assert.ok(info.isFile() && (info.mode & 0o077) === 0, "config must be a private regular file with mode 0600 or stricter")
  return JSON.parse(await readFile(resolved, "utf8"))
}

async function main(argv) {
  if (argv[0] !== "--execute-live" || argv[1] !== "--config" || !argv[2]) {
    throw new Error("usage: live-room-placement-matrix.mjs --execute-live --config /absolute/private-config.json")
  }
  const config = validateRoomPlacementMatrixConfig(await readPrivateConfig(argv[2]))
  return await runRoomPlacementMatrix(config)
}

if (process.argv[1] && pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url) {
  main(process.argv.slice(2)).then((report) => {
    process.stdout.write(`${JSON.stringify(report, null, 2)}\n`)
    if (report.status !== "placement_proofs_collected") process.exitCode = 1
  }).catch(() => {
    process.stderr.write(`${JSON.stringify({ schema: reportSchema, status: "failed", failureCode: "placement_matrix_failed" })}\n`)
    process.exitCode = 1
  })
}
