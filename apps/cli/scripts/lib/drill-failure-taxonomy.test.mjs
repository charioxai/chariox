import assert from "node:assert/strict"
import test from "node:test"

import {
  DRILL_FAILURE_CLASSIFICATION_KINDS,
  drillFailureClassificationForKind,
  drillFailureNextActionForClassification,
  drillFailureTaxonomyManifest,
  drillFailureOwnerForClassification,
  isKnownDrillFailureClassification,
  validateDrillFailureClassification,
  validateDrillFailureTaxonomyManifest,
} from "./drill-failure-taxonomy.mjs"

test("maps classifications to owners", () => {
  assert.equal(drillFailureOwnerForClassification("provider-auth"), "provider-account")
  assert.equal(drillFailureOwnerForClassification("cloud-runtime"), "cloud-deployment")
  assert.equal(drillFailureOwnerForClassification("relay-runtime"), "runtime-network")
  assert.equal(drillFailureOwnerForClassification("relay-target-freshness"), "runtime-network")
  assert.equal(drillFailureOwnerForClassification("remote-worker-version"), "worker-kernel")
  assert.equal(drillFailureOwnerForClassification("remote-host-capacity"), "remote-machine")
  assert.equal(drillFailureOwnerForClassification("runtime-timeout"), "runtime-state")
  assert.equal(drillFailureOwnerForClassification("kernel-authority"), "kernel-authority")
  assert.equal(drillFailureOwnerForClassification("remote-extension-sync"), "kernel-authority")
  assert.equal(drillFailureOwnerForClassification("projection-staleness"), "kernel-authority")
  assert.equal(drillFailureOwnerForClassification("runtime-projection-health"), "kernel-authority")
  assert.equal(drillFailureOwnerForClassification("worker-execution"), "worker-kernel")
  assert.equal(drillFailureOwnerForClassification("ui-client-projection"), "ui-client")
  assert.equal(drillFailureOwnerForClassification("workspace-live-sync-conflict"), "runtime-state")
  assert.equal(drillFailureOwnerForClassification("slice-auth"), "provider-account")
  assert.equal(drillFailureOwnerForClassification("slice-runtime"), "worker-kernel")
  assert.equal(drillFailureOwnerForClassification("test-harness"), "validation-harness")
  assert.equal(drillFailureOwnerForClassification("unknown"), "drill-or-runtime")
  assert.equal(drillFailureOwnerForClassification("unknown", { fallback: "validation-harness" }), "validation-harness")
})

test("exposes known classifications", () => {
  assert(DRILL_FAILURE_CLASSIFICATION_KINDS.includes("kernel-authority"))
  assert(DRILL_FAILURE_CLASSIFICATION_KINDS.includes("relay-target-freshness"))
  assert(DRILL_FAILURE_CLASSIFICATION_KINDS.includes("remote-worker-version"))
  assert(DRILL_FAILURE_CLASSIFICATION_KINDS.includes("remote-host-capacity"))
  assert(DRILL_FAILURE_CLASSIFICATION_KINDS.includes("projection-staleness"))
  assert(DRILL_FAILURE_CLASSIFICATION_KINDS.includes("runtime-projection-health"))
  assert(DRILL_FAILURE_CLASSIFICATION_KINDS.includes("slice-runtime"))
  assert.deepEqual(DRILL_FAILURE_CLASSIFICATION_KINDS, [...DRILL_FAILURE_CLASSIFICATION_KINDS].sort())
  assert.equal(isKnownDrillFailureClassification("slice-runtime"), true)
  assert.equal(isKnownDrillFailureClassification("not-real"), false)
  assert.doesNotThrow(() => validateDrillFailureClassification("slice-runtime", "scenario"))
  assert.throws(
    () => validateDrillFailureClassification("not-real", "scenario"),
    /scenario has unknown classification "not-real"/,
  )
  assert.throws(
    () => validateDrillFailureClassification("not-real", "preset.requiredFailureClassifications[0]", {
      label: "failure classification",
    }),
    /preset\.requiredFailureClassifications\[0\] has unknown failure classification "not-real"/,
  )
  assert.throws(
    () => validateDrillFailureClassification("not-real", "required failure classifications", {
      message: (classification) => `unknown required failure classification: ${classification}`,
    }),
    /unknown required failure classification: not-real/,
  )
})

