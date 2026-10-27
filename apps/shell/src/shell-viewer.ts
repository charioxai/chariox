import { cloudRelayStatusRequest } from "@chariox/kernel-client/ipc-requests"
import { buildHostedCloudViewUrl, type ScopedSliceViewerTarget } from "@chariox/kernel-client/slice-screen-viewer"

export function createShellViewer(client: {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
}) {
  return async (target: ScopedSliceViewerTarget): Promise<{ url: string; opened: boolean } | null> => {
    const response = await client.send(cloudRelayStatusRequest())
    const status = response.CloudRelayStatus as { profile: { api_url: string } | null } | undefined
    if (!status || !("profile" in status)) throw new Error("Expected CloudRelayStatus from kernel")
    if (!status.profile) return null
    // Scripts and remote shells return the link, never launch a browser on the execution host.
    return { url: buildHostedCloudViewUrl(status.profile.api_url, target), opened: false }
  }
}
