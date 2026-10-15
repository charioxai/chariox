// MP-08 / MP-10 / MP-11: shared Skills v2 wire snapshots, not live acceptance.
import { test } from "node:test"
import assert from "node:assert/strict"
import { skillCatalogRequest, mutateSkillCatalogRequest, skillsV2Request, SKILLS_MIN_LOCAL_PROTOCOL, SKILLS_MIN_PEER_PROTOCOL } from "./ipc-skills-v2-requests.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"

test("MP-08 Skills v2 533/141 request snapshots", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 533)
  assert.equal(SKILLS_MIN_LOCAL_PROTOCOL, 533)
  assert.equal(SKILLS_MIN_PEER_PROTOCOL, 141)
  assert.deepEqual(skillCatalogRequest("project-1", "agent-1"), {
    SkillsV2: { action: "catalog", project_id: "project-1", agent_id: "agent-1" },
  })
  assert.deepEqual(mutateSkillCatalogRequest({ scope: { kind: "project", project_id: "project-1" }, expected_revision: 4, actions: [{ action: "set_state", skill_id: "s1", state: null }] }), {
    SkillsV2: { action: "mutate", mutation: { scope: { kind: "project", project_id: "project-1" }, expected_revision: 4, actions: [{ action: "set_state", skill_id: "s1", state: null }] } },
  })
})
test("MP-11 native selection binds provider/account/placement without a client path", () => {
  const wire = skillsV2Request({ action: "submit", session_id: "s", attachment_id: "a", target_agent_id: "g", prompt: "$review", attachments: [], selections: [{ source: "native", entry_id: "n", name: "review", provider: "codex", account_profile: "p", execution_id: "e", inventory_revision: "r" }] })
  assert.equal(JSON.stringify(wire).includes('"path"'), false)
  assert.equal(wire.SkillsV2.action, "submit")
})
