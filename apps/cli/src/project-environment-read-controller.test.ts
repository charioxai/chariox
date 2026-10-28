// MP-08 / MP-10: no-agent request and late-response cancellation.
import assert from "node:assert/strict"
import test from "node:test"
import { environmentFromView } from "@chariox/kernel-client"
import { createProjectEnvironmentReadController } from "./project-environment-read-controller.js"
import { requireKernelFeatureProtocol } from "./kernel-feature-minimum.js"

function environmentSnapshot(environment: any) {
  return environmentFromView(environment.local_project_id, environmentReadResponse(environment), null)
}

function environmentReadResponse(environment: any) {
  const proposals = environment.proposals ?? []
  return { ProjectEnvironmentView: { view: {
    observed_at_ms: Date.now(), snapshot_digest: environment.content_digest ?? "snapshot",
    proposal_count: proposals.length, proposal_offset: 0, next_proposal_offset: null,
    operation_item_counts: [], ...environment.read_view, environment, proposals,
  } } }
}

test("ENV P01 standalone TUI request and protocol gate", async () => {
  const requests: unknown[] = []
  const controller = createProjectEnvironmentReadController({ send: async request => { requests.push(request); throw new Error("not available") }, render() {}, pageSize: () => 10, textWidth: () => 72 })
  await controller.open("project")
  assert.deepEqual(requests, [{ GetProjectEnvironment: { projectId: "project", view:{knownDigest:null,proposals:null} } }])
  assert.equal(controller.isOpen(), true)
  assert.deepEqual(controller.visibleLines(), ["Environment unavailable. Check the kernel connection and refresh.", "r · Refresh"])
  assert.throws(() => requireKernelFeatureProtocol(requests[0], 504), /660/)
  assert.doesNotThrow(() => requireKernelFeatureProtocol(requests[0], 660))
  controller.handleKey({ name: "escape" })
  assert.equal(controller.isOpen(), false)
})

test("ENV P01 closed TUI panel ignores a late kernel response", async () => {
  let reject: (error: Error) => void = () => undefined
  let renders = 0
  const controller = createProjectEnvironmentReadController({ send: () => new Promise((_resolve, fail) => { reject = fail }), render() { renders++ }, pageSize: () => 10, textWidth: () => 72 })
  const opening = controller.open("project")
  controller.close()
  reject(new Error("late"))
  await opening
  assert.equal(controller.isOpen(), false)
  assert.equal(renders, 2)
})

// MP-08 / MP-10 / MP-11: main explains its read-only boundary without diagnostics.
test("ENV-UX-01 TUI missing Project is an empty state", async () => {
  const controller = createProjectEnvironmentReadController({send: async () => { throw new Error("local transport `project.get` failed: project `missing` was not found (code=local_transport_error retryable=true)") }, render() {}, pageSize: () => 100, textWidth: () => 72})
  await controller.open("missing")
  assert(controller.visibleLines().includes("No Environment for this project yet."))
  assert(!controller.visibleLines().some(line => line.includes("local transport")))
})

// MP-08 / MP-10 / MP-11: the explanation wraps to the panel width and every wrapped row stays reachable.
test("ENV-UX-02 TUI explanation wraps to the panel width without losing text", async () => {
  const controller = createProjectEnvironmentReadController({ send: async () => { throw new Error("not available") }, render() {}, pageSize: () => 100, textWidth: () => 30 })
  await controller.open("project")
  const rows = controller.visibleLines()
  assert(rows.every(row => row.length <= 30), rows.filter(row => row.length > 30).join("|"))
  const text = rows.join(" ").replace(/\s+/g, " ")
  assert(text.includes("Environment unavailable. Check the kernel connection and refresh."))
})

test("ENV-UX-02 TUI scrolls wrapped rows, not source lines", async () => {
  const controller = createProjectEnvironmentReadController({ send: async () => { throw new Error("not available") }, render() {}, pageSize: () => 3, textWidth: () => 20 })
  await controller.open("project")
  for (let index = 0; index < 40; index++) controller.handleKey({ name: "down" })
  assert(controller.visibleLines().join(" ").endsWith("r · Refresh"), controller.visibleLines().join("|"))
  assert(controller.visibleLines().every(row => row.length <= 20))
})

