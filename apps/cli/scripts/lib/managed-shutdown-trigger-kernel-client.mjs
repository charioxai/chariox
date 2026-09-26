import { dirname, resolve } from "node:path"
import { fileURLToPath, pathToFileURL } from "node:url"

import { requireValue } from "./managed-shutdown-trigger-config.mjs"

export async function loadManagedShutdownKernelClient(kernelUrl) {
  const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../../..")
  const kernelClientDist = resolve(repositoryRoot, "packages/kernel-client/dist")
  const [transport, requests, protocol] = await Promise.all([
    import(pathToFileURL(resolve(kernelClientDist, "ipc.js")).href),
    import(pathToFileURL(resolve(kernelClientDist, "ipc-managed-environment-requests.js")).href),
    import(pathToFileURL(resolve(kernelClientDist, "kernel-types.js")).href),
  ])
  requireValue(protocol.LOCAL_DAEMON_PROTOCOL_VERSION >= requests.managedEnvironmentShutdownObservationMinimumProtocolVersion,
    "kernel client build does not support managed shutdown observations")
  return { client: new transport.LocalIpcClient(kernelUrl), requests }
}
