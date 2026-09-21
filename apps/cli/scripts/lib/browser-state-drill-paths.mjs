import path from "node:path"

export function resolveBrowserStateDrillPaths({ homeDir, runId, stamp, env }) {
  return {
    artifactDir: env.M20_ARTIFACT_DIR ?? path.join(
      homeDir,
      ".codex",
      "evidence",
      "browser-computer-use",
      "persistence",
      stamp,
    ),
    tempRoot: env.M20_RUNTIME_ROOT ?? path.join(
      homeDir,
      ".chariox",
      "dev",
      "browser-computer-use-persistence",
      runId,
    ),
  }
}