// MP-08 / MP-10: P02a Detect must go through the real kernel request surface.
test("ENV P02a TUI Detect is read-only, uses kernel folders and shows evidence skips", async () => {
  const requests: any[] = []
  const environment: any = { schema_version: 1, local_project_id: "project", lineage: { environment_id: "env", project_id: "lineage" }, folders: [], project_requirements: [], proposals: [], operations: [], delivered_capabilities: { enabled_environment_operations: ["get", "detect"], supported_schema: 1 } }
  const controller = createProjectEnvironmentReadController({ send: async request => { requests.push(request); return "RelayStatus" in (request as any) ? { RelayStatus: { status: { machine_id: "machine", daemon_id: "kernel" } } } : environmentReadResponse(environment) }, render() {}, pageSize: () => 100, textWidth: () => 72 })
  await controller.open("project")
  controller.handleKey({ name: "d" })
  await new Promise(resolve => setTimeout(resolve, 0))
  assert.equal(requests.length, 3)
  assert.equal(requests[2].DetectProjectEnvironment.projectId, "project")
  assert.deepEqual(requests[2].DetectProjectEnvironment.allowModelFolders, [])
  assert.equal(requests.some(r => r.SaveProjectEnvironmentRevision), false)
})

// MP-08 / MP-10 / MP-11: first pending proposal can be accepted through the real key controller.
test("P02b TUI accept sends one CAS save and observes its revision", async () => {
  const requests: any[] = []
  const environment: any = { schema_version: 1, local_project_id: "project", revision: 3, content_digest: "current", lineage: { environment_id: "env", project_id: "lineage" }, folders: [], project_requirements: [], proposals: [{ proposal_id: "proposal", requirement: { requirement_id: "node", title: "Node", scope: { kind: "project" }, origins: [], spec: { kind: "software", identity: "node", version_constraint: null, detect_only: true } } }], operations: [], delivered_capabilities: { enabled_environment_operations: ["get", "save"], supported_schema: 1 } }
  const controller = createProjectEnvironmentReadController({ send: async request => { requests.push(request); return "SaveProjectEnvironmentRevision" in (request as any) ? { ProjectEnvironmentSaved: { environment: environmentSnapshot({ ...environment, revision: 4, proposals: [] }) } } : environmentReadResponse(environment) }, render() {}, pageSize: () => 100, textWidth: () => 72 })
  await controller.open("project")
  controller.handleKey({ name: "a" })
  await new Promise(resolve => setTimeout(resolve, 0))
  assert.equal(requests.length, 2)
  assert.deepEqual(requests[1].SaveProjectEnvironmentRevision.acceptedProposalIds, ["proposal"])
  assert.equal(requests[1].SaveProjectEnvironmentRevision.expectedRevision, 3)
  assert.deepEqual(requests[1].SaveProjectEnvironmentRevision.excludedProposalIds, [])
  assert(controller.visibleLines().some(line => line.includes("Revision 4")))
})

// MP-08 / MP-10 / MP-11: modified terminal keys cannot authorize a review decision.
test("P02b Ctrl+A is not proposal acceptance", async () => {
  const requests: any[]=[]
  const environment: any={schema_version:1,local_project_id:"project",revision:0,content_digest:"base",folders:[],project_requirements:[],proposals:[{proposal_id:"proposal",requirement:{requirement_id:"node",title:"Node",scope:{kind:"project"},origins:[],spec:{kind:"software",identity:"node",version_constraint:null,detect_only:true}}}],operations:[],delivered_capabilities:{enabled_environment_operations:["get","save"],supported_schema:1}}
  const controller=createProjectEnvironmentReadController({send:async request=>{requests.push(request);return environmentReadResponse(environment)},render(){},pageSize:()=>100, textWidth: () => 72})
  await controller.open("project")
  controller.handleKey({ name: "a", ctrl: true })
  await new Promise(resolve => setTimeout(resolve, 0))
  assert.equal(requests.length, 1, "modified keys must not save a review")
})

