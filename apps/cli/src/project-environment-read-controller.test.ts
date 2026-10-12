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
