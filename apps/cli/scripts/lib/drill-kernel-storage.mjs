import path from "node:path"
import { mkdir, writeFile } from "node:fs/promises"

export function drillKernelStoragePaths(storageRoot) {
  return {
    statePath: path.join(storageRoot, "state.db"),
    operationalHistoryPath: path.join(storageRoot, "operational-history.db"),
    operationalArtifactRoot: path.join(storageRoot, "artifacts"),
    operationalArtifactIndexPath: path.join(storageRoot, "artifacts.db"),
  }
}

export function isolatedKernelConfigToml(storageRoot, extraToml = []) {
  const storage = drillKernelStoragePaths(storageRoot)
  return [
    "version = 1",
    "",
    "[history.operational]",
    `path = ${JSON.stringify(storage.operationalHistoryPath)}`,
    "",
    "[artifacts.operational]",
    `root = ${JSON.stringify(storage.operationalArtifactRoot)}`,
    `index_path = ${JSON.stringify(storage.operationalArtifactIndexPath)}`,
    "",
    "[state]",
    `path = ${JSON.stringify(storage.statePath)}`,
    "snapshot_interval_events = 1000",
    ...(extraToml.length > 0 ? ["", ...extraToml] : []),
    "",
  ].join("\n")
}

export async function writeIsolatedKernelConfig({
  xdgConfigHome,
  storageRoot,
  extraToml = [],
}) {
  const configDir = path.join(xdgConfigHome, "chariox")
  await mkdir(configDir, { recursive: true })
  await writeFile(
    path.join(configDir, "config.toml"),
    isolatedKernelConfigToml(storageRoot, extraToml),
    { mode: 0o600 },
  )
  return drillKernelStoragePaths(storageRoot)
}
