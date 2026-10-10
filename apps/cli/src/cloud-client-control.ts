import type { CloudClient } from "./cloud-client.js"
import type { LocalIpcClient } from "./ipc.js"
import { getKernelCloudRelayProfile } from "./relay-api.js"

// MP-08 / MP-11: client authority stays private. Enrollment-bound operations
// supply the kernel; human collaboration uses the terminal's own account.
export async function getCloudClientControlProfile(client?: CloudClient, kernel?: LocalIpcClient) {
  const human = await client?.humanProfile() ?? null
  const enrollment = kernel && human ? await getKernelCloudRelayProfile(kernel) : null
  if (human && enrollment && (human.accountId !== enrollment.accountId || new URL(human.apiUrl).origin !== new URL(enrollment.apiUrl).origin)) throw new Error("Cloud account conflict; use a separate CHARIOX_HOME profile")
  return human
}
