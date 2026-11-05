import { readFile } from "node:fs/promises"
import { join } from "node:path"

// The one default connected-service URL lives in the kernel client; a build may override it.
// Setup and the browser-import extension both take their service from here.
export async function defaultControlPlaneUrl(repository) {
  const source = await readFile(join(repository, "packages/kernel-client/src/default-control-plane.ts"), "utf8")
  const url = /DEFAULT_CONTROL_PLANE_URL = "(https:\/\/[A-Za-z0-9.-]+)"/.exec(source)?.[1]
  if (!url) throw new Error("packages/kernel-client/src/default-control-plane.ts must export DEFAULT_CONTROL_PLANE_URL")
  return url
}