// MP-08 / MP-10 / MP-11: a review key must show the selected constraint and provenance first.
test("P02b selected proposal shows its constraint and origin in the visible review header", async () => {
 const environment:any={schema_version:1,local_project_id:"project",revision:0,content_digest:"base",lineage:{environment_id:"env",project_id:"lineage"},folders:[],project_requirements:[],proposals:[{proposal_id:"proposal",requirement:{requirement_id:"react",title:"React",scope:{kind:"project"},origins:[{kind:"detected_metadata",source:"package.json",reference:"react dependency"}],spec:{kind:"software",identity:"react",version_constraint:"19.0.0",detect_only:true}}}],operations:[],delivered_capabilities:{enabled_environment_operations:["get","save"],supported_schema:1}}
 const controller=createProjectEnvironmentReadController({send:async()=>(environmentReadResponse(environment)),render(){},pageSize:()=>6, textWidth: () => 72})
 await controller.open("project")
 assert(controller.visibleLines().some(line=>line==="Version constraint: 19.0.0"))
 assert(controller.visibleLines().some(line=>line.includes("package.json")))
})

// MP-08 / MP-10 / MP-11: real shaped relay RED showed ~893ms on reopening.
test("Environment reopens its last kernel view while a fresh Get is pending", async () => {
  const environment: any = { schema_version: 1, local_project_id: "project", lineage: {}, folders: [], project_requirements: [], proposals: [], observations: [], operations: [], delivered_capabilities: { enabled_environment_operations: ["get"] } }
  let count = 0, release: (value: unknown) => void = () => {}
  const controller = createProjectEnvironmentReadController({ send: () => ++count === 1 ? Promise.resolve(environmentReadResponse(environment)) : new Promise(resolve => { release = resolve }), render() {}, pageSize: () => 30, textWidth: () => 72 })
  await controller.open("project"); controller.close()
  const pending = controller.open("project")
  try {
    assert(controller.visibleLines().some(line => line.includes("Project-wide")), "last kernel snapshot should render immediately")
    assert(controller.visibleLines().some(line => line.includes("Refreshing")))
    assert.equal(count, 2, "every open still requests a fresh kernel snapshot")
  } finally { release(environmentReadResponse(environment)); await pending }
})

// MP-08 / MP-10 / MP-11: cached proposals are display-only until the current kernel read settles.
test("P02b cached review waits for refresh and caches successful Save", async () => {
  const environment: any = { schema_version: 1, local_project_id: "project", revision: 0, content_digest: "base", lineage: {}, folders: [], project_requirements: [], proposals: [{ proposal_id: "p", requirement: { requirement_id: "node", title: "Node", scope: { kind: "project" }, origins: [], spec: { kind: "software", identity: "node", version_constraint: "22", detect_only: true } } }], operations: [], delivered_capabilities: { enabled_environment_operations: ["get", "save"] } }
  let gets = 0, saves = 0, release: (value: unknown) => void = () => {}
  const controller = createProjectEnvironmentReadController({ send: async request => {
    if ("SaveProjectEnvironmentRevision" in (request as any)) { saves++; return { ProjectEnvironmentSaved: { environment: environmentSnapshot({ ...environment, revision: 1, proposals: [] }) } } }
    if (++gets === 1) return environmentReadResponse(environment)
    return new Promise(resolve => { release = resolve })
  }, render() {}, pageSize: () => 100, textWidth: () => 72 })
  await controller.open("project"); controller.close()
  const pending = controller.open("project")
  try {
    controller.handleKey({ name: "a" }); await new Promise(resolve => setTimeout(resolve, 0))
    assert.equal(saves, 0, "cached proposal must not authorize Save")
  } finally { release(environmentReadResponse(environment)); await pending }
  await controller.review("accept"); controller.close()
  const reopened = controller.open("project")
  try { assert(controller.visibleLines().some(line => line.includes("Revision 1")), "saved view replaces cached revision") }
  finally { release(environmentReadResponse({ ...environment, revision: 1, proposals: [] })); await reopened }
})

// MP-08 / MP-10 / MP-11: typed edits remain Web-first, with a real copyable Project link.
test("P03 TUI exposes a safe Web link and saved diff after review", async () => {
  const environment: any = { schema_version: 1, local_project_id: "project", revision: 0, content_digest: "base", folders: [], project_requirements: [], proposals: [], operations: [], delivered_capabilities: { enabled_environment_operations: ["get", "save"] } }
  let copied = ""
  const controller = createProjectEnvironmentReadController({ send: async r => "CloudRelayStatus" in (r as any) ? { CloudRelayStatus: { profile: { api_url: "https://owned.example/api" } } } : "RelayStatus" in (r as any) ? { RelayStatus: { status: { daemon_id: "owned-kernel" } } } : environmentReadResponse(environment), render() { }, pageSize: () => 100, textWidth: () => 72, copyLink: link => { copied = link } })
  await controller.open("project")
  controller.handleKey({ name: "w" })
  await new Promise(r => setTimeout(r, 0))
  assert.equal(copied, "https://owned.example/waiting-room?environmentProjectId=project&environmentKernelId=owned-kernel")
  assert(controller.visibleLines().some(l => l.includes("Web-first")))
})

