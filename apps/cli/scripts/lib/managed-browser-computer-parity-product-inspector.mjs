import { observeManagedParityHost } from "./managed-browser-computer-parity-host-observer.mjs"
import { loadReviewedManagedParityInspectorModule } from "./managed-browser-computer-parity-cli.mjs"
import { createHash } from "node:crypto"
import { writeFile } from "node:fs/promises"
import path from "node:path"
import { enumerateManagedParityEvidenceRoot } from "./managed-browser-computer-parity-evidence.mjs"

export const MANAGED_BROWSER_COMPUTER_PARITY_INSPECTOR_IDENTITY = "chariox-managed-parity-product-inspector-v1"

export async function createManagedBrowserComputerParityInspector({ config, evidenceRoot }) {
  const settings = config?.inspector?.observations
  if (!settings?.cloudInventoryModule?.path || !settings.machineOwnership) {
    throw new Error("reviewed Cloud inventory authority and explicit machine ownership are required")
  }
  const endpoint = process.env.CHARIOX_MANAGED_PARITY_HOME_KERNEL_URL
  if (!endpoint || (!process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN && !process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE)) {
    throw new Error("existing home kernel endpoint and standard operator authentication are required")
  }
  if (!["ws:", "wss:"].includes(new URL(endpoint).protocol)) {
    throw new Error("independent inspector requires the close-cancellable public WebSocket client")
  }
  const reviewed = await loadReviewedManagedParityInspectorModule(
    settings.cloudInventoryModule.path, settings.cloudInventoryModule,
  )
  if (typeof reviewed.imported.createManagedParityCloudInventory !== "function") {
    throw new Error("reviewed Cloud inventory factory is required")
  }
  const [{ LocalIpcClient }, requests] = await Promise.all([
    import("../../../../packages/kernel-client/dist/ipc.js"),
    import("../../../../packages/kernel-client/dist/ipc-requests.js"),
  ])
  const home = new LocalIpcClient(endpoint)
  let cloud
  let observations = 0
  try {
    cloud = await reviewed.imported.createManagedParityCloudInventory({ config, evidenceRoot })
    return createManagedParityInspectorFromAuthorities({
      config, home, requests, cloud,
      physical: createManagedParityPhysicalLedger({ host: settings.host, async observe(input) {
        const result = await observeManagedParityHost(input)
        await writeFile(path.join(evidenceRoot, `physical-observation-${String(++observations).padStart(4, "0")}.json`),
          `${JSON.stringify(result)}\n`, { mode: 0o600, flag: "wx" })
        return result
      } }),
      async inspectEvidence() {
        const scanned = await enumerateManagedParityEvidenceRoot(evidenceRoot)
        const allowedEvidencePaths = [
          ...Array.from({ length: observations }, (_, index) => `physical-observation-${String(index + 1).padStart(4, "0")}.json`),
          ...(settings.allowedEvidencePaths ?? []),
        ]
        return { ...scanned, allowedEvidencePaths }
      },
    })
  } catch (error) {
    await Promise.allSettled([home.close(), cloud?.close?.()])
    throw error
  }
}

