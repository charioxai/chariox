import { opendir, readFile, stat } from "node:fs/promises"
import path from "node:path"

const DEFAULT_PRUNED_DIRECTORIES = new Set([
  ".git",
  "node_modules",
  ".pnpm-store",
  "debug",
  "release",
])

export async function findDrillJsonArtifactPaths(roots, {
  extension = ".json",
  fileName = null,
  maxDepth = 8,
  schema = null,
} = {}) {
  const discovered = new Set()
  for (const root of Array.isArray(roots) ? roots : [roots]) {
    await collectDrillJsonArtifactPaths(discovered, path.resolve(root), {
      depth: 0,
      extension,
      fileName,
      maxDepth,
      schema,
    })
  }
  return [...discovered].sort()
}

async function collectDrillJsonArtifactPaths(discovered, entryPath, options) {
  const entry = await stat(entryPath).catch(() => null)
  if (!entry) return
  if (entry.isFile()) {
    await maybeCollectDrillJsonArtifactPath(discovered, entryPath, options)
    return
  }
  if (!entry.isDirectory() || options.depth > options.maxDepth) return
  try {
    const dir = await opendir(entryPath)
    for await (const child of dir) {
      const childPath = path.join(entryPath, child.name)
      if (child.isFile()) {
        await maybeCollectDrillJsonArtifactPath(discovered, childPath, options)
        continue
      }
      if (!child.isDirectory() || DEFAULT_PRUNED_DIRECTORIES.has(child.name)) continue
      await collectDrillJsonArtifactPaths(discovered, childPath, {
        ...options,
        depth: options.depth + 1,
      })
    }
  } catch {
    // Ignore unreadable directories in broad artifact roots.
  }
}

async function maybeCollectDrillJsonArtifactPath(discovered, entryPath, { extension, fileName, schema }) {
  if (fileName !== null && path.basename(entryPath) !== fileName) return
  if (extension !== null && !entryPath.endsWith(extension)) return
  if (schema === null) {
    discovered.add(entryPath)
    return
  }
  try {
    const parsed = JSON.parse(await readFile(entryPath, "utf8"))
    if (parsed?.schema === schema) discovered.add(entryPath)
  } catch {
    // Ignore unrelated, malformed, or partial JSON files in broad artifact roots.
  }
}
