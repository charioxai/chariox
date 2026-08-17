import assert from "node:assert/strict"
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import { setWorkflowPublicationBinding } from "./shell-workflow-publication-bindings.js"

test("workflow publication binding edits enforce the immutable provider policy", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-publication-provider-policy-"))
  try {
    const digest = `sha256:${"a".repeat(64)}`
    await writeFile(join(root, "publication.json"), JSON.stringify({
      schema_version: 1,
      package_version: 3,
      default_bindings_path: "bindings.local.json",
      deployment_contract: { path: "deployment-contract.json", schema_version: 1 },
    }))
    await writeFile(join(root, "workflow.snapshot.json"), JSON.stringify({
      schema_version: 1,
      workflow: { id: "workflow-1", nodes: [{ id: "node-1", agent_id: "agent-1" }] },
      agents: [{ id: "agent-1", provider: "codex", model: "gpt-5.6", effort: "high" }],
    }))
    await writeFile(join(root, "deployment-contract.json"), JSON.stringify({
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
        minimum_local_daemon_protocol_version: 1,
      },
      routes: [{ id: "route-1" }],
      provider_requirements: [{ slot_id: "provider:codex", provider: "codex" }],
      credential_slots: [{ slot_id: "provider:codex", allowed_destination_ids: [] }],
      configuration: [{
        kind: "provider_profile",
        agent_id: "agent-1",
        allowed_providers: ["codex"],
        captured: { provider: "codex" },
      }],
      capabilities: {
        network: {
          policy_version: 1,
          default_action: "deny",
          destinations: [],
          provider_access: [{
            slot_id: "provider:codex",
            bundle_kind: "platform_managed",
            bundle_id: "codex-official-v1",
          }],
        },
      },
      resources: {},
      presentation: {},
      signatures: [],
    }))

    await assert.rejects(
      setWorkflowPublicationBinding(root, "agent-1", { provider: "claude", model: "sonnet" }),
      /provider claude is not packaged/,
    )
    await setWorkflowPublicationBinding(root, "agent-1", { provider: "codex", model: "gpt-5.6" })
    const bindings = JSON.parse(await readFile(join(root, "bindings.local.json"), "utf8"))
    assert.equal(bindings.provider_model_overrides[0].replacement.provider, "codex")
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})