// The injected interfaces are the normal public kernel client and a separately
// reviewed read-only Cloud/provider observer. The physical observer never runs
// provider commands or performs retirement itself.
export function createManagedParityInspectorFromAuthorities({ config, home, requests, cloud, physical, inspectEvidence }) {
  if (typeof inspectEvidence !== "function") throw new Error("physical local evidence inspection required")
  const ownership = config?.inspector?.observations?.machineOwnership
  if (!["preexisting", "run_owned"].includes(ownership?.kind)
    || ownership.machineId !== config.expected.machineId) throw new Error("explicit machine ownership binding required")
  for (const method of ["begin", "observeCreated", "beforeRetire", "inspect"]) {
    if (typeof cloud?.[method] !== "function") throw new Error("complete independent Cloud inventory authority required")
  }
  if (cloud.authority?.kind !== "read-only-cloud-inventory") throw new Error("read-only Cloud inventory authority required")
  const owned = ownership.kind === "run_owned"
  if (owned && (!ownership.creationReceipt || ownership.creationReceipt.runId !== config.runId
    || ownership.creationReceipt.machineId !== config.expected.machineId
    || ownership.creationReceipt.kernelId !== config.expected.kernelId
    || !ownership.creationReceipt.managedEnvironmentId || !ownership.creationReceipt.providerServerId
    || !ownership.creationReceipt.createOperationId
    || typeof cloud.inspectProviderMachine !== "function")) throw new Error("exact run-owned machine creation receipt required")
  const scope = { runId: config.runId, binding: { ...config.expected }, machineOwnership: structuredClone(ownership) }
  let failed = null
  let finalSample = null
  let retirement = null
  let beforeMachineDelete = null
  const cloudOwnership = { targetIds: new Set(), heartbeatIds: new Set(), kernelIds: new Set() }
  const retainCloudOwnership = (value) => {
    if (value?.runId !== scope.runId || value?.machineId !== scope.binding.machineId) {
      throw new Error("Cloud ownership receipt scope mismatch")
    }
    for (const [field, target] of [["ownedTargetIds", "targetIds"], ["ownedHeartbeatIds", "heartbeatIds"], ["ownedKernelIds", "kernelIds"]]) {
      if (!Array.isArray(value[field]) || value[field].some((id) => typeof id !== "string" || !id)) {
        throw new Error("exact Cloud ownership identity receipts required")
      }
      for (const id of value[field]) cloudOwnership[target].add(id)
    }
  }
  const cloudScope = () => ({ ...scope, ownedTargetIds: [...cloudOwnership.targetIds],
    ownedHeartbeatIds: [...cloudOwnership.heartbeatIds], ownedKernelIds: [...cloudOwnership.kernelIds] })
  const observe = async (method, receipt, options) => {
    try {
      await physical[method](receipt, options)
      retainCloudOwnership(await cloud[method]({ ...cloudScope(), receipt }, options))
    } catch (error) { failed ??= error; throw error }
  }
  const send = async (request, signal) => {
    if (signal?.aborted) throw new Error("independent product inspection aborted")
    let timer
    let closePromise = null
    let interruption = null
    const abort = () => {
      interruption ??= new Error("independent kernel observation interrupted")
      // LocalIpcClient.close synchronously retires the request lifetime and
      // rejects pending WebSocket sends, then awaits closing both socket lanes.
      closePromise ??= home.close()
    }
    signal?.addEventListener("abort", abort, { once: true })
    try {
      const pending = home.send(request)
      timer = setTimeout(() => { interruption = new Error("independent kernel observation timed out"); abort() }, 15_000)
      if (signal?.aborted) abort()
      try { return await pending } catch (error) { throw interruption ?? error }
    } finally {
      clearTimeout(timer)
      signal?.removeEventListener("abort", abort)
      if (closePromise) await closePromise
    }
  }
  const inspectProvider = async (signal) => {
    const value = await cloud.inspectProviderMachine(scope, { signal })
    if (value?.authority !== "hetzner-api" || value.providerServerId !== ownership.creationReceipt.providerServerId
      || ![200, 404].includes(value.httpStatus) || !value.observedAt) throw new Error("authoritative exact provider machine observation required")
    return value
  }
  return {
    authority: { kind: "independent-product-inspector", identity: config.inspector.identity, sha256: config.inspector.sha256 },
    async begin(_request, options) {
      try {
        await physical.begin(options)
        retainCloudOwnership(await cloud.begin(scope, options))
      } catch (error) { failed ??= error; throw error }
    },
    observeCreated: (receipt, options) => observe("observeCreated", receipt, options),
    beforeRetire: (receipt, options) => observe("beforeRetire", receipt, options),
    setBeforeMachineDelete(callback) {
      if (beforeMachineDelete || typeof callback !== "function") throw new Error("single before-machine-delete observer required")
      beforeMachineDelete = callback
    },
    getMachineRetirementEvidence() { return retirement ? structuredClone(retirement) : null },
    async run(step, _request, { signal } = {}) {
      if (step !== "cleanup.inspect") throw new Error("independent inspector only supports cleanup.inspect")
      let physicalResult
      let cloudResult
      try {
        physicalResult = await physical.inspect({ signal })
        cloudResult = await cloud.inspect(cloudScope(), { signal })
      } catch (error) { failed ??= error }
      if (owned) {
        const receipt = ownership.creationReceipt
        const current = (await send(requests.getManagedEnvironmentRequest(receipt.managedEnvironmentId), signal))?.ManagedEnvironment?.environment
        if (current?.environmentId !== receipt.managedEnvironmentId
          || current.runtimeMachineId !== config.expected.machineId
          || current.runtimeKernelId !== config.expected.kernelId) throw new Error("managed machine deletion identity mismatch")
        const providerBefore = await inspectProvider(signal)
        if (providerBefore.httpStatus !== 200 || providerBefore.creationOperationId !== receipt.createOperationId
          || providerBefore.managedEnvironmentId !== receipt.managedEnvironmentId) throw new Error("provider creation ownership mismatch")
        if (!beforeMachineDelete) throw new Error("final before-delete telemetry hook required")
        try { finalSample = await beforeMachineDelete({ signal, physical: physicalResult }) }
        catch (error) { failed ??= error }
        let response
        try { response = await send(requests.requestManagedEnvironmentLifecycleRequest({
          environmentId: receipt.managedEnvironmentId, action: "delete",
          idempotencyKey: `parity-cleanup-${createHash("sha256").update(config.runId).digest("hex")}`,
        }), signal) } catch (error) {
          // A closed client does not cancel a server-side lifecycle mutation.
          // Do not resend it; reconcile its exact provider outcome below.
          failed ??= error
        }
        const result = response?.ManagedEnvironmentLifecycleRequested?.result
        if (response && (result?.environment?.environmentId !== receipt.managedEnvironmentId
          || result?.operation?.kind !== "delete")) throw new Error("normal managed deletion receipt required")
        let providerAfter
        const deadline = Date.now() + 120_000
        do {
          providerAfter = await inspectProvider(signal)
          if (providerAfter.httpStatus === 404) break
          if (signal?.aborted) throw new Error("managed deletion observation aborted")
          await new Promise((resolve) => setTimeout(resolve, 1000))
        } while (Date.now() < deadline)
        if (providerAfter.httpStatus !== 404) throw new Error("provider machine absence was not established")
        retirement = { kind: "run_owned_machine_deleted", providerAbsence: providerAfter,
          finalBeforeDeleteSample: finalSample, finalBeforeDeletePhysical: physicalResult ?? null,
          afterDeletionTelemetry: { available: false, reason: "machine_deleted" } }
        // Query again after deletion: a pre-delete Cloud row census cannot prove
        // target/heartbeat retirement caused by machine deletion.
        cloudResult = await cloud.inspect(cloudScope(), { signal })
      }
      if (failed) throw failed
      validateCloudInventory(cloudResult, cloudScope(), config.inspector.observations.cloudInventory)
      if (physicalResult.volumes !== 0) throw new Error("retained physical volume prevents profile cleanup proof")
      const relay = (await send({ QueryFreshRemoteMachineKernels: { machine_ref: config.expected.machineId } }, signal))
        ?.FreshRemoteMachineKernelsObserved
      if (relay?.machine_ref !== config.expected.machineId || !Array.isArray(relay.kernels)
        || !Number.isFinite(relay.query_started_at_ms) || !Number.isFinite(relay.query_completed_at_ms)) {
        throw new Error("fresh scoped relay observation required")
      }
      const ownedKernelIds = cloudOwnership.kernelIds
      if (owned) ownedKernelIds.add(config.expected.kernelId)
      const activeRegistrations = relay.kernels.filter((item) => ownedKernelIds.has(item.kernel_id)).length
      const roomResponse = await send(requests.listSessionsRequest(), signal)
      const sliceResponse = await send(requests.listSlicesRequest(), signal)
      const sessions = roomResponse?.SessionsListed?.sessions
      const slices = sliceResponse?.SlicesListed?.slices
      if (!Array.isArray(sessions) || !Array.isArray(slices)) throw new Error("independent kernel Room and slice inventory required")
      const evidence = await inspectEvidence()
      if (evidence?.source !== "physical-evidence-root" || evidence.enumerationComplete !== true) {
        throw new Error("complete physical local evidence census required")
      }
      if (!Array.isArray(evidence.files) || !Array.isArray(evidence.allowedEvidencePaths)
        || evidence.files.some((file) => file.scan?.completed !== true
          || !Number.isSafeInteger(file.scan.forbiddenMatches) || file.scan.forbiddenMatches < 0)) {
        throw new Error("classified evidence files and completed leak scan required")
      }
      const allowedEvidence = new Set(evidence.allowedEvidencePaths)
      const retainedEvidenceLeakCount = evidence.files.reduce((sum, file) => sum + file.scan.forbiddenMatches
        + (allowedEvidence.has(file.relativePath) ? 0 : 1), 0)
      return {
        managedMachines: owned && !retirement ? 1 : 0,
        rooms: sessions.filter((item) => item.id === config.expected.roomId).length,
        environments: slices.filter((item) => physicalResult.ownedSliceIds.includes(item.id)).length,
        processes: physicalResult.processes, listeners: physicalResult.listeners,
        containers: physicalResult.containers, profiles: physicalResult.profiles,
        activeTargets: activeRegistrations + cloudResult.activeTargets.length,
        temporaryFiles: physicalResult.temporaryFiles, retainedEvidenceLeakCount,
        evidence: { source: evidence.source, enumerationComplete: true, enumeratedFileCount: evidence.enumeratedFileCount },
        resources: physicalResult.resources,
        machineOwnership: ownership.kind, physical: physicalResult, retirement,
        relay: { activeRegistrations, queryStartedAtMs: relay.query_started_at_ms, queryCompletedAtMs: relay.query_completed_at_ms },
        historicalRows: { targets: cloudResult.retainedTargetRows.length, heartbeats: cloudResult.retainedHeartbeatRows.length },
        cloudRetirementProof: cloudResult.retirementProof ?? null,
      }
    },
    async close() { await Promise.allSettled([home.close(), cloud.close?.()]) },
  }
}

