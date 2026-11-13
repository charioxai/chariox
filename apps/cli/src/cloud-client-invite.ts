import type { CloudClient } from "./cloud-client.js"
import type { LocalIpcClient } from "./ipc.js"
import { getRelayStatus } from "./relay-api.js"

/** Cloud records the Room's actual kernel, regardless of how this terminal reached it. */
export async function createCloudClientInvite(
  cloud: CloudClient,
  kernel: LocalIpcClient,
  sessionId: string,
  options: Omit<Parameters<CloudClient["collaboration"]["createSessionInvite"]>[1], "kernelId">,
) {
  const kernelId = (await getRelayStatus(kernel)).daemon_id
  if (!kernelId) throw new Error("The connected kernel did not return its id. Reconnect before creating an invite.")
  return cloud.collaboration.createSessionInvite(sessionId, { ...options, kernelId })
}
