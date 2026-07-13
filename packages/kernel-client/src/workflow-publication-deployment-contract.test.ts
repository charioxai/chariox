import assert from "node:assert/strict"
import test from "node:test"

import {
  resolveWorkflowPublicationDeploymentContract,
  workflowPublicationDeploymentContractPath,
} from "./workflow-publication-deployment-contract.js"

test("publication deployment contract adapter preserves legacy v1 and v2 packages", () => {
  assert.deepEqual(resolveWorkflowPublicationDeploymentContract({ package_version: 1 }), {
    kind: "legacy_adapter",
    packageVersion: 1,
    contract: null,
  })
  assert.deepEqual(resolveWorkflowPublicationDeploymentContract({ package_version: 2 }), {
    kind: "legacy_adapter",
    packageVersion: 2,
    contract: null,
  })
})

test("publication deployment contract validates v3 provenance", () => {
  const publicationPackage = {
    package_version: 3,
    publication_id: "publication-1",
    source_session_id: "session-1",
    workflow_id: "workflow-1",
    deployment_contract: { path: "deployment-contract.json", schema_version: 1 },
  }
  assert.equal(workflowPublicationDeploymentContractPath(publicationPackage), "deployment-contract.json")
  const resolution = resolveWorkflowPublicationDeploymentContract(publicationPackage, fixture())
  assert.equal(resolution.kind, "native")
  assert.equal(resolution.contract?.source.endpoint_id, "endpoint-1")
})

test("publication deployment contract rejects unsafe paths, mismatches, and secrets", () => {
  assert.throws(
    () => workflowPublicationDeploymentContractPath({
      package_version: 3,
      deployment_contract: { path: "../contract.json", schema_version: 1 },
    }),
    /safe relative/,
  )
  assert.throws(
    () => resolveWorkflowPublicationDeploymentContract({
      package_version: 3,
      publication_id: "other-publication",
      deployment_contract: { path: "deployment-contract.json", schema_version: 1 },
    }, fixture()),
    /publication_id does not match/,
  )
  assert.throws(
    () => resolveWorkflowPublicationDeploymentContract({
      package_version: 3,
      deployment_contract: { path: "deployment-contract.json", schema_version: 1 },
    }, { ...fixture(), token: "must-not-appear" }),
    /forbidden secret payload field token/,
  )
  assert.throws(
    () => workflowPublicationDeploymentContractPath({ package_version: 4 }),
    /unsupported publication package_version 4/,
  )
})

function fixture(): Record<string, unknown> {
  const digest = `sha256:${"a".repeat(64)}`
  return {
    schema_version: 1,
    package_id: digest,
    artifact: {
      content_digest: digest,
      digest_algorithm: "sha256",
      digest_scope: "package_files_excluding_deployment_contract",
    },
    source: {
      publication_id: "publication-1",
      session_id: "session-1",
      workflow_id: "workflow-1",
      endpoint_id: "endpoint-1",
      creator_user_id: "user-1",
      captured_at_ms: 1,
    },
    compatibility: {
      package_version: 3,
      minimum_kernel_version: "0.1.0",
      minimum_local_daemon_protocol_version: 239,
    },
    routes: [{ id: "hook-1" }],
    provider_requirements: [],
    credential_slots: [{ slot_id: "provider:codex" }],
    configuration: [],
    capabilities: {},
    resources: {},
    presentation: {},
    signatures: [],
  }
}
