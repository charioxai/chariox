import { randomUUID } from "node:crypto"
import { mkdirSync, existsSync } from "node:fs"
import { join } from "node:path"
import { readProtectedLayoutReceipt, writeProtectedLayoutReceipt } from "./protected-layout-store.mjs"
import { verifyPrivateHostDirectory } from "./protected-host-root.mjs"

function refuse() { throw new Error("Slice restore is incomplete; the previous home and retained identity are preserved") }
function identifier(value) { return typeof value === "string" && /^chariox-slice-[A-Za-z0-9_.:-]+$/.test(value) }
export function createHomeGenerationStore(root) {
  const directory = join(root, "home-transactions")
  function initialize() {
    verifyPrivateHostDirectory(root, process.getuid(), true)
    try { mkdirSync(directory, {mode: 0o700}) } catch (error) { if (error.code !== "EEXIST") throw error }
    verifyPrivateHostDirectory(directory, process.getuid())
  }
  function read(container) {
    if (!identifier(container)) refuse()
    initialize()
    return existsSync(join(directory, `${container}.json`)) ? readProtectedLayoutReceipt(directory, container) : null
  }
  function check(record, container, token, volume, digest) {
    if (!record || record.container !== container || record.token !== token
        || record.newHomeVolume !== volume || record.archiveDigest !== digest) refuse()
  }
  return {
    read,
    begin({container, oldHomeVolume, oldContainerId, archiveDigest, imageId, targetOrigin}) {
      if (!identifier(container) || !/^sha256:[a-f0-9]{64}$/.test(imageId)
          || !/^[a-f0-9]{64}$/.test(archiveDigest) || !identifier(oldHomeVolume)) refuse()
      const previous = read(container)
      if (previous && previous.phase !== "resolved") refuse() // Interruption requires explicit recovery, never reuse.
      const token = randomUUID().replaceAll("-", "")
      const record = {version: 1, sliceId: container, container, token,
        phase: "preparing", oldHomeVolume, oldContainerId, newHomeVolume: `${container}-home-g${token}`,
        archiveDigest, imageId, targetOrigin, retainedPreviousHomes: previous
          ? [...(previous.retainedPreviousHomes ?? []), previous.oldHomeVolume, previous.failedHomeVolume].filter(Boolean)
          : []}
      writeProtectedLayoutReceipt(directory, container, record)
      return record
    },
    complete({container, token, volume, digest}) {
      const record = read(container)
      check(record, container, token, volume, digest)
      if (record.phase !== "preparing") refuse()
      writeProtectedLayoutReceipt(directory, container, {...record, phase: "ready"})
    },
    requireReady({container, token, volume, digest}) {
      const record = read(container)
      check(record, container, token, volume, digest)
      if (!["ready", "rollback-ready"].includes(record.phase)) refuse()
      return record
    },
    publish({container, token, volume, digest}) {
      const record = this.requireReady({container, token, volume, digest})
      // Keep the old reference until kernel-owned durable publication resolves
      // the restore transaction. This store never deletes home/private data.
      writeProtectedLayoutReceipt(directory, container, {...record, phase: "published"})
    },
    prepareRollback({container, archiveDigest, imageId, origin}) {
      const record = read(container)
      if (!record || record.phase === "resolved" || origin?.container !== container
          || origin.homeVolume !== record.oldHomeVolume || origin.containerId !== record.oldContainerId
          || origin.digest !== archiveDigest || !/^sha256:[a-f0-9]{64}$/.test(imageId)) refuse()
      const restored = {...record, phase: "rollback-ready", failedHomeVolume: record.newHomeVolume,
        newHomeVolume: record.oldHomeVolume, archiveDigest, imageId, targetOrigin: origin}
      writeProtectedLayoutReceipt(directory, container, restored)
      return restored
    },
    resolve(container, activeHomeVolume) {
      const record = read(container)
      if (!record) return
      if (record.phase !== "published" || record.newHomeVolume !== activeHomeVolume) refuse()
      // Preserve public ownership/history and all data references. Actual old
      // volume retirement is a separate operation after durable publication.
      writeProtectedLayoutReceipt(directory, container, {...record, phase: "resolved"})
    },
  }
}

if (process.argv[1]?.endsWith("/protected-home-generation.mjs")) {
  try {
    if (process.getuid() !== 0 || process.argv[2] !== "--require-ready") refuse()
    createHomeGenerationStore("/var/lib/chariox-docker/private-layout").requireReady({
      container: process.env.CHARIOX_SLICE_NAME,
      token: process.env.CHARIOX_SLICE_RESTORE_GENERATION,
      volume: process.env.CHARIOX_SLICE_HOME_VOLUME,
      digest: process.env.CHARIOX_SLICE_RESTORE_DIGEST,
    })
  } catch {
    console.error("Slice restore is incomplete; the previous home and retained identity are preserved")
    process.exitCode = 1
  }
}
