// MP-08 / MP-10 / MP-11 — supplementary TUI contract checks.
import assert from "node:assert/strict"
import { test } from "node:test"
import { SkillsTuiController } from "./skills-tui-controller.js"
import type { SkillsV2Request } from "@chariox/kernel-client/kernel-types-skills-v2"

test("TUI scopes, previews, confirmations and package actions share v2 requests", async () => {
  const calls: { SkillsV2: SkillsV2Request }[] = []
  const output: string[] = []
  const controller = new SkillsTuiController({
    context: () => ({ key: "k", projectId: "p", agentId: "a", sessionId: "s" }),
    provider: () => "codex",
    notice: text => output.push(text),
    send: async request => {
      calls.push(request)
      const action = request.SkillsV2.action
      const result = action === "catalog"
        ? { kind: "catalog", revision: 7, skills: [], packages: [], native: [], metadata_budget_percent: 1 }
        : action === "loads" ? { kind: "loads", records: [] }
        : action === "preview" ? { kind: "preview", expected_revision: 7, affected: [], collisions: [] }
        : { kind: "published", revision: 8 }
      return { SkillsV2: { result } }
    },
  })
  await controller.command(["project"])
  await controller.command(["create-package", "Review"])
  const preview = calls.at(-1)!.SkillsV2
  assert.equal(preview.action, "preview")
  if (preview.action !== "preview") throw new Error("Preview missing")
  assert.deepEqual(preview.mutation.scope, { kind: "project", project_id: "p" })
  assert.ok(!calls.some(r => r.SkillsV2.action === "mutate"))
  await controller.command(["confirm"])
  assert.ok(calls.some(r => r.SkillsV2.action === "mutate"))
  assert.match(output.join("\n"), /Ungrouped/)
})

test("TUI rejects obsolete grants and Project operations with no Project", async () => {
  const controller = new SkillsTuiController({
    context: () => ({ key: "k", projectId: null, agentId: null, sessionId: null }),
    provider: () => "codex", notice: () => {},
    send: async () => ({ SkillsV2: { result: { kind: "catalog", revision: 1, skills: [], packages: [], native: [], metadata_budget_percent: 1 } } }),
  })
  await assert.rejects(() => controller.command(["grant", "a", "s"]), /\/skills/)
  await assert.rejects(() => controller.command(["project"]), /Project/)
})
