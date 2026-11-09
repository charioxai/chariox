import assert from "node:assert/strict"
import { access, copyFile, mkdir } from "node:fs/promises"
import { join } from "node:path"
import { fileURLToPath } from "node:url"

// Copy the actual offline module graph, with no generated client or loaders, and return its
// scripts directory. Keep capture outputs outside this fixture repository's protection boundary.
export async function offlineScriptsDirectory(directory) {
  const repository = join(directory, "offline-repository")
  const scripts = join(repository, "apps/cli/scripts")
  await mkdir(scripts, { recursive: true })
  await copyFile(fileURLToPath(new URL("./path1-capture-output.mjs", import.meta.url)), join(scripts, "path1-capture-output.mjs"))
  await assert.rejects(access(join(repository, "packages/kernel-client/dist")))
  return scripts
}
