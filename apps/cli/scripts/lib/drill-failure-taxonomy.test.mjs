import assert from "node:assert/strict"
import test from "node:test"

import {
  drillFailureClassificationForKind,
  drillFailureNextActionForClassification,
  drillFailureOwnerForClassification,
} from "./drill-failure-taxonomy.mjs"

test("maps classifications to owners", () => {
  assert.equal(drillFailureOwnerForClassification("provider-auth"), "provider-account")
  assert.equal(drillFailureOwnerForClassification("cloud-runtime"), "cloud-deployment")
  assert.equal(drillFailureOwnerForClassification("relay-runtime"), "runtime-network")
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
})

test("builds manifest classification records from taxonomy", () => {
  assert.deepEqual(drillFailureClassificationForKind("docker-runtime", { target: "drill" }), {
    kind: "docker-runtime",
    owner: "local-machine",
    nextAction: "start Docker or Colima, confirm `docker info` succeeds, then rerun the drill",
  })
})
