export const DRILL_RUNTIME_SIGNALS_SCHEMA = "arroba.drill.runtime_signals.v1"

const RUNTIME_SIGNALS = Object.freeze({
  "agent-lifecycle": {
    owner: "kernel-authority",
    description: "Kernel-owned agent creation, launch, stop, reuse, and terminal lifecycle state.",
  },
  "client-projection-health": {
    owner: "ui-client",
    description: "TUI, web, and native provider TUI projection freshness, transcript parity, and render state.",
  },
  "home-extension-manifest-sync": {
    owner: "kernel-authority",
    description: "Home-owned extension grant, revoke, manifest version, sync status, and stale-invocation denial state.",
  },
  "lease-health": {
    owner: "kernel-authority",
    description: "Remote leased-agent session, worker, reconnect, and provider-run binding health.",
  },
  "permission-interaction": {
    owner: "kernel-authority",
    description: "Runtime interaction ownership and approval visibility across attached clients and provider TUIs.",
  },
  "provider-run-lifecycle": {
    owner: "provider-runtime",
    description: "Provider run identity, launch request, active state, completion, cancellation, and stuck-run diagnostics.",
  },
  "relay-target-freshness": {
    owner: "runtime-network",
    description: "Relay target heartbeat freshness, selected kernel identity, and stale-target rejection.",
  },
  "runtime-projection-health": {
    owner: "kernel-authority",
    description: "Kernel-owned read-model projection freshness, invariant checks, and stale projection reconciliation state.",
  },
  "session-authority": {
    owner: "kernel-authority",
    description: "Kernel-owned session state, prompt routing, attachments, history, and authority boundaries.",
  },
  "slice-auth-state": {
    owner: "provider-account",
    description: "Slice provider account summaries, aliases, credential isolation, and auth readiness state.",
  },
  "slice-runtime-state": {
    owner: "worker-kernel",
    description: "Slice container lifecycle, join/reuse/delete state, headless mode, resource profile, and worker availability.",
  },
  "workspace-live-sync-state": {
    owner: "runtime-state",
    description: "Workspace Live Sync mode, included paths, ignored paths, conflict/reconcile status, and peer propagation state.",
  },
})

export const DRILL_RUNTIME_SIGNAL_IDS = Object.freeze(Object.keys(RUNTIME_SIGNALS).sort())
export const DRILL_RUNTIME_SIGNAL_OWNERS = Object.freeze([
  ...new Set(Object.values(RUNTIME_SIGNALS).map((signal) => signal.owner)),
].sort())

export function isKnownDrillRuntimeSignal(signal) {
  return typeof signal === "string"
    && Object.prototype.hasOwnProperty.call(RUNTIME_SIGNALS, signal)
}

export function drillRuntimeSignalOwner(signal) {
  if (!isKnownDrillRuntimeSignal(signal)) {
    throw new Error(`unknown drill runtime signal ${JSON.stringify(signal)}`)
  }
  return RUNTIME_SIGNALS[signal].owner
}

export function drillRuntimeSignalOwnersFor(runtimeSignals) {
  validateDrillRuntimeSignals(runtimeSignals)
  return [...new Set((runtimeSignals ?? []).map((signal) => drillRuntimeSignalOwner(signal)))].sort()
}

export function drillRuntimeSignalOwnerCounts(runtimeSignals) {
  const counts = new Map()
  for (const [signal, count] of Object.entries(runtimeSignals ?? {})) {
    if (!Number.isSafeInteger(count) || count < 0) {
      throw new Error(`drill runtime signal ${JSON.stringify(signal)} has invalid count`)
    }
    const owner = drillRuntimeSignalOwner(signal)
    counts.set(owner, (counts.get(owner) ?? 0) + count)
  }
  return Object.fromEntries([...counts.entries()].sort(([left], [right]) => left.localeCompare(right)))
}

export function validateDrillRuntimeSignals(runtimeSignals, source = "drill runtime signals") {
  for (const [index, signal] of (runtimeSignals ?? []).entries()) {
    if (!isKnownDrillRuntimeSignal(signal)) {
      throw new Error(`${source}[${index}] has unknown runtime signal ${JSON.stringify(signal)}`)
    }
  }
}

export function drillRuntimeSignalsManifest() {
  return {
    schema: DRILL_RUNTIME_SIGNALS_SCHEMA,
    signals: DRILL_RUNTIME_SIGNAL_IDS.map((id) => ({
      id,
      owner: drillRuntimeSignalOwner(id),
      description: RUNTIME_SIGNALS[id].description,
    })),
  }
}

export function validateDrillRuntimeSignalsManifest(manifest, source = "runtime signals manifest") {
  if (!manifest || typeof manifest !== "object" || Array.isArray(manifest)) {
    throw new Error(`${source} is not an object`)
  }
  if (manifest.schema !== DRILL_RUNTIME_SIGNALS_SCHEMA) {
    throw new Error(`${source} has unsupported schema ${JSON.stringify(manifest.schema)}`)
  }
  if (!Array.isArray(manifest.signals)) {
    throw new Error(`${source} has invalid signals`)
  }
  const seen = new Set()
  const ids = []
  for (const [index, signal] of manifest.signals.entries()) {
    const signalSource = `${source}.signals[${index}]`
    if (!signal || typeof signal !== "object" || Array.isArray(signal)) {
      throw new Error(`${signalSource} is not an object`)
    }
    if (!isKnownDrillRuntimeSignal(signal.id)) {
      throw new Error(`${signalSource} has unknown id ${JSON.stringify(signal.id)}`)
    }
    if (seen.has(signal.id)) {
      throw new Error(`${source} has duplicate signal ${signal.id}`)
    }
    seen.add(signal.id)
    ids.push(signal.id)
    const expectedOwner = drillRuntimeSignalOwner(signal.id)
    if (signal.owner !== expectedOwner) {
      throw new Error(`${signalSource} has invalid owner`)
    }
    if (typeof signal.description !== "string" || signal.description.trim().length === 0) {
      throw new Error(`${signalSource} has invalid description`)
    }
  }
  if (JSON.stringify(ids.sort()) !== JSON.stringify(DRILL_RUNTIME_SIGNAL_IDS)) {
    throw new Error(`${source} does not match required runtime signals`)
  }
}
