import { mkdirSync, existsSync, readdirSync, lstatSync } from "node:fs"
import { join } from "node:path"
import { preparePrivateHostRoot, verifyPrivateHostDirectory } from "./protected-host-root.mjs"
import { readProtectedLayoutReceipt, writeProtectedLayoutReceipt, requireRetainedRuntimeIdentity } from "./protected-layout-store.mjs"
import { requireManagedImageProof } from "./protected-image-proof.mjs"
import { requireIdentityRetention } from "./protected-identity-retention.mjs"
import { PRIVATE_ROOT, verifyProtectedCaptureLayout } from "./protected-layout.mjs"

function identifier(value) { return typeof value === "string" && /^[A-Za-z0-9][A-Za-z0-9_.:-]{0,179}$/.test(value) }

function refuse() { throw new Error("Protected slice layout is unavailable; existing identity and saved state are preserved") }

export function createManagedLayoutController({root, sourceDigest, docker, dataOwner}) {
  const controlOwner = process.getuid()
  const homeRoot = join(root, "homes")
  const receiptRoot = join(root, "receipts")
  const imageRoot = join(root, "images")
  const backupRoot = join(root, "backups")
  const owner = () => typeof dataOwner === "function" ? dataOwner() : dataOwner
  const trusted = /^sha256:[a-f0-9]{64}$/.test(sourceDigest ?? "")
  function initialize() {
    if (!trusted) refuse()
    for (const path of [root, homeRoot, receiptRoot, imageRoot, backupRoot]) {
      const traversalOnly = path === root || path === homeRoot
      try { mkdirSync(path, {mode: traversalOnly ? 0o711 : 0o700}) } catch (error) { if (error.code !== "EEXIST") throw error }
      verifyPrivateHostDirectory(path, controlOwner, traversalOnly)
    }
  }
  function receipt(container) {
    if (!identifier(container)) refuse()
    if (!trusted || !existsSync(receiptRoot)) return null
    verifyPrivateHostDirectory(receiptRoot, controlOwner)
    if (!existsSync(join(receiptRoot, `${container}.json`))) return null
    return readProtectedLayoutReceipt(receiptRoot, container)
  }
  function containerInfo(container) {
    const inventory = docker(["ps", "-a", "--format", "{{.Names}}"])
    if (inventory.status !== 0) refuse()
    if (!String(inventory.stdout).split("\n").includes(container)) return null
    const inspected = docker(["container", "inspect", container])
    if (inspected.status !== 0) refuse()
    const records = JSON.parse(inspected.stdout)
    if (!Array.isArray(records) || records.length !== 1) refuse()
    return records[0]
  }
  function retained(record) {
    const dataOwner = owner()
    if (record.dataOwner !== dataOwner || !identifier(record.ownerSliceId) || record.privateHostRoot !== join(homeRoot, record.ownerSliceId)) refuse()
    requireRetainedRuntimeIdentity(record.privateHostRoot, record.identityPaths, dataOwner)
    requireIdentityRetention({privateRoot: record.privateHostRoot, backupRoot, sliceId: record.ownerSliceId, dataOwner})
  }
  return {
    imageRoot,
    privateMounts(container) {
      const record = receipt(container)
      if (!record) return []
      retained(record)
      return [
        {destination: PRIVATE_ROOT, source: record.privateHostRoot, rw: true},
        {destination: "/home/slice/.local/share/pki/nssdb", source: `${record.privateHostRoot}/nssdb`, rw: true},
      ]
    },
    prepare(action, environment) {
      if (!trusted) return null // Legacy compatibility never enables capture.
      const container = environment.CHARIOX_SLICE_NAME
      if (!identifier(container) || !identifier(environment.CHARIOX_SLICE_ID)) refuse()
      const existing = receipt(container)
      if (existing) {
        if (existing.ownerSliceId !== environment.CHARIOX_SLICE_ID) refuse()
        retained(existing)
        return existing.privateHostRoot
      }
      if (!["provision", "restore-state"].includes(action)) return null
      const dataOwner = owner()
      if (!Number.isSafeInteger(dataOwner) || dataOwner < 0) refuse()
      // Existing mixed containers/homes are never retrofitted or migrated.
      const prior = containerInfo(container)
      if (prior) {
        if (prior.Mounts?.some(mount => mount.Destination === PRIVATE_ROOT)) refuse()
        return null
      }
      const volumes = docker(["volume", "ls", "--format", "{{.Name}}"])
      if (volumes.status !== 0) refuse()
      const home = environment.CHARIOX_SLICE_HOME_VOLUME ?? `${container}-home`
      if (String(volumes.stdout).split("\n").includes(home)) return null
      initialize()
      return preparePrivateHostRoot(homeRoot, environment.CHARIOX_SLICE_ID, controlOwner, true, dataOwner)
    },
    complete(environment) {
      if (!environment.CHARIOX_SLICE_PRIVATE_HOST_ROOT) return
      const info = containerInfo(environment.CHARIOX_SLICE_NAME)
      if (!info) refuse()
      requireManagedImageProof(imageRoot, sourceDigest, info.Image)
      const privateHostRoot = environment.CHARIOX_SLICE_PRIVATE_HOST_ROOT
      if (!identifier(environment.CHARIOX_SLICE_ID) || privateHostRoot !== join(homeRoot, environment.CHARIOX_SLICE_ID)) refuse()
      // Discover identity file paths only; never read their credential contents.
      const kernels = join(privateHostRoot, "kernel/kernels")
      const identityPaths = readdirSync(kernels).filter(name => /^[A-Za-z0-9][A-Za-z0-9_.:-]{0,179}$/.test(name) && name !== "registry.json")
        .filter(name => lstatSync(join(kernels, name)).isDirectory())
        .map(name => `kernel/kernels/${name}/identity.json`)
      identityPaths.push("kernel/kernels/registry.json")
      const record = {version: 1, sliceId: environment.CHARIOX_SLICE_NAME,
        ownerSliceId: environment.CHARIOX_SLICE_ID, containerId: info.Id,
        imageId: info.Image, baseImageId: info.Image, privateHostRoot,
        homeVolume: environment.CHARIOX_SLICE_HOME_VOLUME ?? `${environment.CHARIOX_SLICE_NAME}-home`,
        identityPaths, dataOwner: owner()}
      retained(record)
      verifyProtectedCaptureLayout(info, record, new Set([info.Image]))
      writeProtectedLayoutReceipt(receiptRoot, record.sliceId, record)
    },
    preflight(container) {
      const record = receipt(container)
      if (!record) refuse()
      retained(record)
      requireManagedImageProof(imageRoot, sourceDigest, record.baseImageId)
      const info = containerInfo(container)
      if (!info) refuse()
      return verifyProtectedCaptureLayout(info, record, new Set([record.baseImageId]))
    },
  }
}
