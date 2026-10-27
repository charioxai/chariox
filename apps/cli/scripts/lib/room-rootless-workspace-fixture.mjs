import assert from "node:assert/strict"
import path from "node:path"
import { chmod, lstat, mkdtemp, realpath, rm } from "node:fs/promises"

export const managedRoomFixtureSliceRoot = "/var/lib/chariox-slice-share/slices"
export const roomDirectDockerWorkspaceRootEnvironment = "CHARIOX_ROOM_DRILL_FIXTURE_WORKSPACE_ROOT"

const directFixturePrefix = "room-workspace-"

export async function createRoomDirectDockerWorkspaceFixture({
  workspaceRoot,
  forbiddenRoots,
  verifyEngineAccess,
}) {
  assertAbsolutePath(workspaceRoot, "direct-Docker workspace root")
  assert.ok(Array.isArray(forbiddenRoots) && forbiddenRoots.length > 0,
    "direct-Docker fixture forbidden roots are required")
  assert.equal(typeof verifyEngineAccess, "function", "rootless engine access probe is required")
  const canonicalRoot = await canonicalRealDirectory(workspaceRoot, "direct-Docker workspace root")
  const canonicalForbiddenRoots = await Promise.all(forbiddenRoots.map(async forbiddenRoot => {
    assertAbsolutePath(forbiddenRoot, "forbidden fixture root")
    return await realpath(forbiddenRoot)
  }))
  for (const forbiddenRoot of canonicalForbiddenRoots) {
    if (pathsOverlap(canonicalRoot, forbiddenRoot)) {
      throw new Error(`room fixture workspace root overlaps a private source or home root: ${canonicalRoot}`)
    }
  }
  await verifyEngineAccess(canonicalRoot, { writable: false })

  let workspace
  try {
    workspace = await mkdtemp(path.join(canonicalRoot, directFixturePrefix))
    await chmod(workspace, 0o777)
    const identity = await directoryIdentity(workspace)
    const processUid = process.getuid?.()
    if (Number.isSafeInteger(processUid) && identity.ownerUid !== processUid) {
      throw new Error(`room fixture workspace is not owned by the drill user: ${workspace}`)
    }
    await verifyEngineAccess(workspace, { writable: true })
    return Object.freeze({
      kind: "direct",
      workspace,
      workspaceRoot: canonicalRoot,
      device: identity.device,
      inode: identity.inode,
      ownerUid: identity.ownerUid,
    })
  } catch (error) {
    if (workspace) await rm(workspace, { recursive: true, force: true }).catch(() => undefined)
    throw error
  }
}

export async function removeRoomDirectDockerWorkspaceFixture(fixture) {
  validateDirectFixtureShape(fixture)
  const resolvedWorkspace = path.resolve(fixture.workspace)
  if (path.dirname(resolvedWorkspace) !== fixture.workspaceRoot
      || !path.basename(resolvedWorkspace).startsWith(directFixturePrefix)) {
    throw new Error(`refusing to remove unowned room fixture workspace: ${resolvedWorkspace}`)
  }
  let identity
  try {
    identity = await directoryIdentity(resolvedWorkspace)
  } catch (error) {
    if (error?.code === "ENOENT") return
    throw error
  }
  if (identity.device !== fixture.device || identity.inode !== fixture.inode
      || identity.ownerUid !== fixture.ownerUid) {
    throw new Error(`refusing to remove replaced room fixture workspace: ${resolvedWorkspace}`)
  }
  await rm(resolvedWorkspace, { recursive: true })
  await assertPathRemoved(resolvedWorkspace, "direct-Docker room fixture residue remains")
}

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
    kind: "broker",
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
  validateBrokerFixtureShape(fixture)
  for (const ownedPath of [fixture.workspace, fixture.storageRoot]) {
    await assertPathRemoved(ownedPath, "kernel-owned room fixture residue remains after slice deletion")
  }
}

function validateDirectFixtureShape(fixture) {
  assert.ok(fixture && typeof fixture === "object", "room fixture ownership record is required")
  assert.equal(fixture.kind, "direct", "direct-Docker room fixture ownership record is required")
  assertAbsolutePath(fixture.workspace, "fixture workspace")
  assertAbsolutePath(fixture.workspaceRoot, "fixture workspace root")
  assert.ok(Number.isSafeInteger(fixture.device), "fixture device identity is required")
  assert.ok(Number.isSafeInteger(fixture.inode), "fixture inode identity is required")
  assert.ok(Number.isSafeInteger(fixture.ownerUid), "fixture owner identity is required")
}

function validateBrokerFixtureShape(fixture) {
  assert.ok(fixture && typeof fixture === "object", "room fixture ownership record is required")
  assert.equal(fixture.kind, "broker", "broker room fixture ownership record is required")
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

async function canonicalRealDirectory(directory, label) {
  const resolved = path.resolve(directory)
  await directoryIdentity(resolved)
  const canonical = await realpath(resolved)
  if (canonical !== resolved) throw new Error(`${label} must not contain symlinks: ${resolved}`)
  return canonical
}

async function assertPathRemoved(target, message) {
  try {
    await lstat(target)
  } catch (error) {
    if (error?.code === "ENOENT") return
    throw error
  }
  throw new Error(`${message}: ${target}`)
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

function pathsOverlap(left, right) {
  return isPathWithin(left, right) || isPathWithin(right, left)
}
