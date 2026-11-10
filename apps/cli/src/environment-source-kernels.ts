import type { CloudClient } from "./cloud-client.js"
import type { EnvironmentSourceKernel } from "./project-environment-projection.js"

/** The user's other online kernels, from the Cloud directory the web lists them from; none when the terminal is signed out or the directory cannot be read. */
export async function environmentSourceKernels(client: Pick<CloudClient, "directory">, ownKernelId: () => Promise<string>): Promise<readonly EnvironmentSourceKernel[]> {
  try {
    const [targets, own] = await Promise.all([client.directory(), ownKernelId()])
    return targets
      .filter((target) => target.status === "ONLINE" && target.daemonId && target.daemonId !== own)
      .map((target) => ({ id: target.daemonId!, label: target.name ?? target.daemonId! }))
  } catch {
    return []
  }
}
