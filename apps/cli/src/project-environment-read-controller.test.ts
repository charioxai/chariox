// MP-08 / MP-10: no-agent request and late-response cancellation.
import assert from "node:assert/strict"
import test from "node:test"
import { createProjectEnvironmentReadController } from "./project-environment-read-controller.js"
import { requireKernelFeatureProtocol } from "./kernel-feature-minimum.js"

test("ENV P01 standalone TUI request and protocol gate", async () => {
  const requests: unknown[] = []
  const controller = createProjectEnvironmentReadController({ send: async request => { requests.push(request); throw new Error("not available") }, render() { }, pageSize: () => 10 })
  await controller.open("project")
  assert.deepEqual(requests, [{ GetProjectEnvironment: { projectId: "project" } }])
  assert.equal(controller.isOpen(), true)
  assert.deepEqual(controller.visibleLines(), ["not available"])
  assert.throws(() => requireKernelFeatureProtocol(requests[0], 470), /471/)
  assert.doesNotThrow(() => requireKernelFeatureProtocol(requests[0], 471))
  controller.handleKey({ name: "escape" })
  assert.equal(controller.isOpen(), false)
})

test("ENV P01 closed TUI panel ignores a late kernel response", async () => {
  let reject: (error: Error) => void = () => undefined
  let renders = 0
  const controller = createProjectEnvironmentReadController({ send: () => new Promise((_resolve, fail) => { reject = fail }), render() { renders++ }, pageSize: () => 10 })
  const opening = controller.open("project")
  controller.close()
  reject(new Error("late"))
  await opening
  assert.equal(controller.isOpen(), false)
  assert.equal(renders, 2)
})

// MP-08 / MP-10: P02a Detect must go through the real kernel request surface.
test("ENV P02a TUI Detect is read-only, uses kernel folders and shows evidence skips", async () => {
  const requests: any[] = []
  const environment: any = { schema_version: 1, local_project_id: "project", lineage: { environment_id: "env", project_id: "lineage" }, folders: [], project_requirements: [], proposals: [], operations: [], delivered_capabilities: { enabled_environment_operations: ["get", "detect"], supported_schema: 1 } }
  const controller = createProjectEnvironmentReadController({ send: async request => { requests.push(request); return "RelayStatus" in (request as any) ? { RelayStatus: { status: { machine_id: "machine", daemon_id: "kernel" } } } : { ProjectEnvironment: { environment } } }, render() { }, pageSize: () => 100 })
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
  const controller = createProjectEnvironmentReadController({ send: async request => { requests.push(request); return "SaveProjectEnvironmentRevision" in (request as any) ? { ProjectEnvironmentSaved: { environment: { ...environment, revision: 4, proposals: [] } } } : { ProjectEnvironment: { environment } } }, render() { }, pageSize: () => 100 })
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
  const requests: any[] = []
  const environment: any = { schema_version: 1, local_project_id: "project", revision: 0, content_digest: "base", folders: [], project_requirements: [], proposals: [{ proposal_id: "proposal", requirement: { requirement_id: "node", title: "Node", scope: { kind: "project" }, origins: [], spec: { kind: "software", identity: "node", version_constraint: null, detect_only: true } } }], operations: [], delivered_capabilities: { enabled_environment_operations: ["get", "save"], supported_schema: 1 } }
  const controller = createProjectEnvironmentReadController({ send: async request => { requests.push(request); return { ProjectEnvironment: { environment } } }, render() { }, pageSize: () => 100 })
  await controller.open("project")
  controller.handleKey({ name: "a", ctrl: true })
  await new Promise(resolve => setTimeout(resolve, 0))
  assert.equal(requests.length, 1, "modified keys must not save a review")
})

// MP-08 / MP-10 / MP-11: a review key must show the selected constraint and provenance first.
test("P02b selected proposal shows its constraint and origin in the visible review header", async () => {
  const environment: any = { schema_version: 1, local_project_id: "project", revision: 0, content_digest: "base", lineage: { environment_id: "env", project_id: "lineage" }, folders: [], project_requirements: [], proposals: [{ proposal_id: "proposal", requirement: { requirement_id: "react", title: "React", scope: { kind: "project" }, origins: [{ kind: "detected_metadata", source: "package.json", reference: "react dependency" }], spec: { kind: "software", identity: "react", version_constraint: "19.0.0", detect_only: true } } }], operations: [], delivered_capabilities: { enabled_environment_operations: ["get", "save"], supported_schema: 1 } }
  const controller = createProjectEnvironmentReadController({ send: async () => ({ ProjectEnvironment: { environment } }), render() { }, pageSize: () => 6 })
  await controller.open("project")
  assert(controller.visibleLines().some(line => line === "Version constraint: 19.0.0"))
  assert(controller.visibleLines().some(line => line.includes("package.json")))
})

