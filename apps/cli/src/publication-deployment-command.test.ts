import assert from "node:assert/strict"
import test from "node:test"

import {
  formatPublicationDeploymentDeployOutput,
  formatPublicationDeploymentListRow,
  formatPublicationDeploymentSummary,
} from "./publication-deployment-command.js"
import type { PublicationDeploymentSummary } from "./publication-deployment-api.js"

test("publication deployment CLI list row includes health, queue, replicas, and errors", () => {
  assert.equal(
    formatPublicationDeploymentListRow(deployment()),
    [
      "deployment-1",
      "hosted_container",
      "failed",
      "unhealthy",
      "human_http",
      "1 ready/2 active",
      "3",
      "https://publications.example.test/shop/",
      "credential_profile_missing: credential profile missing",
    ].join("\t"),
  )
})

test("publication deployment CLI summary exposes operator fields", () => {
  assert.deepEqual(formatPublicationDeploymentSummary(deployment()), [
    "deployment deployment-1",
    "mode hosted_container",
    "status failed",
    "health unhealthy",
    "transport human_http",
    "url https://publications.example.test/shop/",
    "credential_profile miguel_staging",
    "replicas 1 ready/2 active",
    "queue 3",
    "last_error_code credential_profile_missing",
    "last_error credential profile missing",
  ])
})

test("publication deployment CLI deploy output makes public unmanaged access explicit", () => {
  const output = formatPublicationDeploymentDeployOutput(deployment({ mode: "local_runtime" }), {
    packagePath: "/tmp/publication",
    includeServeInstruction: true,
  })

  assert.match(output, /warning public_unmanaged_access anyone with the generated URL can invoke this deployment/)
  assert.match(output, /serve arroba serve \/tmp\/publication <port> --cloud-deployment deployment-1/)
})

function deployment(overrides: Partial<PublicationDeploymentSummary> = {}): PublicationDeploymentSummary {
  return {
    id: "deployment-1",
    mode: "hosted_container",
    slug: "shop",
    publicBaseUrl: "https://publications.example.test/shop/",
    status: "failed",
    publicationId: "pub-1",
    transport: "human_http",
    credentialProfile: "miguel_staging",
    health: "unhealthy",
    queueDepth: 3,
    activeReplicaCount: 2,
    readyReplicaCount: 1,
    lastErrorCode: "credential_profile_missing",
    lastError: "credential profile missing",
    ...overrides,
  }
}
