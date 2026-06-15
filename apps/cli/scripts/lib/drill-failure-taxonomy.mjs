const FAILURE_CLASSIFICATIONS = {
  "provider-auth": {
    owner: "provider-account",
    drillNextAction: "refresh provider login for the profile used by this drill, then rerun the drill",
    scenarioNextAction: "refresh provider login for the profile used by this drill, then rerun the scenario",
  },
  "provider-account": {
    owner: "provider-account",
    drillNextAction: "check provider quota or billing for the account used by this drill, then rerun the drill",
    scenarioNextAction: "check provider quota or billing for the account used by this drill, then rerun the scenario",
  },
  "provider-error": {
    owner: "provider-runtime",
    drillNextAction: "inspect provider runtime logs in the preserved artifact root, then rerun the drill",
    scenarioNextAction: "inspect provider runtime logs in the preserved artifacts, then rerun the scenario",
  },
  "docker-runtime": {
    owner: "local-machine",
    drillNextAction: "start Docker or Colima, confirm `docker info` succeeds, then rerun the drill",
    scenarioNextAction: "start Docker or Colima, confirm `docker info` succeeds, then rerun the scenario",
  },
  "cloud-runtime": {
    owner: "cloud-deployment",
    drillNextAction: "inspect Cloud deployment/control-plane status and preserved logs, then rerun the drill",
    scenarioNextAction: "inspect Cloud deployment/control-plane status and preserved logs, then rerun the scenario",
  },
  "relay-runtime": {
    owner: "runtime-network",
    drillNextAction: "inspect relay and kernel logs in the preserved artifact root, then rerun the drill",
    scenarioNextAction: "inspect relay and kernel logs in the preserved artifacts, then rerun the scenario",
  },
  "relay-target-freshness": {
    owner: "runtime-network",
    drillNextAction: "inspect relay target heartbeat freshness, selected kernel id/alias, and kernel presence logs, then rerun the drill",
    scenarioNextAction: "inspect relay target heartbeat freshness, selected kernel id/alias, and kernel presence logs, then rerun the scenario",
  },
  "remote-worker-version": {
    owner: "worker-kernel",
    drillNextAction: "upgrade/rebuild the remote worker checkout, restart the worker kernel, verify relay peer protocol compatibility, then rerun the drill",
    scenarioNextAction: "upgrade/rebuild the remote worker checkout, restart the worker kernel, verify relay peer protocol compatibility, then rerun the scenario",
  },
  "remote-host-capacity": {
    owner: "remote-machine",
    drillNextAction: "free disk on the remote host or choose a clean worker checkout/artifact root, then rerun the drill",
    scenarioNextAction: "free disk on the remote host or choose a clean worker checkout/artifact root, then rerun the scenario",
  },
  "runtime-timeout": {
    owner: "runtime-state",
    drillNextAction: "inspect preserved runtime state, provider run lifecycle, and drill timeout diagnostics, then rerun the drill",
    scenarioNextAction: "inspect preserved runtime state, provider run lifecycle, and drill timeout diagnostics, then rerun the scenario",
  },
  "kernel-authority": {
    owner: "kernel-authority",
    drillNextAction: "inspect session, agent, lease, provider-run, and projection authority state before rerunning the drill",
    scenarioNextAction: "inspect session, agent, lease, provider-run, and projection authority state before rerunning the scenario",
  },
  "remote-extension-sync": {
    owner: "kernel-authority",
    drillNextAction: "inspect remote extension manifest sync status and audit events, retry sync if the grant is still valid, then rerun the drill",
    scenarioNextAction: "inspect remote extension manifest sync status and audit events, retry sync if the grant is still valid, then rerun the scenario",
  },
  "projection-staleness": {
    owner: "kernel-authority",
    drillNextAction: "inspect kernel projection health, read-model freshness, and reconciliation events before rerunning the drill",
    scenarioNextAction: "inspect kernel projection health, read-model freshness, and reconciliation events before rerunning the scenario",
  },
  "worker-execution": {
    owner: "worker-kernel",
    drillNextAction: "inspect worker kernel logs, leased-agent launch state, and preserved worker artifacts, then rerun the drill",
    scenarioNextAction: "inspect worker kernel logs, leased-agent launch state, and preserved worker artifacts, then rerun the scenario",
  },
  "ui-client-projection": {
    owner: "ui-client",
    drillNextAction: "inspect web/TUI terminal projection logs, transcript rendering state, and preserved screenshots or terminal captures, then rerun the drill",
    scenarioNextAction: "inspect web/TUI terminal projection logs, transcript rendering state, and preserved screenshots or terminal captures, then rerun the scenario",
  },
  "workspace-live-sync-conflict": {
    owner: "runtime-state",
    drillNextAction: "inspect workspace live sync status, conflicts, and preserved file snapshots; reconcile the conflict, then rerun the drill",
    scenarioNextAction: "inspect workspace live sync status, conflicts, and preserved file snapshots; reconcile the conflict, then rerun the scenario",
  },
  "slice-auth": {
    owner: "provider-account",
    drillNextAction: "inspect slice provider auth summaries, login/import the intended account in the slice, then rerun the drill",
    scenarioNextAction: "inspect slice provider auth summaries, login/import the intended account in the slice, then rerun the scenario",
  },
  "slice-runtime": {
    owner: "worker-kernel",
    drillNextAction: "inspect slice lifecycle events, container logs, and worker kernel state; recreate the slice if needed, then rerun the drill",
    scenarioNextAction: "inspect slice lifecycle events, container logs, and worker kernel state; recreate the slice if needed, then rerun the scenario",
  },
  "test-harness": {
    owner: "validation-harness",
    drillNextAction: "install or build the missing local drill prerequisite, then rerun the drill",
    scenarioNextAction: "install or build the missing local drill prerequisite, then rerun the scenario",
  },
  "expected-failure": {
    owner: "validation-harness",
    drillNextAction: "inspect the expected-failure assertion; the drill failed differently than planned",
    scenarioNextAction: "inspect the expected-failure assertion; the scenario failed differently than planned",
  },
  "matrix-coverage": {
    owner: "validation-harness",
    drillNextAction: "run matrix reports for the missing deployment presets, then rerun the validation gate",
    scenarioNextAction: "run the missing deployment preset scenario, then rerun the matrix",
  },
  "child-process": {
    owner: "drill-or-runtime",
    drillNextAction: null,
    scenarioNextAction: "inspect preserved drill artifacts and rerun the command recorded in this report",
  },
}

