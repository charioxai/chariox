import assert from "node:assert/strict"
import test from "node:test"

import {
  drillFailureClassificationForKind,
  drillFailureNextActionForClassification,
  drillFailureTaxonomyManifest,
  drillFailureOwnerForClassification,
} from "./drill-failure-taxonomy.mjs"

test("maps classifications to owners", () => {
  assert.equal(drillFailureOwnerForClassification("provider-auth"), "provider-account")
  assert.equal(drillFailureOwnerForClassification("cloud-runtime"), "cloud-deployment")
  assert.equal(drillFailureOwnerForClassification("relay-runtime"), "runtime-network")
  assert.equal(drillFailureOwnerForClassification("runtime-timeout"), "runtime-state")
  assert.equal(drillFailureOwnerForClassification("kernel-authority"), "kernel-authority")
  assert.equal(drillFailureOwnerForClassification("remote-extension-sync"), "kernel-authority")
  assert.equal(drillFailureOwnerForClassification("worker-execution"), "worker-kernel")
  assert.equal(drillFailureOwnerForClassification("ui-client-projection"), "ui-client")
  assert.equal(drillFailureOwnerForClassification("workspace-live-sync-conflict"), "runtime-state")
  assert.equal(drillFailureOwnerForClassification("slice-auth"), "provider-account")
  assert.equal(drillFailureOwnerForClassification("test-harness"), "validation-harness")
  assert.equal(drillFailureOwnerForClassification("unknown"), "drill-or-runtime")
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
    drillFailureNextActionForClassification("ui-client-projection", { target: "scenario" }),
    "inspect web/TUI terminal projection logs, transcript rendering state, and preserved screenshots or terminal captures, then rerun the scenario",
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
    entry.kind === "remote-extension-sync"
      && entry.owner === "kernel-authority"
      && entry.nextAction.includes("manifest sync")
  )))
  assert(manifest.classifications.some((entry) => (
    entry.kind === "workspace-live-sync-conflict"
      && entry.owner === "runtime-state"
      && entry.nextAction.includes("workspace live sync")
  )))
})