// MP-08 / MP-10 / MP-11: Web edits are visible as a read-only difference between kernel snapshots.
test("P03 TUI inspects a Web-edited revision after Refresh", async () => {
  const requirement = { requirement_id: "node", title: "Node", scope: { kind: "project" }, origins: [], spec: { kind: "software", identity: "node", version_constraint: "22", platform: null, install_scope: "project", install_source: null, detect_only: true }, depends_on: [], platform_variants: [], required: true, legacy_entry: null }
  const previous: any = { schema_version: 1, local_project_id: "project", revision: 0, content_digest: "before", lineage: { environment_id: "env", project_id: "lineage" }, project_requirements: [requirement], folders: [], proposals: [], operations: [], delivered_capabilities: { enabled_environment_operations: ["get", "save"], supported_schema: 1 } }
  const current = { ...previous, revision: 1, content_digest: "after", project_requirements: [{ ...requirement, spec: { ...requirement.spec, version_constraint: "24" } }] }
  let reads = 0
  const controller = createProjectEnvironmentReadController({ send: async () => (environmentReadResponse(reads++ ? current : previous)), render() { }, pageSize: () => 100, textWidth: () => 72 })
  await controller.open("project"); await controller.open("project"); controller.handleKey({ name: "v" })
  assert(controller.visibleLines().some(line => line.startsWith("Observed revision changes")))
  assert(controller.visibleLines().includes("Version constraint: 22"))
  assert(controller.visibleLines().includes("Version constraint: 24"))
})

test("R4 Environment section navigation uses wrapped heading rows and folder identities", async () => {
  const environment: any = { schema_version: 1, local_project_id: "project", revision: 1, content_digest: "x".repeat(64), lineage: {}, folders: [
    { folder_id: "one", label: "A folder heading that wraps", requirements: [] },
    { folder_id: "two", label: "A folder heading that wraps", requirements: [] },
  ], project_requirements: [], proposals: [], operations: [], delivered_capabilities: { enabled_environment_operations: ["get", "save"] } }
  const controller = createProjectEnvironmentReadController({ send: async () => environmentReadResponse(environment), render() {}, pageSize: () => 3, textWidth: () => 18 })
  await controller.open("project")
  controller.handleKey({ name: "right" })
  assert.equal(controller.visibleLines()[0], "Project-wide")
  controller.handleKey({ name: "right" })
  assert.equal(controller.visibleLines()[0], "A folder heading")
  controller.handleKey({ name: "right" })
  assert.equal(controller.visibleLines()[0], "A folder heading")
  controller.handleKey({ name: "left" })
  assert.equal(controller.visibleLines()[0], "A folder heading")
  controller.handleKey({ name: "left" })
  assert.equal(controller.visibleLines()[0], "Project-wide")
})

// MP-08 / MP-10 / MP-11: unsupported Check is explicit and has no mutation.
test("P04 old kernel explains unavailable Check and pressing c sends no operation", async () => {
  const requests: unknown[] = []
  const environment: any = {schema_version:1,local_project_id:"project",revision:0,content_digest:"base",folders:[],project_requirements:[],proposals:[],operations:[],delivered_capabilities:{enabled_environment_operations:["get","save"],supported_schema:1}}
  const controller = createProjectEnvironmentReadController({send:async request=>{requests.push(request);return environmentReadResponse(environment)},render(){},pageSize:()=>100, textWidth: () => 72})
  await controller.open("project")
  controller.handleKey({name:"c"})
  await new Promise(resolve=>setTimeout(resolve,0))
  assert.equal(requests.length,1,"no unsupported Check request or identity lookup")
  assert(controller.visibleLines().includes("Check is unavailable on this kernel"))
})

