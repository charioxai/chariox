// MP-07 / MP-11: synthetic release fixtures; signing keys remain in memory.
import { createHash, generateKeyPairSync, sign } from "node:crypto"
import { chmod, mkdir, readFile, readdir, writeFile } from "node:fs/promises"
import { dirname, join, relative } from "node:path"
import { spawnSync } from "node:child_process"
const sha = b => createHash("sha256").update(b).digest("hex")
export async function setupFixture(root, { version = "0.3.0", platform = "linux-x64", keys = generateKeyPairSync("ed25519"), kernel } = {}) {
  const name = `chariox-${version}-${platform}`, bundle = join(root, name)
  await mkdir(bundle, { recursive: true })
  const entries = []
  async function put(path, bytes, mode = 0o644) {
    bytes = Buffer.from(bytes); const target = join(bundle, path)
    await mkdir(dirname(target), { recursive: true }); await writeFile(target, bytes, { mode }); await chmod(target, mode)
    entries.push({ path, sha256: sha(bytes), size: bytes.length, mode: mode.toString(8).padStart(4, "0") })
  }
  await put("bin/chariox-kernel", kernel ?? '#!/bin/sh\nif [ "$1" = "--print-local-daemon-protocol-version" ]; then echo 479; exit 0; fi\nexit 64\n', 0o755)
  await put("bin/chariox", '#!/bin/sh\nexit 0\n', 0o755)
  await put("bin/chariox-setup", '#!/bin/sh\nexit 0\n', 0o755)
  const bytes = Buffer.from("synthetic runtime; never executed\n")
  await put("runtime/chariox-app-worker", bytes, 0o555)
  const inventory = Buffer.from(JSON.stringify({ schema: "chariox.app-runtime-inventory.v1", sourceCommit: "a".repeat(40), target: platform, files: [{ path: "chariox-app-worker", sha256: sha(bytes), size: bytes.length }] }))
  await put("runtime/runtime-inventory.json", inventory, 0o444)
  await put("runtime/runtime-inventory.sig", sign(null, inventory, keys.privateKey).toString("hex"), 0o444)
  await put("runtime/.runtime-lease", "", 0o444)
  const publicKeyHex = keys.publicKey.export({ format: "der", type: "spki" }).subarray(-32).toString("hex")
  const manifest = Buffer.from(JSON.stringify({ schema: "chariox.release-bundle.v1", version, platform, sourceCommit: "a".repeat(40), files: entries, runtime: { publicKeyHex, inventorySha256: sha(inventory) } }))
  const signature = sign(null, manifest, keys.privateKey).toString("hex")
  await writeFile(join(bundle, "manifest.json"), manifest)
  await writeFile(join(bundle, "manifest.sig"), signature)
  const archive = join(root, `${name}.tar.gz`)
  if (spawnSync("tar", ["-czf", archive, "-C", root, name], { stdio: "ignore" }).status !== 0) throw new Error("Setup fixture archive failed")
  return { bundle, archive, manifest, signature, publicKeyHex, keys, version, platform, digest: `sha256:${sha(manifest)}` }
}
