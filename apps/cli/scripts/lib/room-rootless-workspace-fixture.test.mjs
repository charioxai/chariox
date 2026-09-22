import assert from "node:assert/strict"
import test from "node:test"
import os from "node:os"
import path from "node:path"
import { access, mkdir, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises"

import {
  assertRoomRootlessWorkspaceFixture,
  assertRoomRootlessWorkspaceFixtureRemoved,
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

test("room drill delegates creation and cleanup to the empty-development slice contract", async () => {
  const source = await readFile(new URL("../live-room-environment-pointer-click-drill.mjs", import.meta.url), "utf8")
  assert.match(source, /developmentSetup: \{ kind: "empty" \}/)
  const deleteSlice = source.indexOf("requests.deleteSliceRequest(slice.id)")
  const verifyRemoved = source.indexOf("await assertRoomRootlessWorkspaceFixtureRemoved(fixtureWorkspaceLease)")
  assert.ok(deleteSlice >= 0)
  assert.ok(verifyRemoved > deleteSlice)
  assert.doesNotMatch(source, /chmod\(fixtureWorkspaceLease\.workspace/)
})
