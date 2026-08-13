import path from "node:path"

const manifestByBinary = Object.freeze({
  "chariox-cli": "apps/kernel/Cargo.toml",
  "chariox-kernel": "apps/kernel/Cargo.toml",
  "chariox-relay": "apps/relay/Cargo.toml",
})

export function rustManifestPath(repoRoot, binaryName) {
  const manifest = manifestByBinary[binaryName]
  if (!manifest) throw new Error(`unsupported Rust binary ${binaryName}`)
  return path.join(repoRoot, manifest)
}

export function rustBinaryPath(repoRoot, binaryName, env = process.env) {
  if (!manifestByBinary[binaryName]) throw new Error(`unsupported Rust binary ${binaryName}`)
  const configuredTargetDir = env.CARGO_TARGET_DIR?.trim()
  const targetDir = configuredTargetDir
    ? path.resolve(repoRoot, configuredTargetDir)
    : path.join(repoRoot, "target")
  return path.join(targetDir, "debug", binaryName)
}
