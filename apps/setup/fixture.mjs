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
  await put("bin/chariox-app-wasm-worker", "synthetic WASM worker; never executed\n", 0o755)
  // The signed bundle verifier requires the slice build context's Dockerfile and its local inputs to exist.
  await put("share/chariox/slice-build-context/apps/kernel/slice-linux-docker/docker/Dockerfile", "FROM scratch\nCOPY apps/kernel/slice-linux-docker/input /input\n")
  await put("share/chariox/slice-build-context/apps/kernel/slice-linux-docker/input", "synthetic slice input\n")
  const publicKeyHex = keys.publicKey.export({ format: "der", type: "spki" }).subarray(-32).toString("hex")
  const manifest = Buffer.from(JSON.stringify({ schema: "chariox.release-bundle.v1", version, platform, sourceCommit: "a".repeat(40), files: entries }))
  const signature = sign(null, manifest, keys.privateKey).toString("hex")
  await writeFile(join(bundle, "manifest.json"), manifest)
  await writeFile(join(bundle, "manifest.sig"), signature)
  const archive = join(root, `${name}.tar.gz`)
  if (spawnSync("tar", ["-czf", archive, "-C", root, name], { stdio: "ignore" }).status !== 0) throw new Error("Setup fixture archive failed")
  return { bundle, archive, manifest, signature, publicKeyHex, keys, version, platform, digest: `sha256:${sha(manifest)}` }
}
