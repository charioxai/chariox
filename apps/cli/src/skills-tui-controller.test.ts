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
        : action === "preview" ? { kind: "preview", expected_revision: 7, affected: [], collisions: [], review_receipt: {id:"reviewed-by-home",expires_at_ms:Date.now()+60000,mutation_sha256:"hash",sources:[]} }
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
  assert.equal((calls.find(r=>r.SkillsV2.action==="mutate")!.SkillsV2 as any).review_receipt_id,"reviewed-by-home")
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
      return {SkillsV2:{result:action.action==="catalog"?{kind:"catalog",revision:1,skills:[],packages:[],native:[],metadata_budget_percent:1}:action.action==="loads"?{kind:"loads",records:[]}:{kind:"preview",expected_revision:1,affected:[],collisions:[],review_receipt:{id:"reviewed-by-home",expires_at_ms:Date.now()+60000,mutation_sha256:"hash",sources:[]}}}}
    }})
    const command=parseSlashCommand(entry.raw)
    assert.equal(command?.kind,"skill")
    if(command?.kind!=="skill") throw Error("Expected skill command")
    await c.command(command.args)
    assert.deepEqual(reviewed,{action:"install",source:entry.source,collision:{action:"rename",name:"review"},package_id:null})
  }
})

// MP-08 / MP-10 / MP-11: native details stay read-only and source-bound.
test("TUI show-from uses native details without importing", async () => {
  const selection={source:"native",entry_id:"n",name:"review",provider:"codex",account_profile:"acct",execution_id:"local",inventory_revision:"r1"}
  const calls:any[]=[];const output:string[]=[]
  const controller=new SkillsTuiController({context:()=>({key:"k",projectId:null,agentId:"a",sessionId:null}),provider:()=>"codex",notice:t=>output.push(t),send:async r=>{
    const a=r.SkillsV2;calls.push(a)
    return {SkillsV2:{result:a.action==="catalog"?{kind:"catalog",revision:1,skills:[],packages:[],native:[{selection}],metadata_budget_percent:1}:a.action==="loads"?{kind:"loads",records:[]}:{kind:"native_details",details:{entry:{selection,digest:"digest"},provenance:{locator:"safe-provider-source",source_machine:"worker",provider_version:"1"},files:[{path:"SKILL.md",bytes:10,sha256:"file-hash",executable:false}],declared_version:"v1",body:"Read instructions",compatibility_notes:["No tool grants"],unavailable_reason:null}}}}
  }})
  await controller.command(["show-from","n"])
  assert.deepEqual(calls.at(-1),{action:"native_details",agent_id:"a",selection})
  assert.match(output.join("\n"),/safe-provider-source/)
  assert.match(output.join("\n"),/SKILL.md/)
  assert.ok(!calls.some(c=>c.action==="mutate"))
})

// MP-08 / MP-10 / MP-11: expiration asks for another review, never a fresh fetch on confirm.
test("TUI refuses expired review receipts without publishing", async () => {
  const receipt={id:"review",expires_at_ms:Date.now()+60000,mutation_sha256:"hash",sources:[]}
  const calls:string[]=[]
  const c=new SkillsTuiController({context:()=>({key:"k",projectId:null,agentId:null,sessionId:null}),provider:()=>"codex",notice:()=>{},send:async r=>{
    const action=r.SkillsV2.action;calls.push(action)
    return {SkillsV2:{result:action==="catalog"?{kind:"catalog",revision:1,skills:[],packages:[],native:[],metadata_budget_percent:1}:{kind:"preview",expected_revision:1,affected:[],collisions:[],review_receipt:receipt}}}
  }})
  await c.command(["create-package","Review"])
  receipt.expires_at_ms=Date.now()-1
  await assert.rejects(()=>c.command(["confirm"]),/review.*again/i)
  assert.ok(!calls.includes("mutate"))
})
