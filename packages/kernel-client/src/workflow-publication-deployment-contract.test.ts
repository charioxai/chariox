import assert from "node:assert/strict"
import test from "node:test"

import {
  assertWorkflowPublicationDeploymentRuntimeCompatibility,
  resolveWorkflowPublicationDeploymentContract,
  workflowPublicationDeploymentContractPath,
  workflowPublicationDeploymentNetworkPolicy,
} from "./workflow-publication-deployment-contract.js"

const TARGET_RUNTIME = { targetLocalDaemonProtocolVersion: 240 }

test("publication deployment contract rejects obsolete package versions", () => {
  assert.throws(
    () => resolveWorkflowPublicationDeploymentContract({ package_version: 1 }, undefined),
    /unsupported publication package_version 1/,
  )
  assert.throws(
    () => resolveWorkflowPublicationDeploymentContract({ package_version: 2 }, undefined),
    /unsupported publication package_version 2/,
  )
  assert.throws(
    () => resolveWorkflowPublicationDeploymentContract({}, undefined),
    /publication package_version must be 3/,
  )
})

test("publication deployment contract validates v3 provenance", () => {
  const publicationPackage = {
    package_version: 3,
    publication_id: "publication-1",
    source_session_id: "session-1",
    workflow_id: "workflow-1",
    source_workflow_revision: 7,
    source_snapshot_digest: `sha256:${"b".repeat(64)}`,
    deployment_contract: { path: "deployment-contract.json", schema_version: 1 },
  }
  const contract = fixture()
  ;(contract.source as Record<string, unknown>).workflow_revision = 7
  ;(contract.source as Record<string, unknown>).snapshot_digest = `sha256:${"b".repeat(64)}`
  assert.equal(workflowPublicationDeploymentContractPath(publicationPackage), "deployment-contract.json")
  const resolution = resolveWorkflowPublicationDeploymentContract(publicationPackage, contract)
  assert.equal(resolution.kind, "native")
  assert.equal(resolution.contract?.source.endpoint_id, "endpoint-1")

  assert.throws(
    () => resolveWorkflowPublicationDeploymentContract(
      { ...publicationPackage, source_workflow_revision: 8 },
      contract,
    ),
    /workflow_revision does not match/,
  )
  assert.throws(
    () => resolveWorkflowPublicationDeploymentContract(
      { ...publicationPackage, source_snapshot_digest: `sha256:${"c".repeat(64)}` },
      contract,
    ),
    /snapshot_digest does not match/,
  )
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

test("publication deployment contract separates portable validation from target runtime admission", () => {
  const publicationPackage = {
    package_version: 3,
    deployment_contract: { path: "deployment-contract.json", schema_version: 1 },
  }

  const compatible = resolveWorkflowPublicationDeploymentContract(publicationPackage, fixture(240))
  assert.equal(compatible.kind, "native")
  if (compatible.kind !== "native") assert.fail("expected native deployment contract")
  assert.doesNotThrow(() => assertWorkflowPublicationDeploymentRuntimeCompatibility(compatible.contract, TARGET_RUNTIME))

  const future = resolveWorkflowPublicationDeploymentContract(publicationPackage, fixture(241))
  assert.equal(future.kind, "native")
  if (future.kind !== "native") assert.fail("expected native deployment contract")
  assert.throws(
    () => assertWorkflowPublicationDeploymentRuntimeCompatibility(future.contract, TARGET_RUNTIME),
    /requires local daemon protocol version 241, but target runtime supports 240/,
  )
})

test("publication deployment contract validates an exact deny-by-default egress ceiling", () => {
  const contract = fixture() as ReturnType<typeof fixture>
  contract.provider_requirements = [{ slot_id: "provider:codex" }]
  contract.credential_slots = [
    { slot_id: "provider:codex", allowed_destination_ids: [] },
    { slot_id: "integration:github", allowed_destination_ids: ["integration:github-api"] },
  ]
  contract.capabilities = {
    network: {
      policy_version: 1,
      default_action: "deny",
      destinations: [{
        id: "integration:github-api",
        host: { kind: "exact_dns", value: "api.github.com" },
        ports: [443],
        protocols: ["tls"],
        credential_slot_ids: ["integration:github"],
      }],
      provider_access: [{
        slot_id: "provider:codex",
        bundle_kind: "platform_managed",
        bundle_id: "codex-official-v1",
      }],
    },
  }
  const validated = resolveWorkflowPublicationDeploymentContract({
    package_version: 3,
    deployment_contract: { path: "deployment-contract.json", schema_version: 1 },
  }, contract)
  assert.equal(validated.kind, "native")
  if (validated.kind !== "native") assert.fail("expected native deployment contract")
  const policy = workflowPublicationDeploymentNetworkPolicy(validated.contract)
  assert.equal(policy.kind, "enforced")
  if (policy.kind !== "enforced") assert.fail("expected enforced network policy")
  assert.equal(policy.destinations[0]?.host.value, "api.github.com")

  const inconsistent = structuredClone(contract)
  ;(inconsistent.credential_slots as Array<Record<string, unknown>>)[1]!.allowed_destination_ids = []
  assert.throws(() => resolveWorkflowPublicationDeploymentContract({
    package_version: 3,
    deployment_contract: { path: "deployment-contract.json", schema_version: 1 },
  }, inconsistent), /destination ceiling is inconsistent/)
})

test("publication deployment contract rejects missing and obsolete egress policies", () => {
  const contract = fixture()
  contract.capabilities = {}
  assert.throws(() => resolveWorkflowPublicationDeploymentContract({
    package_version: 3,
    deployment_contract: { path: "deployment-contract.json", schema_version: 1 },
  }, contract), /deployment contract network policy must be an object/)

  contract.capabilities = { network: { egress_policy: "deployment_tightens" } }
  assert.throws(() => resolveWorkflowPublicationDeploymentContract({
    package_version: 3,
    deployment_contract: { path: "deployment-contract.json", schema_version: 1 },
  }, contract), /network policy fields are invalid/)
})

function fixture(minimumLocalDaemonProtocolVersion = 240): Record<string, unknown> {
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
      minimum_local_daemon_protocol_version: minimumLocalDaemonProtocolVersion,
    },
    routes: [{ id: "hook-1" }],
    provider_requirements: [],
    credential_slots: [{ slot_id: "provider:codex" }],
    configuration: [],
    capabilities: {
      network: {
        policy_version: 1,
        default_action: "deny",
        destinations: [],
        provider_access: [],
      },
    },
    resources: {},
    presentation: {},
    signatures: [],
  }
}
