import assert from "node:assert/strict"
import test from "node:test"
import os from "node:os"
import path from "node:path"
import {
  access,
  chmod,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  realpath,
  rename,
  rm,
  stat,
  symlink,
  writeFile,
} from "node:fs/promises"

import {
  assertRoomRootlessWorkspaceFixture,
  assertRoomRootlessWorkspaceFixtureRemoved,
  createRoomDirectDockerWorkspaceFixture,
  removeRoomDirectDockerWorkspaceFixture,
} from "./room-rootless-workspace-fixture.mjs"

async function fixtureTree(t) {
  const root = await mkdtemp(path.join(await realpath(os.tmpdir()), "chariox-room-workspace-test-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  const repositoryRoot = path.join(root, "repository")
  const developmentRoot = path.join(root, "engine-visible", "slices", "development")
  const sliceId = "slice-test"
  const storageRoot = path.join(developmentRoot, sliceId)
  const destinationRoot = path.join(storageRoot, "development")
  const workspace = path.join(destinationRoot, "workspace")
  await mkdir(repositoryRoot)
  await mkdir(workspace, { recursive: true })
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
  return { root, repositoryRoot, developmentRoot, storageRoot, workspace, slice }
}

async function directFixtureTree(t) {
  const tree = await fixtureTree(t)
  const workspaceRoot = path.join(tree.root, "engine-visible-direct")
  const privateHome = path.join(tree.root, "private-home")
  await mkdir(workspaceRoot)
  await mkdir(privateHome)
  await chmod(workspaceRoot, 0o750)
  return { ...tree, workspaceRoot, privateHome }
}

test("direct mode probes the engine, changes only its empty owned child, and removes exact residue", async (t) => {
  const tree = await directFixtureTree(t)
  const probes = []
  const fixture = await createRoomDirectDockerWorkspaceFixture({
    workspaceRoot: tree.workspaceRoot,
    forbiddenRoots: [tree.repositoryRoot, tree.privateHome],
    verifyEngineAccess: async (target, options) => probes.push({ target, ...options }),
  })

  assert.equal(fixture.kind, "direct")
  assert.equal(path.dirname(fixture.workspace), tree.workspaceRoot)
  assert.deepEqual(probes, [
    { target: tree.workspaceRoot, writable: false },
    { target: fixture.workspace, writable: true },
  ])
  assert.deepEqual(await readdir(fixture.workspace), [])
  assert.equal((await stat(tree.workspaceRoot)).mode & 0o777, 0o750)
  assert.equal((await stat(fixture.workspace)).mode & 0o777, 0o777)
  assert.equal(fixture.ownerUid, process.getuid?.())

  await writeFile(path.join(fixture.workspace, "producer-residue"), "disposable")
  await removeRoomDirectDockerWorkspaceFixture(fixture)
  await assert.rejects(access(fixture.workspace), error => error?.code === "ENOENT")
  await access(tree.workspaceRoot)
})

test("direct mode refuses private and symlinked roots before the engine probe", async (t) => {
  const tree = await directFixtureTree(t)
  let probes = 0
  const verifyEngineAccess = async () => { probes += 1 }
  await assert.rejects(createRoomDirectDockerWorkspaceFixture({
    workspaceRoot: tree.privateHome,
    forbiddenRoots: [tree.repositoryRoot, tree.privateHome],
    verifyEngineAccess,
  }), /overlaps a private source or home root/)

  const linkedRoot = path.join(tree.root, "linked-engine-root")
  await symlink(tree.workspaceRoot, linkedRoot)
  await assert.rejects(createRoomDirectDockerWorkspaceFixture({
    workspaceRoot: linkedRoot,
    forbiddenRoots: [tree.repositoryRoot, tree.privateHome],
    verifyEngineAccess,
  }), /must be a real directory|must not contain symlinks/)
  assert.equal(probes, 0)
})

test("direct cleanup refuses a same-name replacement and preserves both directories", async (t) => {
  const tree = await directFixtureTree(t)
  const fixture = await createRoomDirectDockerWorkspaceFixture({
    workspaceRoot: tree.workspaceRoot,
    forbiddenRoots: [tree.repositoryRoot, tree.privateHome],
    verifyEngineAccess: async () => undefined,
  })
  const movedWorkspace = path.join(tree.root, "moved-owned-workspace")
  await rename(fixture.workspace, movedWorkspace)
  await mkdir(fixture.workspace)
  await writeFile(path.join(fixture.workspace, "replacement"), "keep")

  await assert.rejects(removeRoomDirectDockerWorkspaceFixture(fixture), /replaced room fixture workspace/)
  assert.equal(await readFile(path.join(fixture.workspace, "replacement"), "utf8"), "keep")
  await access(movedWorkspace)
})

test("accepts the exact kernel-owned empty publication outside the repository", async (t) => {
  const tree = await fixtureTree(t)
  const fixture = await assertRoomRootlessWorkspaceFixture({
    slice: tree.slice,
    allowedDevelopmentRoot: tree.developmentRoot,
    repositoryRoot: tree.repositoryRoot,
  })

  assert.equal(fixture.sliceId, tree.slice.id)
  assert.equal(fixture.storageRoot, tree.storageRoot)
  assert.equal(fixture.workspace, tree.workspace)
  assert.ok(path.relative(tree.repositoryRoot, fixture.workspace).startsWith(".."))

  await writeFile(path.join(tree.workspace, "producer-residue"), "disposable")
  await rm(tree.storageRoot, { recursive: true })
  await assertRoomRootlessWorkspaceFixtureRemoved(fixture)
  await assert.rejects(access(tree.storageRoot), error => error?.code === "ENOENT")
})

test("refuses an empty publication rooted inside the source repository", async (t) => {
  const tree = await fixtureTree(t)
  const unsafeDevelopmentRoot = path.join(tree.repositoryRoot, "slices", "development")
  const unsafeStorageRoot = path.join(unsafeDevelopmentRoot, tree.slice.id)
  const unsafeDestinationRoot = path.join(unsafeStorageRoot, "development")
  const unsafeWorkspace = path.join(unsafeDestinationRoot, "workspace")
  await mkdir(unsafeWorkspace, { recursive: true })
  const unsafeSlice = {
    ...tree.slice,
    development_storage_root: unsafeStorageRoot,
    workspace_mount: unsafeWorkspace,
    development_publication: {
      publicationId: "development",
      destinationRoot: unsafeDestinationRoot,
      primaryRepositoryPath: unsafeWorkspace,
      repositoryPaths: [unsafeWorkspace],
    },
  }

  await assert.rejects(assertRoomRootlessWorkspaceFixture({
    slice: unsafeSlice,
    allowedDevelopmentRoot: unsafeDevelopmentRoot,
    repositoryRoot: tree.repositoryRoot,
  }), /must be outside the repository/)
  await access(unsafeWorkspace)
})

test("refuses a publication under the wrong engine-visible root", async (t) => {
  const tree = await fixtureTree(t)
  const approvedRoot = path.join(tree.root, "approved", "slices", "development")
  await mkdir(approvedRoot, { recursive: true })

  await assert.rejects(assertRoomRootlessWorkspaceFixture({
    slice: tree.slice,
    allowedDevelopmentRoot: approvedRoot,
    repositoryRoot: tree.repositoryRoot,
  }), /direct child of the approved development root/)
  await access(tree.workspace)
})

test("refuses mismatched publication ownership and reports retained residue", async (t) => {
  const tree = await fixtureTree(t)
  const foreignWorkspace = path.join(tree.slice.development_publication.destinationRoot, "foreign")
  await mkdir(foreignWorkspace)
  await assert.rejects(assertRoomRootlessWorkspaceFixture({
    slice: {
      ...tree.slice,
      development_publication: {
        ...tree.slice.development_publication,
        repositoryPaths: [tree.workspace, foreignWorkspace],
      },
    },
    allowedDevelopmentRoot: tree.developmentRoot,
    repositoryRoot: tree.repositoryRoot,
  }), /must own only its workspace/)

  const fixture = await assertRoomRootlessWorkspaceFixture({
    slice: tree.slice,
    allowedDevelopmentRoot: tree.developmentRoot,
    repositoryRoot: tree.repositoryRoot,
  })
  await assert.rejects(assertRoomRootlessWorkspaceFixtureRemoved(fixture), /residue remains/)
  await access(tree.workspace)
})

test("room drill selects explicit direct mode or the configured broker contract", async () => {
  const source = await readFile(new URL("../live-room-environment-pointer-click-drill.mjs", import.meta.url), "utf8")
  assert.match(source, /developmentSetup: \{ kind: "empty" \}/)
  assert.match(source, /CHARIOX_SLICE_DOCKER_BROKER_SOCKET/)
  assert.match(source, /roomDirectDockerWorkspaceRootEnvironment/)
  assert.match(source, /verifyDirectDockerAccess\(\{ target, writable, platform: process\.platform/)
  const deleteSlice = source.indexOf("requests.deleteSliceRequest(slice.id)")
  const stopProducers = source.indexOf("for (const child of children.toReversed()) await terminateChild(child)")
  const verifyRemoved = source.indexOf("await assertRoomRootlessWorkspaceFixtureRemoved(fixtureWorkspaceLease)")
  const removeDirect = source.indexOf("await removeRoomDirectDockerWorkspaceFixture(fixtureWorkspaceLease)")
  assert.ok(deleteSlice >= 0)
  assert.ok(stopProducers > deleteSlice)
  assert.ok(verifyRemoved > deleteSlice)
  assert.ok(removeDirect > stopProducers)
  assert.doesNotMatch(source, /chmod\(fixtureWorkspaceLease\.workspace/)
})
