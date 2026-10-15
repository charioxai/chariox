// MP-08 / MP-10 / MP-11: shared Skills v2 wire snapshots, not live acceptance.
import { test } from "node:test"
import assert from "node:assert/strict"
import { skillCatalogRequest, mutateSkillCatalogRequest, nativeSkillDetailsRequest, skillsV2Request, SKILLS_MIN_LOCAL_PROTOCOL, SKILLS_MIN_PEER_PROTOCOL } from "./ipc-skills-v2-requests.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"

test("MP-08 Skills v2 533/141 request snapshots", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 533)
  assert.equal(SKILLS_MIN_LOCAL_PROTOCOL, 533)
  assert.equal(SKILLS_MIN_PEER_PROTOCOL, 141)
  assert.deepEqual(skillCatalogRequest("project-1", "agent-1"), {
    SkillsV2: { action: "catalog", project_id: "project-1", agent_id: "agent-1" },
  })
  assert.deepEqual(mutateSkillCatalogRequest({ scope: { kind: "project", project_id: "project-1" }, expected_revision: 4, actions: [{ action: "set_state", skill_id: "s1", state: null }] }, "review-1"), {
    SkillsV2: { action: "mutate", review_receipt_id: "review-1", mutation: { scope: { kind: "project", project_id: "project-1" }, expected_revision: 4, actions: [{ action: "set_state", skill_id: "s1", state: null }] } },
  })
})
test("MP-11 native selection binds provider/account/placement without a client path", () => {
  const wire = skillsV2Request({ action: "submit", session_id: "s", attachment_id: "a", target_agent_id: "g", prompt: "$review", attachments: [], selections: [{ source: "native", entry_id: "n", name: "review", provider: "codex", account_profile: "p", execution_id: "e", inventory_revision: "r" }] })
  assert.equal(JSON.stringify(wire).includes('"path"'), false)
  assert.equal(wire.SkillsV2.action, "submit")
})

test("MP-11 mutations cannot omit or discard the home review receipt", () => {
  const mutation = { scope: { kind: "chariox" as const }, expected_revision: 0, actions: [] }
  assert.throws(() => mutateSkillCatalogRequest(mutation, ""), /preview|review/i)
  assert.deepEqual(mutateSkillCatalogRequest(mutation, "review-owned"), {
    SkillsV2: { action: "mutate", mutation, review_receipt_id: "review-owned" },
  })
})

test("MP-08 MP-11 native details keep the focused source identity and no client path", () => {
  const selection = { source: "native" as const, entry_id: "n", name: "review", provider: "codex", account_profile: "p", execution_id: "e", inventory_revision: "r" }
  assert.deepEqual(nativeSkillDetailsRequest("a", selection), {
    SkillsV2: { action: "native_details", agent_id: "a", selection },
  })
  assert.equal(JSON.stringify(nativeSkillDetailsRequest("a", selection)).includes('"path"'), false)
})
