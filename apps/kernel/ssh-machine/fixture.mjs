// MP-07 / MP-11: signed synthetic fixtures. Private test signing keys stay in memory.
import { createHash, generateKeyPairSync, sign } from "node:crypto"
import { chmod, lstat, mkdir, readFile, readdir, writeFile } from "node:fs/promises"
import { dirname, join, relative } from "node:path"
import { spawnSync } from "node:child_process"

const sha = bytes => `sha256:${createHash("sha256").update(bytes).digest("hex")}`
async function put(root, p, bytes, mode = 0o644) {
  const dest = join(root, p.replace(/^\//, ""))
  await mkdir(dirname(dest), { recursive: true, mode: 0o755 })
  await writeFile(dest, bytes, { mode }); await chmod(dest, mode)
}
async function treeHash(root) {
  const hash = createHash("sha256"), m = await lstat(root)
  hash.update(`directory:1:.:${m.mode & 0o7777}:`)
  async function walk(dir) {
    for (const entry of (await readdir(dir)).sort()) {
      const p = join(dir, entry), rel = relative(root, p), m = await lstat(p)
      if (m.isDirectory()) { hash.update(`directory:${Buffer.byteLength(rel)}:${rel}:${m.mode & 0o7777}:`); await walk(p) }
      else { hash.update(`file:${Buffer.byteLength(rel)}:${rel}:${m.mode & 0o7777}:${m.size}:`); hash.update(await readFile(p)) }
    }
  }
  await walk(root)
  return `sha256:${hash.digest("hex")}`
}
export async function createFixture(dir, chosenKernelBytes) {
  const kernelBytes = chosenKernelBytes ?? Buffer.from('#!/bin/sh\nif [ "$1" = "--print-local-daemon-protocol-version" ]; then printf "479\\n"; exit 0; fi\nexit 64\n')
  const root = join(dir, "image"); await mkdir(root)
  const releaseKeys = generateKeyPairSync("ed25519"), builderKeys = generateKeyPairSync("ed25519")
  const publicKey = key => key.export({ format: "der", type: "spki" }).subarray(-32).toString("base64")
  const releasePublicKey = join(dir, "release-public-pin"), builderPublicKey = join(dir, "builder-public-pin")
  await writeFile(releasePublicKey, publicKey(releaseKeys.publicKey)); await writeFile(builderPublicKey, publicKey(builderKeys.publicKey))
  const sourceCommit = "a".repeat(40), sourceTree = "b".repeat(40)
  const artifacts = []
  const context = "usr/lib/chariox/slice-build-context"
  const bytes = Buffer.from("synthetic artifact; never executed\n")
  await put(root, `${context}/apps/kernel/slice-linux-docker/prebuilt/chariox-relay`, bytes, 0o755)
  for (const [path, source] of [
    ["deploy/local-linux/provision-docker-admission-locks.py", "../../../deploy/local-linux/provision-docker-admission-locks.py"],
    ["deploy/managed-kernel/chariox-docker-admission-locks.service", "../../../deploy/managed-kernel/chariox-docker-admission-locks.service"],
    ...["chariox-data-volume-admission.mjs", "slice-data-volume-device.mjs", "slice-data-volume-protected-io.mjs", "slice-disk-quota-xfs-readback.mjs"].map(p => [`apps/kernel/slice-linux-docker/${p}`, `../slice-linux-docker/${p}`]),
  ]) await put(root, `${context}/${path}`, await readFile(new URL(source, import.meta.url)))
  const attestation = Buffer.from(JSON.stringify({ schemaVersion: 1, sourceCommit, sourceTree, target: "x86_64-unknown-linux-gnu", artifacts: ["chariox-kernel", "chariox-managed-bootstrap", "chariox-relay"].map(name => ({ name, sha256: sha(name === "chariox-kernel" && kernelBytes ? kernelBytes : bytes) })) }))
  const specs = [
    ["chariox-kernel", "/usr/local/bin/chariox-kernel", kernelBytes ?? bytes, 0o755],
    ["chariox-managed-bootstrap", "/usr/local/bin/chariox-managed-bootstrap", bytes, 0o755],
    ...["chariox-managed-bootstrap.service", "chariox-path1-managed-bootstrap.service", "chariox-disposable-worker-bootstrap.service", "chariox-rootless-docker.service", "chariox-slice-broker.service"].map(n => [n, `/etc/systemd/system/${n}`, new URL(`../../../deploy/managed-kernel/${n}`, import.meta.url)]),
    ["chariox-data-volume-admission.service", "/etc/systemd/system/chariox-data-volume-admission.service", new URL("../slice-linux-docker/chariox-data-volume-admission.service", import.meta.url)],
    ["chariox-rootless-docker.path1-data-volume.conf", "/etc/systemd/system/chariox-rootless-docker.service.d/50-chariox-data-volume.conf", new URL("../slice-linux-docker/chariox-rootless-docker.path1-data-volume.conf", import.meta.url)],
    ["chariox-slice-disk-quota-allocator.path1-data-volume.conf", "/etc/systemd/system/chariox-slice-disk-quota-allocator.service.d/50-chariox-data-volume.conf", new URL("../slice-linux-docker/chariox-slice-disk-quota-allocator.path1-data-volume.conf", import.meta.url)],
    ["chariox-build-attestation", "/usr/lib/chariox/build-attestation.json", attestation],
    ["chariox-build-attestation-signature", "/usr/lib/chariox/build-attestation.sig", Buffer.from(sign(null, attestation, builderKeys.privateKey).toString("base64"))],
    ["chariox-builder-public-key", "/usr/lib/chariox/builder-public-key", Buffer.from(publicKey(builderKeys.publicKey))],
  ]
  for (const [name, path, data, mode] of specs) {
    const content = Buffer.isBuffer(data) ? data : await readFile(data)
    await put(root, path, content, mode)
    artifacts.push({ name, path, sha256: sha(content) })
  }
  artifacts.push({ name: "chariox-slice-build-context", path: `/${context}`, sha256: await treeHash(join(root, context)) })
  const manifest = Buffer.from(JSON.stringify({ schemaVersion: 3, managedUpdateEvidenceVersion: 1, sourceCommit, sourceTree, artifacts }))
  await put(root, "/usr/lib/chariox/release-manifest.json", manifest)
  await put(root, "/usr/lib/chariox/release-manifest.sig", Buffer.from(sign(null, manifest, releaseKeys.privateKey).toString("base64")))
  await put(root, "/usr/lib/chariox/release-public-key", Buffer.from(publicKey(releaseKeys.publicKey)))
  const archive = join(dir, "release.tar.gz")
  const result = spawnSync("tar", ["-czf", archive, "-C", root, "."], { stdio: "ignore" })
  if (result.status !== 0) throw new Error("fixture archive creation failed")
  return { root, archive, releasePublicKey, builderPublicKey, releasePublicKeyFingerprint: sha(releaseKeys.publicKey.export({ format: "der", type: "spki" }).subarray(-32)), builderPublicKeyFingerprint: sha(builderKeys.publicKey.export({ format: "der", type: "spki" }).subarray(-32)), releaseDigest: sha(manifest) }
}
