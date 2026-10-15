import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance } from "./cli-types.js"
import { parseSlashCommand } from "./commands.js"
import {
  handleCredentialSlashCommand,
  handleExtensionSlashCommand,
  handleMcpSlashCommand,
  handleSkillSlashCommand,
  VAULT_FOLDED_PASSPHRASE_HINT,
  type CapabilityCommandHandlerDeps,
} from "./capability-command-handlers.js"

test("App bindings use the existing slash grant path without a second remote confirmation", async () => {
  const harness = capabilityHarness(agent({ remote: true }))
  await handleExtensionSlashCommand(harness.deps, parseSlashCommand("/extension grant app agent-1 installed") as Extract<ReturnType<typeof parseSlashCommand>, { kind: "extension" }>)
  await handleExtensionSlashCommand(harness.deps, parseSlashCommand("/extension grant app agent-1 installed --credential secret") as Extract<ReturnType<typeof parseSlashCommand>, { kind: "extension" }>)
  assert.deepEqual(harness.grants, ["app:agent-1:installed"])
  assert.deepEqual(harness.notices, [])
  assert.deepEqual(harness.footers, ["info:bound App installed to agent-1", "error:App bindings take only an installation ID"])
})

test("extension slash command confirms active home-proxy grants before exposing home execution", async () => {
  const remote = agent({ remote: true })
  const harness = capabilityHarness(remote)

  await handleExtensionSlashCommand(harness.deps, parseSlashCommand("/extension grant script agent-1 lookup --env py") as Extract<ReturnType<typeof parseSlashCommand>, { kind: "extension" }>)
  await handleExtensionSlashCommand(harness.deps, parseSlashCommand("/extension grant script agent-1 lookup --env py --confirm-home-proxy") as Extract<ReturnType<typeof parseSlashCommand>, { kind: "extension" }>)

  assert.deepEqual(harness.grants, ["script:agent-1:lookup:py"])
  assert.match(harness.notices[0] ?? "", /Confirm exposing script lookup to remote agent agent-1/)
  assert.match(harness.notices[0] ?? "", /rerun: \/extension grant script agent-1 lookup --env py --confirm-home-proxy/)
  assert.deepEqual(harness.footers, [
    "error:confirmation required for home-proxy grant",
    "info:granted script lookup to agent-1",
  ])
})

test("extension slash command rejects retired skill grants", async () => {
  const remote = agent({ remote: true })
  const harness = capabilityHarness(remote)

  await handleExtensionSlashCommand(harness.deps, parseSlashCommand("/extension grant skill agent-1 qa") as Extract<ReturnType<typeof parseSlashCommand>, { kind: "extension" }>)

  assert.deepEqual(harness.grants, [])
  assert.deepEqual(harness.notices, [])
  assert.match(harness.footers[0]??"", /extension kind must be mcp, script, connector, or app/)
})

