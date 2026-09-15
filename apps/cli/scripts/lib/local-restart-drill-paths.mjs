import path from "node:path"

function validateScopedAbsolutePath(name, value, homeDir) {
  if (!path.isAbsolute(value)) {
    throw new Error(`local restart ${name} must be an absolute path`)
  }
  const resolved = path.resolve(value)
  if (resolved === path.parse(resolved).root || resolved === path.resolve(homeDir)) {
    throw new Error(`local restart ${name} is too broad: ${resolved}`)
  }
  return resolved
}

function pathsOverlap(left, right) {
  const leftToRight = path.relative(left, right)
  const rightToLeft = path.relative(right, left)
  return leftToRight === ""
    || (!leftToRight.startsWith("..") && !path.isAbsolute(leftToRight))
    || (!rightToLeft.startsWith("..") && !path.isAbsolute(rightToLeft))
}

export function resolveLocalRestartDrillPaths({ homeDir, runId, env = process.env }) {
  const evidenceRoot = validateScopedAbsolutePath(
    "evidence root",
    env.CHARIOX_LOCAL_RESTART_EVIDENCE_ROOT
      ?? path.join(
        homeDir,
        ".codex/evidence/browser-computer-use/local-restart-persistence",
        runId,
      ),
    homeDir,
  )
  const runtimeRoot = validateScopedAbsolutePath(
    "runtime root",
    env.CHARIOX_LOCAL_RESTART_RUNTIME_ROOT
      ?? path.join(
        homeDir,
        ".chariox/dev/browser-computer-use-local-restart-persistence",
        runId,
      ),
    homeDir,
  )
  const cargoTargetDir = validateScopedAbsolutePath(
    "Cargo target directory",
    env.CHARIOX_LOCAL_RESTART_CARGO_TARGET_DIR
      ?? path.join(homeDir, ".chariox/dev/browser-computer-use/cargo-target"),
    homeDir,
  )
  const roots = [evidenceRoot, runtimeRoot, cargoTargetDir]
  for (let left = 0; left < roots.length; left += 1) {
    for (let right = left + 1; right < roots.length; right += 1) {
      if (pathsOverlap(roots[left], roots[right])) {
        throw new Error("local restart roots must differ and must not overlap")
      }
    }
  }
  return { evidenceRoot, runtimeRoot, cargoTargetDir }
}
