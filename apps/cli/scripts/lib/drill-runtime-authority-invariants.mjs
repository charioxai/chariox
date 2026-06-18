import {
  validateDrillRuntimeSignal,
  validateDrillRuntimeSignals,
} from "./drill-runtime-signals.mjs"

export const DRILL_RUNTIME_AUTHORITY_INVARIANTS_SCHEMA = "arroba.drill.runtime_authority_invariants.v1"

const RUNTIME_AUTHORITY_INVARIANTS = Object.freeze({
  "client-render-request": {
    owner: "ui-client",
    description: "Clients render kernel projections and submit typed requests; they do not synthesize session, agent, provider-run, permission, history, or health state.",
    requiredRuntimeSignals: Object.freeze(["client-projection-health", "session-authority"]),
  },
  "home-session-authority": {
    owner: "kernel-authority",
    description: "The home kernel owns sessions, prompts, attachments, transcript history, runtime interactions, Workspace Live Sync policy, extension grants, and remote-agent leases.",
    requiredRuntimeSignals: Object.freeze(["home-extension-manifest-sync", "lease-health", "permission-interaction", "session-authority", "workspace-live-sync-state"]),
  },
  "projected-state-diagnostics": {
    owner: "kernel-authority",
    description: "Projected remote state with authority implications has kernel-owned health or audit projection and validation-platform runtime-signal coverage.",
    requiredRuntimeSignals: Object.freeze(["home-extension-manifest-sync", "lease-health", "provider-run-lifecycle", "relay-target-freshness", "runtime-projection-health", "slice-auth-state", "slice-runtime-state", "workspace-live-sync-state"]),
  },
  "relay-cloud-transport-only": {
    owner: "runtime-network",
    description: "Relay and Cloud remain bootstrap, control-plane, or transport surfaces; neither inspects or mutates runtime prompts, provider payloads, workspace files, extension credentials, or session history.",
    requiredRuntimeSignals: Object.freeze(["relay-target-freshness", "session-authority"]),
  },
  "shared-runtime-primitives": {
    owner: "kernel-authority",
    description: "Provider-native TUIs, web terminals, local TUIs, remote TUIs, and slice-backed agents enter through the same kernel-owned prompt, permission, provider-run, and projection primitives.",
    requiredRuntimeSignals: Object.freeze(["client-projection-health", "permission-interaction", "provider-run-lifecycle", "runtime-projection-health", "session-authority"]),
  },
  "worker-execution-authority": {
    owner: "worker-kernel",
    description: "Worker kernels own only hosted execution: provider process lifecycle, worker-local tool execution, slice container/runtime state, and leased-agent transport to home.",
    requiredRuntimeSignals: Object.freeze(["provider-run-lifecycle", "slice-runtime-state", "lease-health"]),
  },
})

export const DRILL_RUNTIME_AUTHORITY_INVARIANT_IDS = Object.freeze(Object.keys(RUNTIME_AUTHORITY_INVARIANTS).sort())
export const DRILL_RUNTIME_AUTHORITY_OWNERS = Object.freeze([
  ...new Set(Object.values(RUNTIME_AUTHORITY_INVARIANTS).map((invariant) => invariant.owner)),
].sort())

export function isKnownDrillRuntimeAuthorityInvariant(invariantId) {
  return typeof invariantId === "string"
    && Object.prototype.hasOwnProperty.call(RUNTIME_AUTHORITY_INVARIANTS, invariantId)
}

export function validateDrillRuntimeAuthorityInvariant(invariantId, source, { label = "runtime authority invariant" } = {}) {
  if (!isKnownDrillRuntimeAuthorityInvariant(invariantId)) {
    throw new Error(`${source} has unknown ${label} ${JSON.stringify(invariantId)}`)
  }
}

export function drillRuntimeAuthorityInvariantOwner(invariantId) {
  validateDrillRuntimeAuthorityInvariant(invariantId, "drill runtime authority invariant", { label: "id" })
  return RUNTIME_AUTHORITY_INVARIANTS[invariantId].owner
}

export function drillRuntimeAuthorityInvariantSignals(invariantId) {
  validateDrillRuntimeAuthorityInvariant(invariantId, "drill runtime authority invariant", { label: "id" })
  return [...RUNTIME_AUTHORITY_INVARIANTS[invariantId].requiredRuntimeSignals]
}

export function drillRuntimeAuthorityManifest() {
  return {
    schema: DRILL_RUNTIME_AUTHORITY_INVARIANTS_SCHEMA,
    invariants: DRILL_RUNTIME_AUTHORITY_INVARIANT_IDS.map((id) => ({
      id,
      owner: RUNTIME_AUTHORITY_INVARIANTS[id].owner,
      description: RUNTIME_AUTHORITY_INVARIANTS[id].description,
      requiredRuntimeSignals: drillRuntimeAuthorityInvariantSignals(id),
    })),
  }
}

export function validateDrillRuntimeAuthorityManifest(manifest, source = "runtime authority manifest") {
  if (!manifest || typeof manifest !== "object" || Array.isArray(manifest)) {
    throw new Error(`${source} is not an object`)
  }
  if (manifest.schema !== DRILL_RUNTIME_AUTHORITY_INVARIANTS_SCHEMA) {
    throw new Error(`${source} has unsupported schema ${JSON.stringify(manifest.schema)}`)
  }
  if (!Array.isArray(manifest.invariants)) {
    throw new Error(`${source} has invalid invariants`)
  }
  const seen = new Set()
  const ids = []
  for (const [index, invariant] of manifest.invariants.entries()) {
    const invariantSource = `${source}.invariants[${index}]`
    if (!invariant || typeof invariant !== "object" || Array.isArray(invariant)) {
      throw new Error(`${invariantSource} is not an object`)
    }
    validateDrillRuntimeAuthorityInvariant(invariant.id, invariantSource, { label: "id" })
    if (seen.has(invariant.id)) {
      throw new Error(`${source} has duplicate invariant ${invariant.id}`)
    }
    seen.add(invariant.id)
    ids.push(invariant.id)
    const expected = RUNTIME_AUTHORITY_INVARIANTS[invariant.id]
    if (invariant.owner !== expected.owner) {
      throw new Error(`${invariantSource} has invalid owner`)
    }
    if (typeof invariant.description !== "string" || invariant.description.trim().length === 0) {
      throw new Error(`${invariantSource} has invalid description`)
    }
    validateDrillRuntimeSignals(invariant.requiredRuntimeSignals, `${invariantSource}.requiredRuntimeSignals`)
    if (JSON.stringify([...invariant.requiredRuntimeSignals].sort()) !== JSON.stringify([...expected.requiredRuntimeSignals].sort())) {
      throw new Error(`${invariantSource}.requiredRuntimeSignals do not match required runtime signals`)
    }
  }
  if (JSON.stringify(ids.sort()) !== JSON.stringify(DRILL_RUNTIME_AUTHORITY_INVARIANT_IDS)) {
    throw new Error(`${source} does not match required runtime authority invariants`)
  }
}
