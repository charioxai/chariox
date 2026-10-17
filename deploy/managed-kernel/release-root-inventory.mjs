import { readdir } from "node:fs/promises"
import { dirname, join } from "node:path"

// MP-07/MP-11: tree artifacts already authenticate all their descendants.
// Outside them the release contains only declared artifacts, signature metadata,
// and the directories needed to reach those entries. Legacy installers also
// created these two empty directories without installing any drop-ins.
const legacyEmptyDirectories = new Set([
  "etc/systemd/system/chariox-rootless-docker.service.d",
  "etc/systemd/system/chariox-slice-disk-quota-allocator.service.d",
])
export async function verifyReleaseRootInventory(rootfs, artifacts, treePaths) {
  const files = new Set([
    "usr/lib/chariox/release-manifest.json",
    "usr/lib/chariox/release-manifest.sig",
    "usr/lib/chariox/release-public-key",
    ...artifacts.map((artifact) => artifact.path.slice(1)),
  ])
  const trees = new Set(treePaths.map((path) => path.slice(1)))
  const directories = new Set(trees)
  for (const path of files) {
    for (let parent = dirname(path); parent !== "."; parent = dirname(parent)) {
      directories.add(parent)
    }
  }
  async function inspect(directory, prefix = "") {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const path = prefix ? `${prefix}/${entry.name}` : entry.name
      if (entry.isDirectory() && directories.has(path)) {
        if (!trees.has(path)) await inspect(join(directory, entry.name), path)
      } else if (entry.isDirectory() && legacyEmptyDirectories.has(path)
        && (await readdir(join(directory, entry.name))).length === 0) {
        // MP-07/MP-11: compatibility is limited to empty installer-owned paths.
      } else if (!entry.isFile() || !files.has(path) || trees.has(path)) {
        throw new Error("release root contains an undeclared entry")
      }
    }
  }
  await inspect(rootfs)
}