test("formats target-specific next actions", () => {
  assert.equal(
    drillFailureNextActionForClassification("provider-auth", { target: "drill" }),
    "refresh provider login for the profile used by this drill, then rerun the drill",
  )
  assert.equal(
    drillFailureNextActionForClassification("provider-auth", { target: "scenario" }),
    "refresh provider login for the profile used by this drill, then rerun the scenario",
  )
  assert.equal(
    drillFailureNextActionForClassification("unknown", { target: "drill", rootDir: "/tmp/arroba-drill" }),
    "inspect preserved artifacts under /tmp/arroba-drill; rerun the drill after addressing the failure",
  )
  assert.equal(
    drillFailureNextActionForClassification("workspace-live-sync-conflict", { target: "scenario" }),
    "inspect workspace live sync status, conflicts, and preserved file snapshots; reconcile the conflict, then rerun the scenario",
  )
  assert.equal(
    drillFailureNextActionForClassification("worker-execution", { target: "scenario" }),
    "inspect worker kernel logs, leased-agent launch state, and preserved worker artifacts, then rerun the scenario",
  )
  assert.equal(
    drillFailureNextActionForClassification("projection-staleness", { target: "scenario" }),
    "inspect kernel projection health, read-model freshness, and reconciliation events before rerunning the scenario",
  )
  assert.equal(
    drillFailureNextActionForClassification("runtime-projection-health", { target: "scenario" }),
    "inspect kernel projection health, read-model freshness, and reconciliation events before rerunning the scenario",
  )
  assert.equal(
    drillFailureNextActionForClassification("ui-client-projection", { target: "scenario" }),
    "inspect web/TUI terminal projection logs, transcript rendering state, and preserved screenshots or terminal captures, then rerun the scenario",
  )
  assert.equal(
    drillFailureNextActionForClassification("relay-target-freshness", { target: "scenario" }),
    "inspect relay target heartbeat freshness, selected kernel id/alias, and kernel presence logs, then rerun the scenario",
  )
  assert.equal(
    drillFailureNextActionForClassification("remote-worker-version", { target: "scenario" }),
    "upgrade/rebuild the remote worker checkout, restart the worker kernel, verify relay peer protocol compatibility, then rerun the scenario",
  )
  assert.equal(
    drillFailureNextActionForClassification("remote-host-capacity", { target: "scenario" }),
    "free disk on the remote host or choose a clean worker checkout/artifact root, then rerun the scenario",
  )
})

test("builds manifest classification records from taxonomy", () => {
  assert.deepEqual(drillFailureClassificationForKind("docker-runtime", { target: "drill" }), {
    kind: "docker-runtime",
    owner: "local-machine",
    nextAction: "start Docker or Colima, confirm `docker info` succeeds, then rerun the drill",
  })
})

test("builds stable failure taxonomy manifest", () => {
  const manifest = drillFailureTaxonomyManifest({ target: "scenario" })

  assert.equal(manifest.schema, "arroba.drill.failure_taxonomy.v1")
  assert.equal(manifest.target, "scenario")
  assert.deepEqual(
    manifest.classifications.map((entry) => entry.kind),
    [...manifest.classifications.map((entry) => entry.kind)].sort(),
  )
  assert(manifest.classifications.some((entry) => (
    entry.kind === "kernel-authority"
      && entry.owner === "kernel-authority"
      && entry.nextAction.includes("session, agent, lease")
  )))
  assert(manifest.classifications.some((entry) => (
    entry.kind === "relay-target-freshness"
      && entry.owner === "runtime-network"
      && entry.nextAction.includes("heartbeat freshness")
  )))
  assert(manifest.classifications.some((entry) => (
    entry.kind === "remote-worker-version"
      && entry.owner === "worker-kernel"
      && entry.nextAction.includes("relay peer protocol compatibility")
  )))
  assert(manifest.classifications.some((entry) => (
    entry.kind === "remote-host-capacity"
      && entry.owner === "remote-machine"
      && entry.nextAction.includes("free disk")
  )))
  assert(manifest.classifications.some((entry) => (
    entry.kind === "remote-extension-sync"
      && entry.owner === "kernel-authority"
      && entry.nextAction.includes("manifest sync")
  )))
  assert(manifest.classifications.some((entry) => (
    entry.kind === "projection-staleness"
      && entry.owner === "kernel-authority"
      && entry.nextAction.includes("projection health")
  )))
  assert(manifest.classifications.some((entry) => (
    entry.kind === "runtime-projection-health"
      && entry.owner === "kernel-authority"
      && entry.nextAction.includes("projection health")
  )))
  assert(manifest.classifications.some((entry) => (
    entry.kind === "workspace-live-sync-conflict"
      && entry.owner === "runtime-state"
      && entry.nextAction.includes("workspace live sync")
  )))
  assert(manifest.classifications.some((entry) => (
    entry.kind === "slice-runtime"
      && entry.owner === "worker-kernel"
      && entry.nextAction.includes("slice lifecycle events")
  )))
  assert.doesNotThrow(() => validateDrillFailureTaxonomyManifest(manifest))
  assert.doesNotThrow(() => validateDrillFailureTaxonomyManifest(
    drillFailureTaxonomyManifest({ target: "drill" }),
    "drill target manifest",
    { target: "drill" },
  ))
  assert.throws(
    () => validateDrillFailureTaxonomyManifest({ ...manifest, target: "drill" }),
    /has invalid target "drill"/,
  )
  assert.throws(
    () => validateDrillFailureTaxonomyManifest({
      ...manifest,
      classifications: manifest.classifications.filter((entry) => entry.kind !== "workspace-live-sync-conflict"),
    }),
    /classifications do not match drill failure taxonomy/,
  )
  assert.throws(
    () => validateDrillFailureTaxonomyManifest({
      ...manifest,
      classifications: manifest.classifications.map((entry) => entry.kind === "kernel-authority"
        ? { ...entry, owner: "runtime-state" }
        : entry),
    }),
    /classifications\[\d+\] has invalid owner/,
  )
})
