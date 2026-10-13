import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { createHash } from "node:crypto"
import { constants } from "node:fs"
import { lstat, mkdir, mkdtemp, open, realpath, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"

// Provider profiles and refreshed child homes survive every drill outcome.
// This root is product state, never a task scratch or evidence directory.
export async function createRetainedRoomRuntime({ home = os.homedir(), owner = process.getuid?.(), runId } = {}) {
  const base = path.join(home, ".chariox", "dev", "provider-runtimes", "browser-computer")
  for (const directory of [path.join(home, ".chariox"), path.join(home, ".chariox", "dev"), path.dirname(base), base]) {
    try { await mkdir(directory, { mode: 0o700 }) } catch (error) {
      if (error.code !== "EEXIST") throw error
    }
    const metadata = await lstat(directory)
    assert.ok(metadata.isDirectory() && !metadata.isSymbolicLink(), "retained provider root must be a real directory")
    if (owner !== undefined) assert.equal(metadata.uid, owner, "retained provider root has a different owner")
    // .chariox is an existing shared product parent; only the dedicated roots
    // must be private. Never chmod an existing shared parent as a side effect.
    if ([path.dirname(base), base].includes(directory)) {
      assert.equal(metadata.mode & 0o777, 0o700, "retained provider root must be mode0700")
    }
  }
  const root = await mkdtemp(path.join(base, "runtime-"))
  await writeFile(path.join(root, "RETENTION.json"), `${JSON.stringify({
    version: 1, purpose: "Browser and Computer official-provider acceptance",
    owner, runId, createdAt: new Date().toISOString(), cleanup: "retain",
    protected: ["provider profiles", "kernel state", "slice homes", "saved states", "private execution records"],
  }, null, 2)}\n`, { mode: 0o600, flag: "wx" })
  return root
}

// Keep the policy next to the calls so failure cleanup cannot accidentally
// replace StopSlice with DeleteSlice for a credential-bearing environment.
export async function stopOrDeleteRoomSlice({ retain, stop, remove }) {
  return retain ? stop() : remove()
}

export function roomCleanupComplete(cleanup, retain) {
  return cleanup.containerGone && cleanup.fixtureWorkspaceRemoved
    && cleanup.listenersReleased && !cleanup.plaintextSecretLeak
    && (cleanup.childCleanupFailures?.length ?? 0) === 0
    && (retain
      ? cleanup.providerStateRetained && cleanup.homeVolumeRetained
      : cleanup.volumeGone && cleanup.tempRootRemoved)
}

export function roomDrillLeakScanRoots({ stateRoot, evidenceRoot, publicEvidenceRoot, retain }) {
  return retain ? [
    path.join(stateRoot, "kernel-logs"), path.join(stateRoot, "history"),
    path.join(stateRoot, "provider-workspace"), evidenceRoot, publicEvidenceRoot,
  ] : [stateRoot, evidenceRoot]
}

// Validate the saved copy before a persistence drill removes its original
// volume. Member names and contents never enter logs or public receipts.
export async function verifyRetainedRoomArchive(state) {
  for (const name of ["home_archive_path", "manifest_path"]) {
    assert.equal(await realpath(state[name]), path.resolve(state[name]), "saved state path contains a symlink")
  }
  const manifestFile = await open(state.manifest_path, constants.O_RDONLY | constants.O_NOFOLLOW)
  let manifest
  try {
    const metadata = await manifestFile.stat()
    assert.ok(metadata.isFile() && metadata.size < 1024 * 1024, "saved manifest must be a bounded regular file")
    try { manifest = JSON.parse(await manifestFile.readFile("utf8")) } catch {
      throw new Error("saved state manifest is not valid JSON")
    }
  } finally { await manifestFile.close() }
  for (const key of ["id", "source_slice_id", "home_archive_path", "image_ref", "size_bytes"]) {
    assert.ok(manifest[key] === state[key], `saved state ${key} differs from the kernel result`)
  }
  const archive = await open(state.home_archive_path, constants.O_RDONLY | constants.O_NOFOLLOW)
  try {
    const before = await archive.stat()
    assert.ok(before.isFile() && before.nlink === 1 && before.size > 0, "saved archive must be a nonempty unlinked regular file")
    assert.equal(before.mode & 0o777, 0o600, "saved archive must remain private")
    assert.equal(before.size, state.size_bytes, "saved archive size differs from the kernel result")
    const hash = createHash("sha256")
    for await (const chunk of archive.createReadStream({ autoClose: false })) hash.update(chunk)
    await new Promise((resolve, reject) => {
      const child = spawn("tar", ["--zstd", "-tf", state.home_archive_path], { stdio: "ignore" })
      const timer = setTimeout(() => child.kill("SIGKILL"), 600_000)
      child.once("error", () => { clearTimeout(timer); reject(new Error("saved archive validation could not start")) })
      child.once("close", code => {
        clearTimeout(timer)
        if (code === 0) resolve()
        else reject(new Error("saved archive failed decompression and tar validation"))
      })
    })
    const after = await lstat(state.home_archive_path)
    assert.ok(after.isFile() && after.dev === before.dev && after.ino === before.ino
      && after.size === before.size && after.mtimeMs === before.mtimeMs && after.ctimeMs === before.ctimeMs,
    "saved archive changed during validation")
    return { sizeBytes: before.size, sha256: hash.digest("hex"), private: true, readableArchive: true }
  } finally { await archive.close() }
}
