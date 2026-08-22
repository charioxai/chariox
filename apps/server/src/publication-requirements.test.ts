import assert from "node:assert/strict"
import test from "node:test"

import { validatePublicationRequirements } from "./publication-requirements.js"

const digest = `sha256:${"a".repeat(64)}`

test("publication requirements v2 validate exact extension shape and runtime presence", async () => {
  const requests: Record<string, unknown>[] = []
  const extension = {
    id: "skill:qa",
    kind: "skill" as const,
    name: "qa",
    version: digest,
    content_digest: digest,
    launch_definition: {
      kind: "skill_package",
      package: {
        name: "qa",
        description: "Quality checks",
        short_description: null,
        version_hash: "a".repeat(64),
        files: [],
      },
    },
    credential_slots: [],
    network_destinations: [],
    uses: [{ agent_id: "agent-1", node_ids: ["node-1"] }],
    readiness_test: { kind: "skill_materialized" },
    portability: { classification: "portable" as const },
  }

  await validatePublicationRequirements({
    schema_version: 2,
    extensions: [extension],
    credential_slots: [],
    network_destinations: [],
  }, {
    send: async (request) => {
      requests.push(request)
      if ("ListSkills" in request) {
        return { SkillsListed: { skills: [{ name: "qa", path: "/runtime-capabilities/user/skills/qa" }] } }
      }
      throw new Error(`unexpected request: ${JSON.stringify(request)}`)
    },
  }, "/runtime-workspace")

  assert.deepEqual(requests, [{ ListSkills: { workspace_id: "/runtime-workspace" } }])
})

test("publication requirements v2 reject inexact immutable extension requirements", async () => {
  await assert.rejects(
    () => validatePublicationRequirements({
      schema_version: 2,
      extensions: [{
        id: "skill:qa",
        kind: "skill",
        name: "qa",
        version: digest,
        content_digest: `sha256:${"b".repeat(64)}`,
        launch_definition: { kind: "skill_package" },
        credential_slots: [],
        network_destinations: [],
        uses: [{ agent_id: "agent-1", node_ids: ["node-1"] }],
        readiness_test: { kind: "skill_materialized" },
        portability: { classification: "portable" },
      }],
      credential_slots: [],
      network_destinations: [],
    }, { send: async () => assert.fail("invalid v2 requirements must fail before kernel lookup") }),
    /version must match its immutable content digest/,
  )
})

test("publication requirements v2 accept kernel-sorted credential slots from multiple extension kinds", async () => {
  const uses = [{ agent_id: "agent-1", node_ids: ["node-1"] }]
  const mcpSlot = {
    slot_id: "integration:mcp-github-bearer-aaaa",
    kind: "integration" as const,
    label: "github: OAuth or bearer token",
    integration: "github",
    extension_id: "mcp:github",
    role: "bearer",
    authentication_method: "oauth_or_api_key",
    required: true as const,
    agent_ids: ["agent-1"],
    node_ids: ["node-1"],
    readiness_test: "integration_native" as const,
  }
  const connectorSlot = {
    slot_id: "integration:connector-linear-credential-bbbb",
    kind: "integration" as const,
    label: "linear: Connector credential",
    integration: "linear",
    extension_id: "connector:linear",
    role: "credential",
    authentication_method: "api_key_or_service_account",
    required: true as const,
    agent_ids: ["agent-1"],
    node_ids: ["node-1"],
    readiness_test: "integration_native" as const,
  }
  const extensions = [{
    id: "mcp:github",
    kind: "mcp" as const,
    name: "github",
    version: digest,
    content_digest: digest,
    launch_definition: { kind: "streamable_http", url: "https://mcp.github.test/mcp" },
    credential_slots: [mcpSlot],
    network_destinations: [{
      id: "extension:mcp-github",
      host: { kind: "exact_dns" as const, value: "mcp.github.test" },
      ports: [443] as const,
      protocols: ["tls"] as const,
      credential_slot_ids: [mcpSlot.slot_id],
    }],
    uses,
    readiness_test: { kind: "mcp_initialize" },
    portability: { classification: "portable" as const },
  }, {
    id: "connector:linear",
    kind: "connector" as const,
    name: "linear",
    version: digest,
    content_digest: digest,
    launch_definition: null,
    credential_slots: [connectorSlot],
    network_destinations: [],
    uses,
    readiness_test: { kind: "connector_adapter" },
    portability: {
      classification: "local_only" as const,
      reason: "connector adapter is local",
      recommendation: "Use connected ingress.",
    },
  }]
  const requests: Record<string, unknown>[] = []

  await validatePublicationRequirements({
    schema_version: 2,
    extensions,
    // The kernel's BTreeMap emits top-level slots by slot_id, independently of extension order.
    credential_slots: [connectorSlot, mcpSlot],
    network_destinations: extensions.flatMap((extension) => extension.network_destinations),
  }, {
    send: async (request) => {
      requests.push(request)
      if ("ListMcpServers" in request) return { McpServersListed: { mcps: [{ name: "github" }] } }
      if ("ListConnectors" in request) return { ConnectorsListed: { connectors: [{ name: "linear" }] } }
      throw new Error(`unexpected request: ${JSON.stringify(request)}`)
    },
  }, "/runtime-workspace")

  assert.deepEqual(requests, [
    { ListMcpServers: { workspace_id: "/runtime-workspace" } },
    { ListConnectors: null },
  ])
})

test("publication requirements reject unsupported schema versions before kernel lookup", async () => {
  await assert.rejects(
    () => validatePublicationRequirements(
      { schema_version: 3 } as never,
      { send: async () => assert.fail("unsupported requirements must fail before kernel lookup") },
    ),
    /unsupported publication requirements schema_version 3/,
  )
})