test("extension slash command renders remote sync status, retry, and audit counts", async () => {
  const remote = agent({
    remote: true,
    extension_grants: [{ kind: "script", name: "lookup", environment: "py" }],
    remote_extension_manifest_sync: {
      state: "failed",
      manifest_hash: "abcdef1234567890",
      last_error: "worker offline",
      pending_revoke: true,
    },
  })
  const retried = agent({
    remote: true,
    extension_grants: [{ kind: "script", name: "lookup", environment: "py" }],
    remote_extension_manifest_sync: {
      state: "synced",
      manifest_hash: "fedcba9876543210",
      pending_revoke: false,
    },
  })
  const auditEvents = [{
    kind: "home_extension.invoke.denied",
    timestamp_ms: 1780000000000,
    payload: {
      status: "denied",
      agent_ref: "agent-1",
      worker_machine_id: "machine-1",
      tool: { kind: "script", name: "lookup", tool_name: "lookup", safety: "read" },
      error: "worker mismatch",
    },
  }]
  const harness = capabilityHarness(remote, { auditEvents, retriedAgent: retried })

  await handleExtensionSlashCommand(harness.deps, parseSlashCommand("/extension sync-status agent-1") as Extract<ReturnType<typeof parseSlashCommand>, { kind: "extension" }>)
  await handleExtensionSlashCommand(harness.deps, parseSlashCommand("/extension sync-retry agent-1") as Extract<ReturnType<typeof parseSlashCommand>, { kind: "extension" }>)
  await handleExtensionSlashCommand(harness.deps, parseSlashCommand("/extension audit agent-1 --limit 1") as Extract<ReturnType<typeof parseSlashCommand>, { kind: "extension" }>)

  assert.equal(harness.syncRetries, 1)
  assert.deepEqual(harness.auditRequests, [{ agentRef: "agent-1", limit: 1 }])
  assert.match(harness.notices[0] ?? "", /agent-1 remote extension sync: failed, pending revoke, worker offline/)
  assert.match(harness.notices[0] ?? "", /manifest hash: abcdef1234567890/)
  assert.match(harness.notices[0] ?? "", /revoke state: pending worker acknowledgement/)
  assert.match(harness.notices[0] ?? "", /next: keep the home revoke in place; run \/extension sync-status agent-1; run \/machine kernels machine-1 if the revoke stays pending; use \/extension sync-retry agent-1 after the worker reconnects/)
  assert.match(harness.notices[1] ?? "", /agent-1 remote extension sync: synced/)
  assert.match(harness.notices[1] ?? "", /manifest hash: fedcba9876543210/)
  assert.match(harness.notices[2] ?? "", /home_extension\.invoke\.denied lookup denied/)
  assert.match(harness.notices[2] ?? "", /boundary: home grants, validates, and executes; worker receives projected tools only; credentials stay home/)
  assert.match(harness.notices[2] ?? "", /tool: script:lookup as=lookup safety=read/)
  assert.match(harness.notices[2] ?? "", /error: worker mismatch/)
  assert.deepEqual(harness.footers, [
    "info:showing remote extension sync for agent-1",
    "info:retried remote extension sync for agent-1",
    "info:home extension audit for agent-1: 1 event",
  ])
})

test("extension slash command reports empty home extension audit accurately", async () => {
  const remote = agent({ remote: true })
  const harness = capabilityHarness(remote, { auditEvents: [] })

  await handleExtensionSlashCommand(harness.deps, parseSlashCommand("/extension audit agent-1") as Extract<ReturnType<typeof parseSlashCommand>, { kind: "extension" }>)

  assert.deepEqual(harness.notices, ["no home extension audit events"])
  assert.deepEqual(harness.footers, ["info:home extension audit for agent-1: 0 events"])
})

test("mcp slash command confirms active home-proxy shorthand grants", async () => {
  const remote = agent({ remote: true })
  const harness = capabilityHarness(remote)

  await handleMcpSlashCommand(harness.deps, parseSlashCommand("/mcp grant agent-1 filesystem") as Extract<ReturnType<typeof parseSlashCommand>, { kind: "mcp" }>)

  assert.deepEqual(harness.grants, [])
  assert.match(harness.notices[0] ?? "", /Confirm exposing mcp filesystem to remote agent agent-1/)
  assert.match(harness.notices[0] ?? "", /rerun: \/mcp grant agent-1 filesystem --confirm-home-proxy/)
  assert.deepEqual(harness.footers, ["error:confirmation required for home-proxy grant"])
})

test("a rejected vault passphrase adds the one-line recovery hint and still fails", async () => {
  const command = parseSlashCommand("/credential vault manage") as Extract<ReturnType<typeof parseSlashCommand>, { kind: "credential" }>
  for (const message of [
    "failed to unlock Chariox vault; passphrase may be incorrect or the vault is corrupted",
    "the current Chariox vault passphrase is incorrect; the passphrase is unchanged",
  ]) {
    const harness = capabilityHarness(agent())
    harness.deps.manageCredentialVault = async () => { throw new Error(message) }
    await assert.rejects(handleCredentialSlashCommand(harness.deps, command), { message })
    assert.deepEqual(harness.notices, [VAULT_FOLDED_PASSPHRASE_HINT])
  }
  const cancelled = capabilityHarness(agent())
  cancelled.deps.manageCredentialVault = async () => { throw new Error("Chariox vault unlock was cancelled") }
  await assert.rejects(handleCredentialSlashCommand(cancelled.deps, command))
  assert.deepEqual(cancelled.notices, [])
  assert.match(VAULT_FOLDED_PASSPHRASE_HINT, /lower case and without spaces.*Change passphrase/)
})

test("a vault passphrase change reports its own outcome", async () => {
  const harness = capabilityHarness(agent())
  harness.deps.manageCredentialVault = async () => ({ action: "passphrase_changed", status: { unlocked: true } })
  await handleCredentialSlashCommand(harness.deps, parseSlashCommand("/credential vault manage") as Extract<ReturnType<typeof parseSlashCommand>, { kind: "credential" }>)
  assert.deepEqual(harness.footers, ["info:vault passphrase changed"])
})