export const DRILL_FAILURE_CLASSIFICATION_KINDS = Object.freeze(Object.keys(FAILURE_CLASSIFICATIONS).sort())

export function isKnownDrillFailureClassification(classification) {
  return typeof classification === "string"
    && Object.prototype.hasOwnProperty.call(FAILURE_CLASSIFICATIONS, classification)
}

export function drillFailureOwnerForClassification(classification, { fallback = "drill-or-runtime" } = {}) {
  return FAILURE_CLASSIFICATIONS[classification]?.owner ?? fallback
}

export function drillFailureNextActionForClassification(classification, { target = "scenario", rootDir = null } = {}) {
  const details = FAILURE_CLASSIFICATIONS[classification]
  if (target === "drill") {
    return details?.drillNextAction
      ?? `inspect preserved artifacts${rootDir ? ` under ${rootDir}` : ""}; rerun the drill after addressing the failure`
  }
  return details?.scenarioNextAction
    ?? "inspect preserved drill artifacts and rerun the command recorded in this report"
}

export function drillFailureClassificationForKind(kind, options = {}) {
  return {
    kind,
    owner: drillFailureOwnerForClassification(kind),
    nextAction: drillFailureNextActionForClassification(kind, options),
  }
}

export function drillFailureTaxonomyManifest({ target = "scenario" } = {}) {
  return {
    schema: "arroba.drill.failure_taxonomy.v1",
    target,
    classifications: Object.keys(FAILURE_CLASSIFICATIONS).sort().map((kind) => ({
      kind,
      owner: drillFailureOwnerForClassification(kind),
      nextAction: drillFailureNextActionForClassification(kind, { target }),
    })),
  }
}
