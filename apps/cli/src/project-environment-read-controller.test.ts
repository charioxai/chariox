// MP-08 / MP-10: no-agent request and late-response cancellation.
import assert from "node:assert/strict"
import test from "node:test"
import { createProjectEnvironmentReadController } from "./project-environment-read-controller.js"
import { requireKernelFeatureProtocol } from "./kernel-feature-minimum.js"

test("ENV P01 standalone TUI request and protocol gate", async () => {
  const requests: unknown[] = []
  const controller = createProjectEnvironmentReadController({ send: async request => { requests.push(request); throw new Error("not available") }, render() {}, pageSize: () => 10 })
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
  const controller = createProjectEnvironmentReadController({ send: async request => { requests.push(request); return "RelayStatus" in (request as any) ? { RelayStatus: { status: { machine_id: "machine", daemon_id: "kernel" } } } : { ProjectEnvironment: { environment } } }, render() {}, pageSize: () => 100 })
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
  const controller = createProjectEnvironmentReadController({ send: async request => { requests.push(request); return "SaveProjectEnvironmentRevision" in (request as any) ? { ProjectEnvironmentSaved: { environment: { ...environment, revision: 4, proposals: [] } } } : { ProjectEnvironment: { environment } } }, render() {}, pageSize: () => 100 })
  await controller.open("project")
  controller.handleKey({ name: "a" })
  await new Promise(resolve => setTimeout(resolve, 0))
  assert.equal(requests.length, 2)
  assert.deepEqual(requests[1].SaveProjectEnvironmentRevision.acceptedProposalIds, ["proposal"])
  assert.equal(requests[1].SaveProjectEnvironmentRevision.expectedRevision, 3)
  assert.deepEqual(requests[1].SaveProjectEnvironmentRevision.excludedProposalIds, [])
  assert(controller.visibleLines().some(line => line.includes("Revision 4")))
})