// MP-08 / MP-10 / MP-11: a quiet terminal cannot retain expired readiness.
test("P04 idle TUI expires Ready without another kernel snapshot",async()=>{
 const environment:any={schema_version:1,local_project_id:"project",revision:1,content_digest:"digest",lineage:{project_id:"p",environment_id:"env"},delivered_capabilities:{enabled_environment_operations:["get"],supported_schema:1},project_requirements:[{requirement_id:"node",title:"Node",scope:{kind:"project"},origins:[],spec:{kind:"software",identity:"node",version_constraint:null,detect_only:true},depends_on:[],required:true}],folders:[],proposals:[],operations:[],observations:[{environment_id:"env",requirement_id:"node",revision_digest:"digest",status:"ready",checked_at_ms:Date.now()-300000+150,stale:false,safe_summary:"Named probe passed",target:{machine_id:"machine",target_instance_generation:"boot",slice_ref:null},measured_facts:{}}]}
 let requests=0,renders=0
 const controller=createProjectEnvironmentReadController({send:async()=>{requests++;return environmentReadResponse(environment)},render(){renders++},pageSize:()=>100, textWidth: () => 72})
 try {await controller.open("project");assert(controller.visibleLines().some(l=>l.includes("Ready")));const settled=renders;await new Promise(resolve=>setTimeout(resolve,250));assert(controller.visibleLines().some(l=>l.includes("Last check stale")));assert(renders>settled);assert.equal(requests,1)} finally{controller.close()}
})

// MP-08 / MP-10 / MP-11: a later client's operation cannot hold this TUI's Check lock.
test("review0212 TUI polls its acknowledged Check when a later operation replaces the aggregate",async()=>{
 const requests:any[]=[]
 let acknowledged=""
 const environment:any={schema_version:1,local_project_id:"project",revision:1,content_digest:"digest",lineage:{project_id:"p",environment_id:"env"},delivered_capabilities:{enabled_environment_operations:["get","check"],supported_schema:1},project_requirements:[],folders:[],proposals:[],operations:[],observations:[]}
 const controller=createProjectEnvironmentReadController({send:async request=>{
  requests.push(request)
  const r=request as any
  if(r.RelayStatus!==undefined)return {RelayStatus:{status:{machine_id:"machine",daemon_id:"kernel"}}}
  if(r.CheckProjectEnvironment){acknowledged=r.CheckProjectEnvironment.operationId;return {EnvironmentOperation:{operation:{operation_id:acknowledged,phase:"requested"}}}}
  if(r.GetEnvironmentOperation)return {EnvironmentOperation:{operation:{operation_id:acknowledged,phase:"ready"}}}
  return environmentReadResponse({...environment,operations:acknowledged?[{operation_id:"later-client",kind:"check",phase:"running"}]:[]})
 },render(){},pageSize:()=>100, textWidth: () => 72})
 let pending:Promise<void>|undefined
 try {
  await controller.open("project");pending=controller.check()
  await new Promise(resolve=>setTimeout(resolve,50))
  assert(requests.some(r=>r.GetEnvironmentOperation?.operationId===acknowledged),"retained acknowledged operation must be queried")
  await pending
 } finally {controller.close();await pending}
})

// MP-08 / MP-10 / MP-11: local dependency expiry needs no network refresh.
test("review1301 idle TUI expires dependent Ready without another kernel snapshot",async()=>{
 const environment:any={schema_version:1,local_project_id:"project",revision:1,content_digest:"digest",lineage:{project_id:"p",environment_id:"env"},delivered_capabilities:{enabled_environment_operations:["get"],supported_schema:1},project_requirements:[{requirement_id:"node",title:"Node",scope:{kind:"project"},origins:[],spec:{kind:"software",identity:"node",version_constraint:null,detect_only:true},depends_on:[],required:true}],folders:[],proposals:[],operations:[],observations:[{environment_id:"env",requirement_id:"node",revision_digest:"digest",status:"ready",checked_at_ms:Date.now()-300000+150,stale:false,safe_summary:"Named probe passed",target:{machine_id:"machine",target_instance_generation:"boot",slice_ref:null},measured_facts:{}}]}
 environment.project_requirements.push({...environment.project_requirements[0],requirement_id:"dependent",title:"Dependent",depends_on:["node"]});
 environment.observations.push({...environment.observations[0],requirement_id:"dependent",checked_at_ms:Date.now()});
 let requests=0,renders=0
 const controller=createProjectEnvironmentReadController({send:async()=>{requests++;return environmentReadResponse(environment)},render(){renders++},pageSize:()=>100, textWidth: () => 72})
 try {await controller.open("project");assert(controller.visibleLines().some(l=>l.includes("Ready")));const settled=renders;await new Promise(resolve=>setTimeout(resolve,250));assert(controller.visibleLines().some(l=>l.includes("Dependent · Last check stale")));assert(renders>settled);assert.equal(requests,1)} finally{controller.close()}
})