// MP-08 / MP-10 / MP-11: real shaped relay RED showed ~893ms on reopening.
test("Environment reopens its last kernel view while a fresh Get is pending", async () => {
  const environment: any = { schema_version: 1, local_project_id: "project", lineage: {}, folders: [], project_requirements: [], proposals: [], observations: [], operations: [], delivered_capabilities: { enabled_environment_operations: ["get"] } }
  let count = 0, release: (value: unknown) => void = () => { }
  const controller = createProjectEnvironmentReadController({ send: () => ++count === 1 ? Promise.resolve({ ProjectEnvironment: { environment } }) : new Promise(resolve => { release = resolve }), render() { }, pageSize: () => 30 })
  await controller.open("project"); controller.close()
  const pending = controller.open("project")
  try {
    assert(controller.visibleLines().some(line => line.includes("Project-wide")), "last kernel snapshot should render immediately")
    assert(controller.visibleLines().some(line => line.includes("Refreshing")))
    assert.equal(count, 2, "every open still requests a fresh kernel snapshot")
  } finally { release({ ProjectEnvironment: { environment } }); await pending }
})

// MP-08 / MP-10 / MP-11: cached proposals are display-only until the current kernel read settles.
test("P02b cached review waits for refresh and caches successful Save", async () => {
  const environment: any = { schema_version: 1, local_project_id: "project", revision: 0, content_digest: "base", lineage: {}, folders: [], project_requirements: [], proposals: [{ proposal_id: "p", requirement: { requirement_id: "node", title: "Node", scope: { kind: "project" }, origins: [], spec: { kind: "software", identity: "node", version_constraint: "22", detect_only: true } } }], operations: [], delivered_capabilities: { enabled_environment_operations: ["get", "save"] } }
  let gets = 0, saves = 0, release: (value: unknown) => void = () => { }
  const controller = createProjectEnvironmentReadController({
    send: async request => {
      if ("SaveProjectEnvironmentRevision" in (request as any)) { saves++; return { ProjectEnvironmentSaved: { environment: { ...environment, revision: 1, proposals: [] } } } }
      if (++gets === 1) return { ProjectEnvironment: { environment } }
      return new Promise(resolve => { release = resolve })
    }, render() { }, pageSize: () => 100
  })
  await controller.open("project"); controller.close()
  const pending = controller.open("project")
  try {
    controller.handleKey({ name: "a" }); await new Promise(resolve => setTimeout(resolve, 0))
    assert.equal(saves, 0, "cached proposal must not authorize Save")
  } finally { release({ ProjectEnvironment: { environment } }); await pending }
  await controller.review("accept"); controller.close()
  const reopened = controller.open("project")
  try { assert(controller.visibleLines().some(line => line.includes("Revision 1")), "saved view replaces cached revision") }
  finally { release({ ProjectEnvironment: { environment: { ...environment, revision: 1, proposals: [] } } }); await reopened }
})

// MP-08 / MP-10 / MP-11: typed edits remain Web-first, with a real copyable Project link.
test("P03 TUI exposes a safe Web link and saved diff after review", async () => {
  const environment: any = { schema_version: 1, local_project_id: "project", revision: 0, content_digest: "base", folders: [], project_requirements: [], proposals: [], operations: [], delivered_capabilities: { enabled_environment_operations: ["get", "save"] } }
  let copied = ""
  const controller = createProjectEnvironmentReadController({ send: async r => "CloudRelayStatus" in (r as any) ? { CloudRelayStatus: { profile: { api_url: "https://owned.example/api" } } } : "RelayStatus" in (r as any) ? { RelayStatus: { status: { daemon_id: "owned-kernel" } } } : { ProjectEnvironment: { environment } }, render() { }, pageSize: () => 100, copyLink: link => { copied = link } })
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
  const controller = createProjectEnvironmentReadController({ send: async () => ({ ProjectEnvironment: { environment: reads++ ? current : previous } }), render() { }, pageSize: () => 100 })
  await controller.open("project"); await controller.open("project"); controller.handleKey({ name: "v" })
  assert(controller.visibleLines().some(line => line.startsWith("Observed revision changes")))
  assert(controller.visibleLines().includes("Version constraint: 22"))
  assert(controller.visibleLines().includes("Version constraint: 24"))
})
