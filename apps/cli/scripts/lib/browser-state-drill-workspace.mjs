import assert from "node:assert/strict"
import path from "node:path"

import {
  assertRoomRootlessWorkspaceFixture,
  assertRoomRootlessWorkspaceFixtureRemoved,
  createRoomDirectDockerWorkspaceFixture,
  managedRoomFixtureSliceRoot,
  removeRoomDirectDockerWorkspaceFixture,
} from "./room-rootless-workspace-fixture.mjs"

export const browserStateDirectWorkspaceRootEnvironment = "M20_WORKSPACE_ROOT"

export async function prepareBrowserStateDrillWorkspace({
  env,
  repositoryRoot,
  homeRoot,
  verifyEngineAccess,
}) {
  const rawDirectRoot = env[browserStateDirectWorkspaceRootEnvironment]
  const directRoot = rawDirectRoot?.trim()
  const brokerSocket = env.CHARIOX_SLICE_DOCKER_BROKER_SOCKET?.trim()
  if (rawDirectRoot !== undefined && rawDirectRoot !== directRoot) {
    throw new Error(`${browserStateDirectWorkspaceRootEnvironment} must not contain surrounding whitespace`)
  }
  if (directRoot && brokerSocket) {
    throw new Error(`${browserStateDirectWorkspaceRootEnvironment} cannot be combined with a slice Docker broker`)
  }
  if (directRoot) {
    const lease = await createRoomDirectDockerWorkspaceFixture({
      workspaceRoot: directRoot,
      forbiddenRoots: [repositoryRoot, homeRoot],
      verifyEngineAccess,
    })
    return { kind: "direct", lease, workspace: lease.workspace }
  }
  if (brokerSocket) return { kind: "broker", lease: null, workspace: null }
  throw new Error(
    `browser persistence drill requires an actual slice Docker broker or explicit ${browserStateDirectWorkspaceRootEnvironment}`,
  )
}

export function browserStateDrillWorkspaceSliceOptions(fixture) {
  assert.ok(fixture && typeof fixture === "object", "browser-state workspace fixture is required")
  if (fixture.kind === "direct") return { workspaceMount: fixture.workspace }
  assert.equal(fixture.kind, "broker", "unknown browser-state workspace fixture kind")
  return { developmentSetup: { kind: "empty" } }
}

export async function finalizeBrowserStateDrillWorkspace({
  fixture,
  slice,
  repositoryRoot,
  managedSliceRoot = managedRoomFixtureSliceRoot,
}) {
  if (fixture.kind === "direct") {
    assert.equal(slice.workspace_mount, fixture.workspace,
      "direct-Docker slice must retain the exact owned fixture workspace")
    return fixture
  }
  const lease = await assertRoomRootlessWorkspaceFixture({
    slice,
    allowedDevelopmentRoot: path.join(managedSliceRoot, "development"),
    repositoryRoot,
  })
  return { kind: "broker", lease, workspace: lease.workspace }
}

export async function cleanupBrowserStateDrillWorkspace(fixture) {
  assert.ok(fixture && typeof fixture === "object", "browser-state workspace fixture is required")
  if (fixture.kind === "direct") {
    await removeRoomDirectDockerWorkspaceFixture(fixture.lease)
    return
  }
  assert.equal(fixture.kind, "broker", "unknown browser-state workspace fixture kind")
  if (fixture.lease) await assertRoomRootlessWorkspaceFixtureRemoved(fixture.lease)
}
