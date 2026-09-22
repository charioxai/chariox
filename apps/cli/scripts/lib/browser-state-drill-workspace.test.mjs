import assert from "node:assert/strict"
import test from "node:test"
import os from "node:os"
import path from "node:path"
import {
  access,
  mkdir,
  mkdtemp,
  readFile,
  realpath,
  rename,
  rm,
  writeFile,
} from "node:fs/promises"

import {
  browserStateDirectWorkspaceRootEnvironment,
  browserStateDrillWorkspaceSliceOptions,
  cleanupBrowserStateDrillWorkspace,
  finalizeBrowserStateDrillWorkspace,
  prepareBrowserStateDrillWorkspace,
} from "./browser-state-drill-workspace.mjs"

async function scratch(t) {
  const root = await mkdtemp(path.join(await realpath(os.tmpdir()), "chariox-browser-state-workspace-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  const repositoryRoot = path.join(root, "repository")
  const homeRoot = path.join(root, "home")
  const workspaceRoot = path.join(root, "engine-visible")
  await Promise.all([repositoryRoot, homeRoot, workspaceRoot].map(directory => mkdir(directory)))
  return { root, repositoryRoot, homeRoot, workspaceRoot }
}

test("direct mode selects one probed outside-home workspace and cleans its exact child", async (t) => {
  const tree = await scratch(t)
  const probes = []
  const fixture = await prepareBrowserStateDrillWorkspace({
    env: { [browserStateDirectWorkspaceRootEnvironment]: tree.workspaceRoot },
    repositoryRoot: tree.repositoryRoot,
    homeRoot: tree.homeRoot,
    verifyEngineAccess: async (target, options) => probes.push({ target, ...options }),
  })
  assert.equal(fixture.kind, "direct")
  assert.deepEqual(browserStateDrillWorkspaceSliceOptions(fixture), { workspaceMount: fixture.workspace })
  assert.deepEqual(probes, [
    { target: tree.workspaceRoot, writable: false },
    { target: fixture.workspace, writable: true },
  ])
  assert.equal((await finalizeBrowserStateDrillWorkspace({
    fixture,
    slice: { workspace_mount: fixture.workspace },
    repositoryRoot: tree.repositoryRoot,
  })).workspace, fixture.workspace)

  await writeFile(path.join(fixture.workspace, "producer-residue"), "disposable")
  await cleanupBrowserStateDrillWorkspace(fixture)
  await assert.rejects(access(fixture.workspace), error => error?.code === "ENOENT")
  await access(tree.workspaceRoot)
})

test("direct wrapper propagates replacement refusal without deleting either directory", async (t) => {
  const tree = await scratch(t)
  const fixture = await prepareBrowserStateDrillWorkspace({
    env: { [browserStateDirectWorkspaceRootEnvironment]: tree.workspaceRoot },
    repositoryRoot: tree.repositoryRoot,
    homeRoot: tree.homeRoot,
    verifyEngineAccess: async () => undefined,
  })

  const moved = path.join(tree.root, "moved-owned-workspace")
  await rename(fixture.workspace, moved)
  await mkdir(fixture.workspace)
  await writeFile(path.join(fixture.workspace, "replacement"), "keep")
  await assert.rejects(cleanupBrowserStateDrillWorkspace(fixture), /replaced room fixture workspace/)
  assert.equal(await readFile(path.join(fixture.workspace, "replacement"), "utf8"), "keep")
  await access(moved)
})

test("broker mode retains the kernel empty-development publication and verifies cleanup", async (t) => {
  const tree = await scratch(t)
  const sliceId = "slice-browser-state"
  const developmentRoot = path.join(tree.root, "managed-slices", "development")
  const storageRoot = path.join(developmentRoot, sliceId)
  const destinationRoot = path.join(storageRoot, "development")
  const workspace = path.join(destinationRoot, "workspace")
  await mkdir(workspace, { recursive: true })
  const prepared = await prepareBrowserStateDrillWorkspace({
    env: { CHARIOX_SLICE_DOCKER_BROKER_SOCKET: "/run/test-broker.sock" },
    repositoryRoot: tree.repositoryRoot,
    homeRoot: tree.homeRoot,
    verifyEngineAccess: async () => { throw new Error("direct probe must not run") },
  })
  assert.deepEqual(browserStateDrillWorkspaceSliceOptions(prepared), { developmentSetup: { kind: "empty" } })

  const slice = {
    id: sliceId,
    development: { kind: "empty" },
    development_storage_root: storageRoot,
    development_publication: {
      publicationId: "development",
      destinationRoot,
      primaryRepositoryPath: workspace,
      repositoryPaths: [workspace],
    },
    workspace_mount: workspace,
  }
  const fixture = await finalizeBrowserStateDrillWorkspace({
    fixture: prepared,
    slice,
    repositoryRoot: tree.repositoryRoot,
    managedSliceRoot: path.dirname(developmentRoot),
  })
  assert.equal(fixture.kind, "broker")
  assert.equal(fixture.workspace, workspace)
  await rm(storageRoot, { recursive: true })
  await cleanupBrowserStateDrillWorkspace(fixture)
})

test("workspace mode is explicit and mutually exclusive", async (t) => {
  const tree = await scratch(t)
  const base = {
    repositoryRoot: tree.repositoryRoot,
    homeRoot: tree.homeRoot,
    verifyEngineAccess: async () => undefined,
  }
  await assert.rejects(prepareBrowserStateDrillWorkspace({ env: {}, ...base }), /requires an actual slice Docker broker/)
  await assert.rejects(prepareBrowserStateDrillWorkspace({
    env: {
      [browserStateDirectWorkspaceRootEnvironment]: tree.workspaceRoot,
      CHARIOX_SLICE_DOCKER_BROKER_SOCKET: "/run/test-broker.sock",
    },
    ...base,
  }), /cannot be combined/)
})

test("live persistence drill wires exact workspace selection without reducing persistence assertions", async () => {
  const source = await readFile(new URL("../live-docker-slice-browser-state-drill.mjs", import.meta.url), "utf8")
  assert.doesNotMatch(source, /workspaceMount:\s*repoRoot/)
  assert.match(source, /browserStateDrillWorkspaceSliceOptions\(workspaceFixture\)/)
  assert.match(source, /finalizeBrowserStateDrillWorkspace/)
  const stopProducers = source.indexOf("for (const child of children.toReversed())")
  const cleanupWorkspace = source.indexOf("await cleanupBrowserStateDrillWorkspace(workspaceFixture)")
  assert.ok(stopProducers >= 0)
  assert.ok(cleanupWorkspace > stopProducers)
  for (const retainedAssertion of [
    "stateCookie",
    "stateLocalStorage",
    "stateIndexedDb",
    "stateCacheStorage",
    "service-worker registration should persist",
    "browser download should survive saved-state restore",
    "restoring the same immutable backup again by id",
    "external service session",
  ]) assert.ok(source.includes(retainedAssertion), `missing persistence assertion: ${retainedAssertion}`)
})
