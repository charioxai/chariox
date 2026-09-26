import assert from "node:assert/strict"
import { access, copyFile, mkdir } from "node:fs/promises"
import { join } from "node:path"
import { fileURLToPath } from "node:url"

// Copy the actual offline module graph, with no generated client or loaders.
// Keep capture outputs outside this fixture repository's protection boundary.
export async function offlineCorrelationScript(directory, name) {
  const repository = join(directory, "offline-repository")
  const scripts = join(repository, "apps/cli/scripts")
  await mkdir(scripts, { recursive: true })
  for (const file of [
    "path1-host-cloud-correlation.mjs",
    "path1-provider-cloud-correlation.mjs",
    "path1-provider-rebuild-capture.mjs",
  ]) {
    await copyFile(fileURLToPath(new URL(`./${file}`, import.meta.url)), join(scripts, file))
  }
  await assert.rejects(access(join(repository, "packages/kernel-client/dist")))
  return join(scripts, name)
}
