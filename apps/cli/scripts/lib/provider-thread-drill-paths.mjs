import path from "node:path"

export function resolveProviderThreadDrillPaths({ homeDir, runId, env = process.env }) {
  const evidenceRoot = env.CHARIOX_PROVIDER_THREAD_EVIDENCE_ROOT
    ?? path.join(
      homeDir,
      ".codex/evidence/browser-computer-use/provider-thread-transfer",
      runId,
    )
  const runtimeRoot = env.CHARIOX_PROVIDER_THREAD_RUNTIME_ROOT
    ?? path.join(
      homeDir,
      ".chariox/dev/browser-computer-use-provider-thread-transfer",
      runId,
    )

  for (const [name, value] of Object.entries({ evidenceRoot, runtimeRoot })) {
    if (!path.isAbsolute(value)) {
      throw new Error(`provider thread ${name} must be an absolute path`)
    }
    const resolved = path.resolve(value)
    if (resolved === path.parse(resolved).root || resolved === path.resolve(homeDir)) {
      throw new Error(`provider thread ${name} is too broad: ${resolved}`)
    }
  }

  const evidenceFromRuntime = path.relative(runtimeRoot, evidenceRoot)
  const runtimeFromEvidence = path.relative(evidenceRoot, runtimeRoot)
  if (
    evidenceFromRuntime === ""
    || (!evidenceFromRuntime.startsWith("..") && !path.isAbsolute(evidenceFromRuntime))
    || (!runtimeFromEvidence.startsWith("..") && !path.isAbsolute(runtimeFromEvidence))
  ) {
    throw new Error("provider thread evidence and runtime roots must differ and must not overlap")
  }

  return { evidenceRoot, runtimeRoot }
}
