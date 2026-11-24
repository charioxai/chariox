// The requests the client builds equal the fixture the kernel deserializes (`protocol_shapes/multi_project_context.rs`),
// so a field the kernel retired cannot linger in a builder.
import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"
import type { ContextDevelopmentSetup } from "./context-transfer-types.js"
import { createSliceRequest } from "./ipc-slice-requests.js"
import { startOwnerManagedContextTransferRequest, type ManagedContextLaunchTarget } from "./ipc-managed-context-requests.js"

type Shapes = { createSlice: unknown; ownerManagedTransfer: unknown; launchTarget: ManagedContextLaunchTarget; retiredRequests: unknown[] }

const shapes = async () =>
  JSON.parse(await readFile(new URL("../../../fixtures/multi-project-context-protocol.json", import.meta.url), "utf8")) as Shapes

test("a slice request selects its one Project as source_projects", async () => {
  const developmentSetup: ContextDevelopmentSetup = {
    kind: "source_projects",
    projects: [{ projectId: "project-docs", repositories: [{ role: "primary", workspaceId: "/work/docs", worktreeId: null }] }],
  }
  assert.deepEqual(createSliceRequest({ name: "docs", developmentSetup }), (await shapes()).createSlice)
})

test("a slice cannot select two Projects", () => {
  const project = { projectId: "p", repositories: [{ role: "primary" as const, workspaceId: "/w", worktreeId: null }] }
  assert.throws(
    () => createSliceRequest({ name: "docs", developmentSetup: { kind: "source_projects", projects: [project, { ...project, projectId: "q" }] } }),
    /exactly one Project/,
  )
})

test("an owner-managed transfer carries every ticked Project", async () => {
  const request = startOwnerManagedContextTransferRequest({
    target: { relayRealmId: "realm", machineId: "machine", kernelId: "kernel", relayPublicKey: "public", keyThumbprint: "pin" },
    contextSelection: {
      kernelContext: "source_kernel_without_credentials",
      developmentSetup: {
        kind: "source_projects",
        projects: [
          {
            projectId: "project-app",
            repositories: [
              { role: "primary", workspaceId: "/work/app", worktreeId: null },
              { role: "supporting", workspaceId: "/work/lib", worktreeId: "worktree-lib" },
            ],
          },
          { projectId: "project-docs", repositories: [{ role: "primary", workspaceId: "/work/docs", worktreeId: null }] },
        ],
      },
    },
  })
  assert.deepEqual(request, (await shapes()).ownerManagedTransfer)
})

test("the launch target lists one entry per Project", async () => {
  const { development } = (await shapes()).launchTarget
  assert.equal(development.kind, "from_source")
  if (development.kind !== "from_source") return
  assert.deepEqual(development.projects.map((project) => project.projectId), ["copy-context-1-0", "copy-context-1-1"])
})