function validateCloudInventory(value, scope, actor) {
  if (value?.authority !== "read-only-cloud-inventory" || value.complete !== true) throw new Error("complete independent Cloud inventory required")
  for (const field of ["queriedTargetIds", "queriedHeartbeatIds",
    "activeTargets", "retainedTargetRows", "retainedHeartbeatRows"]) {
    if (!Array.isArray(value[field])) throw new Error(`Cloud inventory omitted ${field}`)
  }
  if (value.runId !== scope.runId || value.machineId !== scope.binding.machineId) throw new Error("Cloud census scope mismatch")
  for (const [queried, owned] of [["queriedTargetIds", "ownedTargetIds"], ["queriedHeartbeatIds", "ownedHeartbeatIds"]]) {
    if (JSON.stringify([...new Set(value[queried])].sort()) !== JSON.stringify([...new Set(scope[owned])].sort())) {
      throw new Error("Cloud census did not query every retained identity")
    }
  }
  if (scope.machineOwnership.kind === "run_owned" || value.retainedTargetRows.length || value.retainedHeartbeatRows.length) {
    validateRetiredHistory(value, scope, actor)
  }
}

function validateRetiredHistory(value, scope, actor) {
  const fail = () => { throw new Error("retained Cloud history lacks exact normal managed DELETE retirement proof") }
  const receipt = scope.machineOwnership.creationReceipt
  const proof = value.retirementProof
  const timestamp = input => typeof input === "string" && Number.isFinite(Date.parse(input))
  if (scope.machineOwnership.kind !== "run_owned" || !actor?.accountId || !actor.realmId || !actor.userId
    || proof?.authority !== "normal-managed-delete" || proof.accountId !== actor.accountId
    || proof.realmId !== actor.realmId || proof.userId !== actor.userId
    || proof.environmentId !== receipt.managedEnvironmentId || proof.machineId !== scope.binding.machineId
    || proof.kernelId !== scope.binding.kernelId || proof.createOperationId !== receipt.createOperationId) fail()
  const operation = proof.operation
  const environment = proof.environment
  const machine = proof.machine
  if (!operation?.id || operation.accountId !== actor.accountId || operation.environmentId !== receipt.managedEnvironmentId
    || operation.requestedByUserId !== actor.userId || operation.kind !== "DELETE" || operation.status !== "SUCCEEDED"
    || operation.idempotencyKey !== `parity-cleanup-${createHash("sha256").update(scope.runId).digest("hex")}`
    || !timestamp(operation.completedAt) || !Number.isSafeInteger(operation.desiredRevision) || operation.desiredRevision < 1
    || environment?.id !== receipt.managedEnvironmentId || environment.accountId !== actor.accountId
    || environment.runtimeMachineId !== scope.binding.machineId || environment.runtimeRelayRealmId !== actor.realmId
    || environment.desiredState !== "DELETED" || environment.observedState !== "DELETED"
    || environment.desiredRevision !== operation.desiredRevision || environment.observedRevision !== operation.desiredRevision
    || machine?.accountId !== actor.accountId || machine.machineId !== scope.binding.machineId
    || machine.status !== "REVOKED" || machine.acceptingLeases !== false || !timestamp(machine.revokedAt)
    || proof.activeCredentials !== 0 || proof.unrevokedTokens !== 0 || proof.unrevokedGrants !== 0 || proof.nonRevokedTargets !== 0
    || value.activeTargets.length !== 0 || !Array.isArray(proof.tombstones)) fail()
  const kernels = new Set([...scope.ownedKernelIds, scope.binding.kernelId])
  for (const [kind, id] of [["MACHINE", scope.binding.machineId], ...[...kernels].map(id => ["KERNEL", id])]) {
    const rows = proof.tombstones.filter(row => row.accountId === actor.accountId && row.subjectKind === kind && row.subject === id
      && row.reason === "managed_environment_deleted")
    if (rows.length !== 1) fail()
  }
  const targets = new Map()
  for (const row of value.retainedTargetRows) {
    if (targets.has(row.id) || !scope.ownedTargetIds.includes(row.id) || row.accountId !== actor.accountId
      || row.realmId !== actor.realmId || row.machineId !== scope.binding.machineId || !kernels.has(row.daemonId)
      || row.status !== "REVOKED" || (row.lastHeartbeatAt !== null && !timestamp(row.lastHeartbeatAt))) fail()
    targets.set(row.id, row)
  }
  const heartbeats = new Set()
  for (const row of value.retainedHeartbeatRows) {
    const target = targets.get(row.id)
    if (heartbeats.has(row.id) || !scope.ownedHeartbeatIds.includes(row.id) || !target
      || row.kernelId !== target.daemonId || !timestamp(row.lastHeartbeatAt) || row.lastHeartbeatAt !== target.lastHeartbeatAt) fail()
    heartbeats.add(row.id)
  }
  for (const row of targets.values()) if (row.lastHeartbeatAt !== null && !heartbeats.has(row.id)) fail()
}

