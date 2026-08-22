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

test("publication requirements reject unsupported schema versions before kernel lookup", async () => {
  await assert.rejects(
    () => validatePublicationRequirements(
      { schema_version: 3 } as never,
      { send: async () => assert.fail("unsupported requirements must fail before kernel lookup") },
    ),
    /unsupported publication requirements schema_version 3/,
  )
})