function capabilityHarness(agent: AgentInstance, options: {
  readonly auditEvents?: Record<string, unknown>[]
  readonly retriedAgent?: AgentInstance
} = {}) {
  const notices: string[] = []
  const footers: string[] = []
  const grants: string[] = []
  const auditRequests: Array<{ agentRef: string; limit: number | null | undefined }> = []
  let syncRetries = 0
  const deps: CapabilityCommandHandlerDeps = {
    grantAgentApp: async (agentRef, name) => { grants.push(`app:${agentRef}:${name}`); return agent },
    appendNotice: (message) => notices.push(message),
    flashFooter: (message, tone) => footers.push(`${tone}:${message}`),
    resolveSessionAgent: (reference) => reference === agent.agent_ref || reference === agent.id
      ? { agent, error: null }
      : { agent: null, error: `agent '${reference}' not found` },
    grantAgentMcp: async (agentRef, name) => {
      grants.push(`mcp:${agentRef}:${name}`)
      return agent
    },
    grantAgentSkill: async (agentRef, name) => {
      grants.push(`skill:${agentRef}:${name}`)
      return agent
    },
    grantAgentScript: async (agentRef, name, environment) => {
      grants.push(`script:${agentRef}:${name}:${environment}`)
      return agent
    },
    syncRemoteExtensionManifest: async () => {
      syncRetries += 1
      return options.retriedAgent ?? agent
    },
    listHomeExtensionAudit: async (agentRef, limit) => {
      auditRequests.push({ agentRef, limit })
      return options.auditEvents ?? []
    },
  }
  return { auditRequests, deps, footers, grants, notices, get syncRetries() { return syncRetries } }
}

function agent(options: Partial<AgentInstance> & { readonly remote?: boolean } = {}): AgentInstance {
  const { remote, ...overrides } = options
  return {
    id: "agent-1",
    agent_ref: "agent-1",
    session_id: "session-1",
    alias: null,
    provider: "opencode",
    model: "default",
    worktree_id: "/repo",
    remote_execution: remote ? {
      worker_kernel_id: "worker-1",
      worker_machine_id: "machine-1",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
    } : null,
    extension_grants: [],
    remote_extension_manifest_sync: null,
    state: "Idle",
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 0,
    last_activity_at_ms: 0,
    ...overrides,
  }
}

// MP-08 / MP-10 / MP-11: foundation kernels retain working management.
test("skill management survives the explicit unavailable v2 foundation endpoint", async () => {
  const { SkillsTuiController } = await import("./skills-tui-controller.js")
  const notices: string[] = [], calls: string[] = []
  const metadata = { name: "review", description: "Review", source_path: "/work/review", instructions: "Read", files: [] } as any
  const deps: CapabilityCommandHandlerDeps = {
    flashFooter: () => {}, appendNotice: t => notices.push(t), resolveSessionAgent: () => { throw Error("No agent") },
    listSkills: async () => { calls.push("list"); return [metadata] },
    getSkill: async () => { calls.push("show"); return metadata },
    installSkill: async path => { calls.push(`install:${path}`); return metadata },
    updateSkill: async path => { calls.push(`update:${path}`); return metadata },
    uninstallSkill: async name => { calls.push(`remove:${name}`); return metadata },
    importSkills: async provider => { calls.push(`import:${provider}`); return { imported: [], skipped: [], failed: [] } as any },
    skillsV2: new SkillsTuiController({context:()=>({key:"kernel",projectId:null,agentId:null,sessionId:null}),provider:()=>"codex",notice:t=>notices.push(t),send:async r=>{
      assert.deepEqual(r,{RelayStatus:null})
      return {RelayStatus:{status:{capabilities:[]}}}
    }}),
  }
  for(const raw of ["/skills", "/skill list", "/skill show review", '/skill install "/work/my skills"', '/skill update "/work/my skills"', "/skill remove review", "/skill import codex"]) {
    await handleSkillSlashCommand(deps,parseSlashCommand(raw) as any)
  }
  assert.deepEqual(calls,["list","list","show","install:/work/my skills","update:/work/my skills","remove:review","import:codex"])
  assert.match(notices.join("\n"),/review/)
})