// Receipts choose ownership. Host observations establish physical identities.
// Neither an empty product list nor a missing connection proves physical absence.
export function createManagedParityPhysicalLedger({ host, observe = observeManagedParityHost }) {
  let baseline = null
  let failure = null
  let last = null
  const resources = new Map()
  const containers = new Set()
  const volumes = new Set()
  const processes = new Map()
  const profiles = new Map()
  const paths = new Map()
  const rootKey = (value) => `${value.path}:${value.device}:${value.inode}`
  const processKey = (value) => `${value.pid}:${value.startTicks}:${value.pidNamespace}:${value.netNamespace}`
  const retained = () => ({
    containerIds: [...containers], volumeNames: [...volumes],
    processes: [...processes.values()], paths: [...paths.values()],
    profiles: [...profiles.values()],
  })
  const census = async (signal) => {
    try {
      const snapshot = await observe({ ...host, resources: [...resources.values()], retained: retained(), signal })
      validateObservation(snapshot)
      if (baseline) {
        if (snapshot.bootId !== baseline.bootId || snapshot.engineId !== baseline.engineId
          || snapshot.mountNamespace !== baseline.mountNamespace) throw new Error("managed parity host identity changed")
        if (JSON.stringify(snapshot.mounts.map(mountIdentity)) !== JSON.stringify(baseline.mounts.map(mountIdentity))) {
          throw new Error("managed parity observation mount identity changed")
        }
        for (const item of snapshot.containers) {
          if (baseline.hostContainerIds.includes(item.id)) throw new Error("managed parity receipt claimed a preexisting container")
          containers.add(item.id)
        }
        for (const item of snapshot.volumes) {
          if (baseline.hostVolumeNames.includes(item.name)) throw new Error("managed parity receipt claimed a preexisting volume")
          volumes.add(item.name)
        }
        for (const item of snapshot.processes) processes.set(processKey(item), item)
        for (const item of snapshot.profiles) profiles.set(rootKey(item), item)
        const baselinePaths = new Set(baseline.paths.map(rootKey))
        for (const item of snapshot.paths) if (!baselinePaths.has(rootKey(item))) paths.set(rootKey(item), item)
      }
      last = snapshot
      return snapshot
    } catch (error) {
      failure ??= error
      throw error
    }
  }
  return {
    async begin({ signal } = {}) {
      if (baseline) throw new Error("managed parity physical observation already began")
      if (!Array.isArray(host?.paths) || host.paths.length === 0) throw new Error("explicit physical census roots required")
      baseline = await census(signal)
      return { bootId: baseline.bootId, engineId: baseline.engineId, mounts: baseline.mounts.map(mountIdentity) }
    },
    async observeCreated(receipt, { signal } = {}) {
      if (!baseline) throw new Error("managed parity physical baseline is required")
      if (failure) throw failure
      if (receipt.slice) {
        const slice = receipt.slice
        for (const field of ["id", "owner_kernel_id", "owner_machine_id"]) {
          if (typeof slice[field] !== "string" || !slice[field]) throw new Error("actual slice ownership receipt required")
        }
        if (receipt.sliceId !== slice.id) throw new Error("slice receipt identity mismatch")
        const owner = { sliceId: slice.id, ownerKernelId: slice.owner_kernel_id, ownerMachineId: slice.owner_machine_id }
        const previous = resources.get(slice.id)
        if (previous && JSON.stringify(previous) !== JSON.stringify(owner)) throw new Error("slice ownership changed")
        resources.set(slice.id, owner)
      }
      const snapshot = await census(signal)
      if (["slice.start", "slice.restore"].includes(receipt.kind) && snapshot.profiles.length === 0) {
        failure ??= new Error("started browser profile physical coverage is missing")
        throw failure
      }
      return snapshot
    },
    async beforeRetire(_receipt, { signal } = {}) {
      if (!baseline) throw new Error("managed parity physical baseline is required")
      return census(signal)
    },
    async inspect({ signal } = {}) {
      if (!baseline) throw new Error("managed parity physical baseline is required")
      const snapshot = await census(signal)
      if (failure) throw failure
      const processIds = new Set([...snapshot.processes, ...snapshot.retainedProcesses].map(processKey))
      const containerIds = new Set([...snapshot.containers.map(({ id }) => id), ...snapshot.retainedContainers])
      const volumeNames = new Set([...snapshot.volumes.map(({ name }) => name), ...snapshot.retainedVolumes])
      const baselinePaths = new Set(baseline.paths.map(rootKey))
      const residualPaths = new Set([
        ...snapshot.paths.filter((item) => !baselinePaths.has(rootKey(item))).map(rootKey),
        ...snapshot.retainedPaths.map(rootKey),
      ])
      const measuredDevices = new Set()
      const diskDeltaBytes = snapshot.mounts.reduce((sum, item, index) => {
        if (measuredDevices.has(item.mountDevice)) return sum
        measuredDevices.add(item.mountDevice)
        return sum + baseline.mounts[index].freeBytes - item.freeBytes
      }, 0)
      return {
        capturedAt: new Date().toISOString(), bootId: snapshot.bootId, engineId: snapshot.engineId,
        mounts: snapshot.mounts.map(mountIdentity),
        containers: containerIds.size, volumes: volumeNames.size,
        profiles: new Set([...snapshot.profiles, ...snapshot.retainedProfiles].map(rootKey)).size,
        ownedSliceIds: [...resources.keys()],
        processes: processIds.size, listeners: snapshot.listeners.length, temporaryFiles: residualPaths.size,
        resources: {
          rssDeltaBytes: snapshot.hostRssBytes - baseline.hostRssBytes,
          diskDeltaBytes,
        },
        // Full receipts remain available to the separately reviewed inspector;
        // do not reduce retained generations to the latest slice identity.
        retained: retained(),
      }
    },
    lastObservation() { return last ? structuredClone(last) : null },
  }
}

function mountIdentity(item) {
  return { path: item.path, device: item.device, inode: item.inode, mountId: item.mountId,
    parentMountId: item.parentMountId, mountDevice: item.mountDevice, mountPoint: item.mountPoint }
}

function validateObservation(value) {
  if (value?.schema !== "chariox.managed_parity.host_observation.v1"
    || !value.bootId || !value.engineId || !value.mountNamespace || !Number.isFinite(value.hostRssBytes)) {
    throw new Error("incomplete physical host observation")
  }
  for (const field of ["containers", "volumes", "processes", "profiles", "retainedProfiles", "listeners", "paths", "mounts", "hostContainerIds", "hostVolumeNames",
    "retainedContainers", "retainedVolumes", "retainedProcesses", "retainedPaths"]) {
    if (!Array.isArray(value[field])) throw new Error(`physical observation omitted ${field}`)
  }
  if (!value.mounts.length || value.mounts.some((mount) => !mount.mountId || !mount.inode
    || !Number.isFinite(mount.freeBytes))) throw new Error("physical observation omitted mount identity or telemetry")
}
