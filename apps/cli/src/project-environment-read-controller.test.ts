// MP-08 / MP-10: no-agent request and late-response cancellation.
import assert from "node:assert/strict"
import test from "node:test"
import { createProjectEnvironmentReadController } from "./project-environment-read-controller.js"
import { requireKernelFeatureProtocol } from "./kernel-feature-minimum.js"

test("ENV P01 standalone TUI request and protocol gate", async () => {
  const requests: unknown[] = []
  const controller = createProjectEnvironmentReadController({ send: async request => { requests.push(request); throw new Error("not available") }, render() {}, pageSize: () => 10, textWidth: () => 72 })
  await controller.open("project")
  assert.deepEqual(requests, [{ GetProjectEnvironment: { projectId: "project" } }])
  assert.equal(controller.isOpen(), true)
  assert(controller.visibleLines().join(" ").includes("Environment unavailable. Check the kernel connection and reopen this panel."))
  assert.throws(() => requireKernelFeatureProtocol(requests[0], 472), /487/)
  assert.doesNotThrow(() => requireKernelFeatureProtocol(requests[0], 487))
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
  assert(controller.visibleLines().some(line => line.startsWith("After the next release")))
})

// MP-08 / MP-10 / MP-11: the explanation wraps to the panel width and every wrapped row stays reachable.
test("ENV-UX-02 TUI explanation wraps to the panel width without losing text", async () => {
  const controller = createProjectEnvironmentReadController({ send: async () => { throw new Error("not available") }, render() {}, pageSize: () => 100, textWidth: () => 30 })
  await controller.open("project")
  const rows = controller.visibleLines()
  assert(rows.every(row => row.length <= 30), rows.filter(row => row.length > 30).join("|"))
  const text = rows.join(" ").replace(/\s+/g, " ")
  assert(text.includes("cannot check or change them here."))
  assert(text.includes("After the next release, you can add requirements and check this machine here."))
})

test("ENV-UX-02 TUI scrolls wrapped rows, not source lines", async () => {
  const controller = createProjectEnvironmentReadController({ send: async () => { throw new Error("not available") }, render() {}, pageSize: () => 3, textWidth: () => 20 })
  await controller.open("project")
  for (let index = 0; index < 40; index++) controller.handleKey({ name: "down" })
  assert(controller.visibleLines().join(" ").endsWith("reopen this panel."), controller.visibleLines().join("|"))
  assert(controller.visibleLines().every(row => row.length <= 20))
})

// MP-08 / MP-10: P02a Detect must go through the real kernel request surface.
test("ENV P02a TUI Detect is read-only, uses kernel folders and shows evidence skips", async () => {
  const requests: any[] = []
  const environment: any = { schema_version: 1, local_project_id: "project", lineage: { environment_id: "env", project_id: "lineage" }, folders: [], project_requirements: [], proposals: [], operations: [], delivered_capabilities: { enabled_environment_operations: ["get", "detect"], supported_schema: 1 } }
  const controller = createProjectEnvironmentReadController({ send: async request => { requests.push(request); return "RelayStatus" in (request as any) ? { RelayStatus: { status: { machine_id: "machine", daemon_id: "kernel" } } } : { ProjectEnvironment: { environment } } }, render() {}, pageSize: () => 100, textWidth: () => 72 })
  await controller.open("project")
  controller.handleKey({ name: "d" })
  await new Promise(resolve => setTimeout(resolve, 0))
  assert.equal(requests.length, 3)
  assert.equal(requests[2].DetectProjectEnvironment.projectId, "project")
  assert.deepEqual(requests[2].DetectProjectEnvironment.allowModelFolders, [])
  assert.equal(requests.some(r => r.SaveProjectEnvironmentRevision), false)
})

// MP-08 / MP-10 / MP-11: real shaped relay RED showed ~893ms on reopening.
test("Environment reopens its last kernel view while a fresh Get is pending", async () => {
  const environment: any = { schema_version: 1, local_project_id: "project", lineage: {}, folders: [], project_requirements: [], proposals: [], observations: [], operations: [], delivered_capabilities: { enabled_environment_operations: ["get"] } }
  let count = 0, release: (value: unknown) => void = () => {}
  const controller = createProjectEnvironmentReadController({ send: () => ++count === 1 ? Promise.resolve({ ProjectEnvironment: { environment } }) : new Promise(resolve => { release = resolve }), render() {}, pageSize: () => 30, textWidth: () => 72 })
  await controller.open("project"); controller.close()
  const pending = controller.open("project")
  try {
    assert(controller.visibleLines().some(line => line.includes("Project-wide")), "last kernel snapshot should render immediately")
    assert(controller.visibleLines().some(line => line.includes("Refreshing")))
    assert.equal(count, 2, "every open still requests a fresh kernel snapshot")
  } finally { release({ ProjectEnvironment: { environment } }); await pending }
})
