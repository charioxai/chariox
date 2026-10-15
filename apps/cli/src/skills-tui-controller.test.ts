// MP-08 / MP-10 / MP-11 — supplementary TUI contract checks.
import assert from "node:assert/strict"
import { test } from "node:test"
import { parseSlashCommand } from "./commands.js"
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

// MP-08 / MP-10 / MP-11: exercise the same parser the real prompt uses.
test("slash install preserves quoted folders and JSON content through review", async () => {
  const body = '---\nname: review\ndescription: Review ordinary source files\n---\nRead "all files".\n'
  const cases = [
    {raw:'/skills install folder "/work/my skills" rename review',source:{kind:"folder",path:"/work/my skills"}},
    {raw:`/skills install content ${JSON.stringify(body)} rename review`,source:{kind:"content",skill_md:body}},
  ]
  for (const entry of cases) {
    let reviewed: unknown
    const c = new SkillsTuiController({context:()=>({key:"k",projectId:null,agentId:null,sessionId:null}),provider:()=>"codex",notice:()=>{},send:async r=>{
      const action=r.SkillsV2
      if(action.action==="preview") reviewed=action.mutation.actions[0]
      return {SkillsV2:{result:action.action==="catalog"?{kind:"catalog",revision:1,skills:[],packages:[],native:[],metadata_budget_percent:1}:action.action==="loads"?{kind:"loads",records:[]}:{kind:"preview",expected_revision:1,affected:[],collisions:[]}}}
    }})
    const command=parseSlashCommand(entry.raw)
    assert.equal(command?.kind,"skill")
    if(command?.kind!=="skill") throw Error("Expected skill command")
    await c.command(command.args)
    assert.deepEqual(reviewed,{action:"install",source:entry.source,collision:{action:"rename",name:"review"},package_id:null})
  }
})
