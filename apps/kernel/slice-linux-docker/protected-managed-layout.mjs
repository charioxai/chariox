import { mkdirSync, chmodSync, existsSync, readdirSync, lstatSync } from "node:fs"
import { join } from "node:path"
import { preparePrivateHostRoot, verifyPrivateHostDirectory } from "./protected-host-root.mjs"
import { readProtectedLayoutReceipt, writeProtectedLayoutReceipt, requireRetainedRuntimeIdentity } from "./protected-layout-store.mjs"
import { requireManagedImageProof, requireManagedRuntimeHash } from "./protected-image-proof.mjs"
import { requireRuntimeProof } from "./protected-runtime-proof.mjs"
import { requireIdentityRetention } from "./protected-identity-retention.mjs"
import { PRIVATE_ROOT, verifyProtectedCaptureLayout } from "./protected-layout.mjs"
import { createHomeGenerationStore } from "./protected-home-generation.mjs"
import { createHash } from "node:crypto"

function identifier(value) { return typeof value === "string" && /^[A-Za-z0-9][A-Za-z0-9_.:-]{0,179}$/.test(value) }

function refuse() { throw new Error("Protected slice layout is unavailable; existing identity and saved state are preserved") }
function originKey(containerId, digest) { return createHash("sha256").update(`${containerId}\0${digest}`).digest("hex") }

export function findRetainedCaptureOrigin(directory, container, digest) {
  verifyPrivateHostDirectory(directory, process.getuid())
  const names = readdirSync(directory)
  if (names.length > 10_000) refuse()
  for (const name of names.sort()) {
    if (/^\.[a-f0-9]{64}\.[1-9][0-9]*\.pending$/.test(name)) {
      // An interrupted public receipt is not an origin. Retain it for recovery;
      // ignore only the exact protected writer's regular-file metadata.
      const pending = lstatSync(join(directory, name))
      if (!pending.isFile() || pending.isSymbolicLink() || pending.nlink !== 1
          || pending.uid !== process.getuid() || (pending.mode & 0o077) !== 0
          || pending.size > 64 * 1024) refuse()
      continue
    }
    if (!/^[a-f0-9]{64}\.json$/.test(name)) refuse()
    const origin = readProtectedLayoutReceipt(directory, name.slice(0, -5))
    if ((container === undefined || origin.container === container) && origin.digest === digest
        && name === `${originKey(origin.containerId, digest)}.json`) return origin
  }
  refuse()
}

// Discover identity file paths only; never read their credential contents. The
// kernel keeps its active-registration files in kernels/active, which is not an
// identity directory.
export function retainedIdentityPaths(privateHostRoot) {
  const kernels = join(privateHostRoot, "kernel/kernels")
  const identityPaths = readdirSync(kernels)
    .filter(name => /^[A-Za-z0-9][A-Za-z0-9_.:-]{0,179}$/.test(name) && name !== "registry.json" && name !== "active")
    .filter(name => lstatSync(join(kernels, name)).isDirectory())
    .map(name => `kernel/kernels/${name}/identity.json`)
  identityPaths.push("kernel/kernels/registry.json")
  return identityPaths
}

