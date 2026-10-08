// MP-07 / MP-11: isolated signed release upgrade/rollback transaction.
import { lstat, readlink, readdir, rename, rm, symlink, writeFile } from "node:fs/promises"
import { join } from "node:path"
import { command, regular } from "./remote.mjs"
const FORMAT = "chariox.ssh-machine-install.v1"
const fail = message => { throw new Error(message) }
const metadata = path => lstat(path).catch(error => error.code === "ENOENT" ? null : Promise.reject(error))
async function activate(root, digest) {
  const pending = join(root, ".current-new")
  if (await metadata(pending)) {
    const m = await lstat(pending)
    if (!m.isSymbolicLink() || !/^releases\/[a-f0-9]{64}$/.test(await readlink(pending))) fail("foreign activation scratch")
    await rm(pending)
  }
  await symlink(`releases/${digest.slice(7)}`, pending)
  await rename(pending, join(root, "current"))
}
async function replaceMarker(path, marker) {
  const pending = `${path}.new`
  if (await metadata(pending)) { await regular(pending); await rm(pending) }
  await writeFile(pending, JSON.stringify(marker), { mode: 0o600, flag: "wx" })
  await rename(pending, path)
}
export async function recoverUpgrade({ root, markerPath, marker, r, service, stage, verify, serviceManager }) {
      const journalPath = join(root, "upgrade.json")
      if (await metadata(journalPath)) {
        const journal = JSON.parse(await regular(journalPath))
        if (journal?.previous?.format !== FORMAT || journal.previous.installId !== r.installId || journal.previous.port !== r.port || journal.previous.service !== service || journal.previous.unitDigest !== marker.unitDigest || !/^sha256:[a-f0-9]{64}$/.test(journal.previous.releaseDigest) || !/^sha256:[a-f0-9]{64}$/.test(journal.target)) fail("invalid upgrade recovery journal")
        await verify(join(root, "releases", journal.previous.releaseDigest.slice(7)), journal.previous.releaseDigest, root, stage)
        await serviceManager(["disable", "--now", service])
        await activate(root, journal.previous.releaseDigest)
        await replaceMarker(markerPath, journal.previous)
        marker = journal.previous
        await serviceManager(["enable", "--now", service])
        await rm(journalPath)
      }
  return marker
}
export async function upgradeRelease({ root, markerPath, marker, r, image, manifest, stage, verify, serviceManager, kernelCommand, kernelPath, env, service }) {
      const invoke = kernelCommand ?? (async args => JSON.parse(await command(join(root, "current", kernelPath), args, true, undefined, env)))
      const previousIdentity = await invoke(["--owner-managed-ready"])
      if (!previousIdentity?.connected) fail("current kernel must be ready before upgrade")
      const journalPath = join(root, "upgrade.json")
      const destination = join(root, "releases", r.releaseDigest.slice(7))
      if (await metadata(destination)) await verify(destination, r.releaseDigest, root, stage)
      else await rename(image, destination)
      const previous = marker
      await writeFile(journalPath, JSON.stringify({ previous, target: r.releaseDigest }), { flag: "wx", mode: 0o600 })
      try {
        await serviceManager(["disable", "--now", service])
        await activate(root, r.releaseDigest)
        await serviceManager(["enable", "--now", service])
        // Upgrade/repair never re-enroll or replace mutable identity/state.
        const ready = await invoke(["--owner-managed-ready"])
        if (!ready?.connected || ["kernelId", "machineId", "userId", "publicKeyThumbprint"].some(key => ready[key] !== previousIdentity[key])) fail("upgraded kernel did not become relay-ready")
        await replaceMarker(markerPath, { ...previous, releaseDigest: r.releaseDigest, sourceCommit: manifest.sourceCommit, sourceTree: manifest.sourceTree })
        await rm(journalPath)
        return { installId: r.installId, status: "upgraded", releaseDigest: r.releaseDigest }
      } catch (error) {
        await serviceManager(["disable", "--now", service])
        await activate(root, previous.releaseDigest)
        await replaceMarker(markerPath, previous)
        await serviceManager(["enable", "--now", service])
        await rm(journalPath)
        throw error
      }

}
