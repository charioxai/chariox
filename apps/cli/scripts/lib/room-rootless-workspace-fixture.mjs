import assert from "node:assert/strict"
import path from "node:path"
import { lstat, realpath } from "node:fs/promises"

export const managedRoomFixtureSliceRoot = "/var/lib/chariox-slice-share/slices"

export async function assertRoomRootlessWorkspaceFixture({
  slice,
  allowedDevelopmentRoot,
  repositoryRoot,
}) {
  assert.ok(slice && typeof slice === "object", "running fixture slice is required")
  assert.equal(slice.development?.kind, "empty", "room fixture must use kernel-owned empty development")
  assertAbsolutePath(allowedDevelopmentRoot, "allowed development root")
  assertAbsolutePath(repositoryRoot, "repository root")

  const storageRoot = requiredAbsolutePath(slice.development_storage_root, "development storage root")
  const publication = slice.development_publication
  assert.ok(publication && typeof publication === "object", "empty development publication is required")
  assert.equal(publication.publicationId, "development", "unexpected empty development publication")
  const destinationRoot = requiredAbsolutePath(publication.destinationRoot, "publication destination")
  const workspace = requiredAbsolutePath(publication.primaryRepositoryPath, "fixture workspace")
  assert.equal(slice.workspace_mount, workspace, "slice workspace must use the empty publication")
  assert.deepEqual(publication.repositoryPaths, [workspace], "empty publication must own only its workspace")

  const [
    canonicalAllowedRoot,
    canonicalRepositoryRoot,
    canonicalStorageRoot,
    canonicalDestinationRoot,
    canonicalWorkspace,
  ] = await Promise.all([
    realpath(allowedDevelopmentRoot),
    realpath(repositoryRoot),
    realpath(storageRoot),
    realpath(destinationRoot),
    realpath(workspace),
  ])
  assert.equal(path.dirname(canonicalStorageRoot), canonicalAllowedRoot,
    "room fixture storage must be a direct child of the approved development root")
  assert.equal(path.basename(canonicalStorageRoot), slice.id,
    "room fixture storage must remain bound to its slice identity")
  assert.equal(canonicalDestinationRoot, path.join(canonicalStorageRoot, "development"),
    "room fixture publication must use the kernel-owned development directory")
  assert.equal(canonicalWorkspace, path.join(canonicalDestinationRoot, "workspace"),
    "room fixture must use the publication's exact workspace")
  if (isPathWithin(canonicalWorkspace, canonicalRepositoryRoot)) {
    throw new Error(`room fixture workspace must be outside the repository: ${canonicalWorkspace}`)
  }

  const [storageIdentity, workspaceIdentity] = await Promise.all([
    directoryIdentity(canonicalStorageRoot),
    directoryIdentity(canonicalWorkspace),
  ])
  const processUid = process.getuid?.()
  if (Number.isSafeInteger(processUid) && storageIdentity.ownerUid !== processUid) {
    throw new Error(`room fixture storage is not owned by the kernel user: ${canonicalStorageRoot}`)
  }
  if (Number.isSafeInteger(processUid) && workspaceIdentity.ownerUid !== processUid) {
    throw new Error(`room fixture workspace is not owned by the kernel user: ${canonicalWorkspace}`)
  }

  return Object.freeze({
    sliceId: slice.id,
    storageRoot: canonicalStorageRoot,
    workspace: canonicalWorkspace,
    storageDevice: storageIdentity.device,
    storageInode: storageIdentity.inode,
    workspaceDevice: workspaceIdentity.device,
    workspaceInode: workspaceIdentity.inode,
  })
}

export async function assertRoomRootlessWorkspaceFixtureRemoved(fixture) {
  validateFixtureShape(fixture)
  for (const ownedPath of [fixture.workspace, fixture.storageRoot]) {
    try {
      await lstat(ownedPath)
    } catch (error) {
      if (error?.code === "ENOENT") continue
      throw error
    }
    throw new Error(`kernel-owned room fixture residue remains after slice deletion: ${ownedPath}`)
  }
}

function validateFixtureShape(fixture) {
  assert.ok(fixture && typeof fixture === "object", "room fixture ownership record is required")
  assert.equal(typeof fixture.sliceId, "string", "fixture slice identity is required")
  assertAbsolutePath(fixture.workspace, "fixture workspace")
  assertAbsolutePath(fixture.storageRoot, "fixture storage root")
}

async function directoryIdentity(directory) {
  const metadata = await lstat(directory)
  if (!metadata.isDirectory() || metadata.isSymbolicLink()) {
    throw new Error(`room fixture path must be a real directory: ${directory}`)
  }
  return { device: metadata.dev, inode: metadata.ino, ownerUid: metadata.uid }
}

function requiredAbsolutePath(value, label) {
  assertAbsolutePath(value, label)
  return path.resolve(value)
}

function assertAbsolutePath(value, label) {
  assert.equal(typeof value, "string", `${label} must be a string`)
  assert.ok(path.isAbsolute(value), `${label} must be absolute`)
}

function isPathWithin(candidate, parent) {
  const relative = path.relative(parent, candidate)
  return relative === "" || (!relative.startsWith(`..${path.sep}`) && relative !== ".." && !path.isAbsolute(relative))
}
