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
  "child-process": {
    owner: "drill-or-runtime",
    drillNextAction: null,
    scenarioNextAction: "inspect preserved drill artifacts and rerun the command recorded in this report",
  },
}

export function drillFailureOwnerForClassification(classification) {
  return FAILURE_CLASSIFICATIONS[classification]?.owner ?? "drill-or-runtime"
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