// MP-08 / MP-10 / MP-11: the TUI reaches proposal pages through the shared kernel read.
test("TUI next proposal crosses a snapshot-bound page and reports the total count",async()=>{
 const requests:any[]=[]
 const requirement={requirement_id:"one",title:"One",scope:{kind:"project"},origins:[],spec:{kind:"software",identity:"node",version_constraint:null,detect_only:true},depends_on:[],platform_variants:[],required:false,legacy_entry:null}
 const first={proposal_id:"one",requirement},second={proposal_id:"two",requirement:{...requirement,requirement_id:"two",title:"Two"}}
 const environment={schema_version:1,local_project_id:"p",lineage:{project_id:"p",environment_id:"e"},revision:0,content_digest:"digest",project_requirements:[],folders:[],proposals:[],observations:[],operations:[],delivered_capabilities:{enabled_environment_operations:["get","save"]}}
 const view={observed_at_ms:Date.now(),snapshot_digest:"snapshot",environment,proposal_count:2,proposal_offset:0,next_proposal_offset:0,proposals:[],operation_item_counts:[]}
 let rendered!:()=>void
 const controller=createProjectEnvironmentReadController({send:async request=>{requests.push(request);return {ProjectEnvironmentView:{view:requests.length===1 ? view : requests.length===2 ? {...view,environment:null,next_proposal_offset:1,proposals:[first]} : {...view,environment:null,proposal_offset:1,next_proposal_offset:null,proposals:[second]}}}},render(){rendered?.()},pageSize:()=>100, textWidth: () => 72})
 await controller.open("p")
 assert(controller.visibleLines().some(l=>l.includes("2 proposals · n to begin review")))
 const firstPage=new Promise<void>(resolve=>{rendered=resolve});controller.handleKey({name:"n"});await firstPage
 assert(controller.visibleLines().some(l=>l.includes("Review 1/2 · One")))
 const done=new Promise<void>(resolve=>{rendered=resolve})
 controller.handleKey({name:"n"});await done
 assert.equal(requests[2].GetProjectEnvironment.view.proposals.snapshotDigest,"snapshot")
 assert(controller.visibleLines().some(l=>l.includes("Review 2/2 · Two")))
 controller.close()
})

// MP-08 / MP-10 / MP-11: byte-bounded pages can contain fewer than twenty proposals.
test("TUI previous page returns to the actual prior byte-bounded offset", async () => {
  const offsets: number[] = []
  const proposals = Array.from({length:7},(_,i)=>({proposal_id:`p-${i}`,requirement:{requirement_id:`r-${i}`,title:`Requirement ${i}`,scope:{kind:"project"},origins:[],spec:{kind:"software",identity:"node",detect_only:true}}}))
  const environment: any = {schema_version:1,local_project_id:"project",revision:0,content_digest:"revision",lineage:{project_id:"lineage",environment_id:"env"},folders:[],project_requirements:[],proposals:proposals.slice(0,3),operations:[],observations:[],delivered_capabilities:{enabled_environment_operations:["get","save"],supported_schema:1},read_view:{snapshot_digest:"snapshot",proposal_count:7,proposal_offset:0,next_proposal_offset:3}}
  const controller = createProjectEnvironmentReadController({send:async (request:any)=>{
    const page=request.GetProjectEnvironment.view.proposals
    if(!page)return environmentReadResponse(environment)
    offsets.push(page.offset)
    return {ProjectEnvironmentView:{view:{environment:null,snapshot_digest:"snapshot",proposal_count:7,proposal_offset:page.offset,next_proposal_offset:page.offset===3?5:null,proposals:proposals.slice(page.offset,page.offset+2),operation_item_counts:[]}}}
  },render(){},pageSize:()=>100, textWidth: () => 72})
  await controller.open("project")
  await controller.proposalPage(3)
  await controller.proposalPage(5)
  controller.handleKey({name:"p"})
  await new Promise(resolve=>setTimeout(resolve,0))
  assert.deepEqual(offsets,[3,5,3])
  controller.close()
})