export function createManagedLayoutController({root, sourceDigest, docker, dataOwner}) {
  const controlOwner = process.getuid()
  const homeRoot = join(root, "homes")
  const receiptRoot = join(root, "receipts")
  const imageRoot = join(root, "images")
  const backupRoot = join(root, "backups")
  const generations = createHomeGenerationStore(root)
  const owner = () => typeof dataOwner === "function" ? dataOwner() : dataOwner
  const trusted = /^sha256:[a-f0-9]{64}$/.test(sourceDigest ?? "")
  function initialize() {
    if (!trusted) refuse()
    for (const path of [root, homeRoot, receiptRoot, imageRoot, backupRoot]) {
      const traversalOnly = path === root || path === homeRoot
      const mode = traversalOnly ? 0o711 : 0o700
      // mkdir applies the process umask (the broker unit's UMask=0007 turns 0711
      // into 0710), so give directories created here their exact mode.
      try { mkdirSync(path, {mode}); chmodSync(path, mode) } catch (error) { if (error.code !== "EEXIST") throw error }
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
  function captureOrigin(container, digest) {
    return findRetainedCaptureOrigin(join(root, "capture-origins"), container, digest)
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
    homeVolume(container) {
      const record = receipt(container)
      if (!record) return `${container}-home`
      retained(record)
      const pending = generations.read(container)
      const info = containerInfo(container)
      if (info && info.Id !== record.containerId) {
        if (!pending || !["ready", "rollback-ready", "published"].includes(pending.phase)
            || info.Image !== pending.imageId
            || info.Mounts?.find(mount => mount.Destination === "/home/slice")?.Name !== pending.newHomeVolume) refuse()
        return pending.newHomeVolume
      }
      return record.homeVolume
    },
    recordCapture(container, digest) {
      const record = receipt(container)
      if (!record || !/^[a-f0-9]{64}$/.test(digest)) refuse()
      const origins = join(root, "capture-origins")
      try { mkdirSync(origins, {mode: 0o700}) } catch (error) { if (error.code !== "EEXIST") throw error }
      verifyPrivateHostDirectory(origins, controlOwner)
      const key = originKey(record.containerId, digest)
      writeProtectedLayoutReceipt(origins, key, {version: 1, sliceId: key, container,
        containerId: record.containerId, homeVolume: record.homeVolume, digest})
    },
    resolveRestore(container, archiveDigest) {
      const record = receipt(container)
      if (!record) return
      const pending = generations.read(container)
      if (!pending || pending.archiveDigest !== archiveDigest || pending.newHomeVolume !== record.homeVolume) refuse()
      const origin = pending.targetOrigin
      if (!origin || (origin.container !== container && pending.oldContainerId)
          || origin.digest !== archiveDigest) refuse()
      const retainedOrigin = readProtectedLayoutReceipt(join(root, "capture-origins"), originKey(origin.containerId, archiveDigest))
      if (JSON.stringify(retainedOrigin) !== JSON.stringify(origin)) refuse()
      generations.resolve(container, record.homeVolume)
    },
    beginRestore(environment, archiveDigest, action) {
      if (!environment.CHARIOX_SLICE_PRIVATE_HOST_ROOT || !environment.CHARIOX_SLICE_SAVED_HOME_ARCHIVE) return
      const container = environment.CHARIOX_SLICE_NAME
      const existing = receipt(container)
      if (existing) generations.resolveInitialization(container, existing)
      if (action !== "restore-state" && existing) {
        environment.CHARIOX_SLICE_HOME_VOLUME = this.homeVolume(container)
        return
      }
      const image = docker(["image", "inspect", environment.CHARIOX_SLICE_DOCKER_IMAGE ?? "chariox-slice-linux:0.1.0"])
      if (image.status !== 0) refuse()
      const images = JSON.parse(image.stdout)
      if (!Array.isArray(images) || images.length !== 1) refuse()
      requireManagedRuntimeHash(imageRoot, sourceDigest, images[0].Id)
      const prior = generations.read(container)
      const pending = prior && prior.phase !== "resolved"
        ? generations.prepareRollback({container, archiveDigest, imageId: images[0].Id,
          origin: readProtectedLayoutReceipt(join(root, "capture-origins"), originKey(prior.oldContainerId, archiveDigest))})
        : generations.begin({container, oldHomeVolume: this.homeVolume(container), oldContainerId: existing?.containerId,
          archiveDigest, imageId: images[0].Id, targetOrigin: captureOrigin(existing ? container : undefined, archiveDigest)})
      environment.CHARIOX_SLICE_HOME_VOLUME = pending.newHomeVolume
      environment.CHARIOX_SLICE_RESTORE_GENERATION = pending.token
      environment.CHARIOX_SLICE_RESTORE_DIGEST = pending.archiveDigest
      environment.CHARIOX_SLICE_PREVIOUS_HOME_VOLUME = pending.oldHomeVolume
    },
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
      if (["provision", "restore-state"].includes(action) && environment.CHARIOX_SLICE_SAVED_HOME_ARCHIVE) {
        // Reject an unknown/missing lineage before creating roots or allowing
        // the provisioner to replace a container/volume. Never fall back to a
        // standard image and discard installed software implicitly.
        const inspected = docker(["image", "inspect", environment.CHARIOX_SLICE_DOCKER_IMAGE ?? "chariox-slice-linux:0.1.0"])
        if (inspected.status !== 0) refuse()
        const images = JSON.parse(inspected.stdout)
        if (!Array.isArray(images) || images.length !== 1) refuse()
        requireManagedRuntimeHash(imageRoot, sourceDigest, images[0].Id)
      }
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
      const identityPaths = retainedIdentityPaths(privateHostRoot)
      const record = {version: 1, sliceId: environment.CHARIOX_SLICE_NAME,
        ownerSliceId: environment.CHARIOX_SLICE_ID, containerId: info.Id,
        imageId: info.Image, baseImageId: info.Image, privateHostRoot,
        homeVolume: environment.CHARIOX_SLICE_HOME_VOLUME ?? `${environment.CHARIOX_SLICE_NAME}-home`,
        homeSource: info.Mounts?.find(mount => mount.Destination === "/home/slice")?.Source,
        identityPaths, dataOwner: owner()}
      retained(record)
      verifyProtectedCaptureLayout(info, record, new Set([info.Image]))
      const kernelHash = requireManagedRuntimeHash(imageRoot, sourceDigest, info.Image)
      if (info.State?.Running === true && info.State?.Paused !== true) {
        requireRuntimeProof(docker, record.sliceId, kernelHash)
      } else {
        const runtime = readProtectedLayoutReceipt(join(root, "runtime-proofs"), record.sliceId)
        if (runtime.containerId !== info.Id || runtime.imageId !== info.Image
            || runtime.sourceDigest !== sourceDigest || runtime.kernelHash !== kernelHash) refuse()
      }
      record.protectedRuntimeHash = kernelHash
      if (environment.CHARIOX_SLICE_RESTORE_GENERATION) {
        generations.requireReady({container: record.sliceId, token: environment.CHARIOX_SLICE_RESTORE_GENERATION,
          volume: record.homeVolume, digest: environment.CHARIOX_SLICE_RESTORE_DIGEST})
      }
      writeProtectedLayoutReceipt(receiptRoot, record.sliceId, record)
      if (environment.CHARIOX_SLICE_RESTORE_GENERATION) {
        generations.publish({container: record.sliceId, token: environment.CHARIOX_SLICE_RESTORE_GENERATION,
          volume: record.homeVolume, digest: environment.CHARIOX_SLICE_RESTORE_DIGEST})
      }
      generations.resolveInitialization(record.sliceId, record)
    },
    requireQuiescedHome(container) {
      const record = receipt(container)
      if (!record) refuse()
      const info = containerInfo(container)
      if (!info || info.Id !== record.containerId
          || (info.State?.Running !== false && info.State?.Paused !== true)) refuse()
      const inventory = docker(["ps", "-a", "--format", "{{.Names}}"])
      if (inventory.status !== 0) refuse()
      for (const name of String(inventory.stdout).trim().split("\n").filter(Boolean)) {
        if (name === container) continue
        const result = docker(["container", "inspect", "--format", "{{json .Mounts}}", name])
        if (result.status !== 0) refuse()
        const mounts = JSON.parse(result.stdout)
        if (!Array.isArray(mounts) || mounts.some(mount =>
          mount.RW !== false && ((mount.Type === "volume" && mount.Name === record.homeVolume)
            || (typeof record.homeSource === "string" && typeof mount.Source === "string"
              && (mount.Source === record.homeSource || mount.Source.startsWith(`${record.homeSource}/`)
                || record.homeSource.startsWith(`${mount.Source}/`)))))) refuse()
      }
    },
    providerAuthProtected(container) {
      const record = receipt(container)
      if (record) { this.preflight(container); return true }
      const info = containerInfo(container)
      if (!info || info.Mounts?.some(mount => mount.Destination === PRIVATE_ROOT)
          || info.Config?.Env?.some(value => value.startsWith("CHARIOX_SLICE_PRIVATE_ROOT="))) refuse()
      return false
    },
    preflight(container) {
      const record = receipt(container)
      if (!record) refuse()
      retained(record)
      requireManagedImageProof(imageRoot, sourceDigest, record.baseImageId)
      const info = containerInfo(container)
      if (!info) refuse()
      const kernelHash = requireManagedRuntimeHash(imageRoot, sourceDigest, record.imageId)
      if (record.protectedRuntimeHash !== kernelHash) refuse()
      // Docker cannot exec in a paused/stopped source. Its reserved runtime was
      // verified before first boot and cannot be changed by the ordinary slice
      // user. The engine/root administrator remains trusted; no writer runs
      // while the low-level capture defense checks the quiesced source.
      if (info.State?.Running === true && info.State?.Paused !== true) {
        requireRuntimeProof(docker, container, kernelHash)
      }
      return verifyProtectedCaptureLayout(info, record, new Set([record.baseImageId]))
    },
  }
}