test("v2 catalog refusal and failed publication never fall through to old mutations", async () => {
  const { SkillsTuiController } = await import("./skills-tui-controller.js")
  for(const message of ["Permission denied", "Skills revision changed", "Connection closed"]) {
    let oldCalls=0
    const deps:CapabilityCommandHandlerDeps={flashFooter:()=>{},appendNotice:()=>{},resolveSessionAgent:()=>{throw Error("No agent")},listSkills:async()=>{oldCalls++;return []},skillsV2:new SkillsTuiController({context:()=>({key:"kernel",projectId:null,agentId:null,sessionId:null}),provider:()=>"codex",notice:()=>{},send:async r=> "RelayStatus" in r ? {RelayStatus:{status:{capabilities:["skills_v2_runtime"]}}} : ({Error:{message}})})}
    await assert.rejects(()=>handleSkillSlashCommand(deps,parseSlashCommand("/skills") as any),new RegExp(message))
    assert.equal(oldCalls,0)
  }
})

// MP-08 / MP-10 / MP-11: only capability absence permits the temporary route.
test("pre-v2 kernel management works without replaying a refused publication", async () => {
  const { SkillsTuiController } = await import("./skills-tui-controller.js")
  let legacy = 0
  const notices: string[] = []
  const context=()=>({key:"kernel",projectId:null,agentId:null,sessionId:null})
  const deps:CapabilityCommandHandlerDeps={flashFooter:()=>{},appendNotice:t=>notices.push(t),resolveSessionAgent:()=>{throw Error("No agent")},listSkills:async()=>{legacy++;return []}}
  deps.skillsV2=new SkillsTuiController({context,provider:()=>"codex",notice:t=>notices.push(t),send:async r=>{assert.deepEqual(r,{RelayStatus:null});return {RelayStatus:{status:{capabilities:[]}}}}})
  await handleSkillSlashCommand(deps,parseSlashCommand("/skills") as any)
  assert.equal(legacy,1)
  legacy=0
  const calls:string[]=[]
  deps.skillsV2=new SkillsTuiController({context,provider:()=>"codex",notice:t=>notices.push(t),send:async r=>{
    if ("RelayStatus" in r) return {RelayStatus:{status:{capabilities:["skills_v2_runtime"]}}}
    const a=r.SkillsV2;calls.push(a.action)
    if(a.action==="mutate") return {Error:{message:"Skills v2 catalog service is not available in this protocol foundation build"}}
    return {SkillsV2:{result:a.action==="catalog"?{kind:"catalog",revision:1,skills:[],packages:[],native:[],metadata_budget_percent:1}:{kind:"preview",expected_revision:1,affected:[],collisions:[],review_receipt:{id:"r",mutation_sha256:"hash",expires_at_ms:Date.now()+60000,sources:[]}}}}
  }})
  await handleSkillSlashCommand(deps,parseSlashCommand("/skills create-package Review") as any)
  await assert.rejects(()=>handleSkillSlashCommand(deps,parseSlashCommand("/skills confirm") as any),/foundation build/)
  assert.equal(legacy,0)
  assert.equal(calls.filter(c=>c==="mutate").length,1)
})

// MP-08 / MP-10 / MP-11: v2 syntax cannot invoke path-based foundation writes.
test("unavailable v2 mutations keep their meaning and never call foundation handlers", async () => {
  const { SkillsTuiController } = await import("./skills-tui-controller.js")
  let writes=0
  const deps:CapabilityCommandHandlerDeps={flashFooter:()=>{},appendNotice:()=>{},resolveSessionAgent:()=>{throw Error("No agent")},installSkill:async()=>{writes++;throw Error("Wrong install")},updateSkill:async()=>{writes++;throw Error("Wrong update")},uninstallSkill:async()=>{writes++;throw Error("Wrong remove")},importSkills:async()=>{writes++;throw Error("Wrong import")},skillsV2:new SkillsTuiController({context:()=>({key:"kernel",projectId:null,agentId:null,sessionId:null}),provider:()=>"codex",notice:()=>{},send:async()=>({RelayStatus:{status:{capabilities:[]}}})})}
  for(const raw of ["/skills install folder /work/source", "/skills update review", "/skills remove skill-id", "/skills import native-id", "/skills confirm"]) {
    await assert.rejects(()=>handleSkillSlashCommand(deps,parseSlashCommand(raw) as any),/Skills v2 is unavailable/)
  }
  assert.equal(writes,0)
})