// MP-08 / MP-10 / MP-11: TUI proposal exclusion is an unchanged-content
// Save too. It must retain the compact kernel clock after a real Check reply.
for (const offset of [-600000,600000]) test(`review0634 TUI Save preserves checked rows at clock offset ${offset}`,async()=>{
 const realNow=Date.now,kernelNow=realNow();Date.now=()=>realNow()+offset
 let controller:ReturnType<typeof createProjectEnvironmentReadController>|undefined
 try {
  const row={requirement_id:"node",title:"Node",scope:{kind:"project"},origins:[],spec:{kind:"software",identity:"node",version_constraint:null,detect_only:true},depends_on:[],required:true,legacy_entry:null}
  const env:any={schema_version:1,local_project_id:"project",revision:1,content_digest:"digest",lineage:{environment_id:"env",project_id:"lineage"},folders:[],project_requirements:[row],proposals:[{proposal_id:"optional",requirement:{...row,requirement_id:"optional",title:"Optional",required:false}}],operations:[],observations:[],delivered_capabilities:{enabled_environment_operations:["get","save","check"],supported_schema:1}}
  let operationId="",saved=false
  const current=()=>({...env,revision:saved?2:1,proposals:saved?[]:env.proposals,operations:operationId?[{operation_id:operationId,kind:"check",phase:"ready",per_item_results:[]}]:[],observations:operationId?[{environment_id:"env",requirement_id:"node",revision_digest:"digest",status:"ready",checked_at_ms:kernelNow,stale:false,safe_summary:"Named probe passed",target:{machine_id:"machine",target_instance_generation:"boot",slice_ref:null},measured_facts:{}}]:[]})
  controller=createProjectEnvironmentReadController({render(){},pageSize:()=>100, textWidth: () => 72,send:async(request:any)=>{
   if(request.RelayStatus===null)return {RelayStatus:{status:{machine_id:"machine",daemon_id:"kernel"}}}
   if(request.CheckProjectEnvironment){operationId=request.CheckProjectEnvironment.operationId;return {EnvironmentOperation:{operation:{operation_id:operationId,phase:"ready"}}}}
   if(request.SaveProjectEnvironmentRevision){saved=true;return {ProjectEnvironmentSaved:{environment:current()}}}
   if(request.GetProjectEnvironment)return {ProjectEnvironmentView:{view:{observed_at_ms:realNow(),snapshot_digest:saved?"saved":operationId?"checked":"initial",environment:current(),proposal_count:current().proposals.length,proposal_offset:0,next_proposal_offset:null,proposals:current().proposals,operation_item_counts:[]}}}
   throw new Error("unexpected request")
  }})
  await controller.open("project");await controller.check()
  assert(controller.visibleLines().some(line=>line.includes("Node · Ready")))
  await controller.review("exclude")
  assert(saved)
  assert(controller.visibleLines().some(line=>line.includes("Node · Ready")),"Save discarded the kernel clock reference")
 }finally{controller?.close();Date.now=realNow}
})

// MP-08 / MP-10 / MP-11: users understand the terminal's editing boundary.
test("ENV-UX-02 TUI explains Environment and available editing",async()=>{
 const environment:any={schema_version:1,local_project_id:"project",lineage:{environment_id:"env",project_id:"p"},revision:0,content_digest:"d",project_requirements:[],folders:[],proposals:[],operations:[],observations:[],delivered_capabilities:{enabled_environment_operations:["get","save","check"],supported_schema:1}}
 const controller=createProjectEnvironmentReadController({send:async()=>(environmentReadResponse(environment)),render(){},pageSize:()=>100, textWidth: () => 72})
 await controller.open("project"); const text = controller.visibleLines().join(" "); assert(text.includes("An Environment lists what this project needs on this machine.")); assert(text.includes("Edit requirements in Web; this terminal supports Check and proposal review."))
})

// MP-08 / MP-10 / MP-11: keep transport diagnostics out of the user's empty state.
test("ENV-UX-01 missing Project TUI is a clear empty state",async()=>{
 const controller=createProjectEnvironmentReadController({send:async()=>{throw new Error("local transport `project.get` failed: project `missing` was not found (code=local_transport_error retryable=true)")},render(){},pageSize:()=>100, textWidth: () => 72});await controller.open("missing");assert(controller.visibleLines().includes("No Environment for this project yet."));assert(!controller.visibleLines().some(line=>line.includes("local transport")))
})
