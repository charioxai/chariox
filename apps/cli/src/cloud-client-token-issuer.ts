import { createCliRelayIdentityStore } from "./cli-relay-identity-store.js"
import type { LocalIpcClient } from "./ipc.js"
import { issueKernelCloudRelayClientToken } from "./relay-api.js"
import type { RelayCloudCommandHandlerDeps } from "./relay-cloud-command-handlers.js"
import { isLocalRelayIssuerEndpoint } from "@chariox/kernel-client/relay-authorization"

export function createInitialCloudClientTokenIssuer(
  client: LocalIpcClient,
  clientId: string,
  getIdentity: () => { publicKeyThumbprint: string } = () => createCliRelayIdentityStore().getOrCreate(),
): NonNullable<RelayCloudCommandHandlerDeps["issueCloudClientRelayToken"]> {
  return async (profile, targetDaemonAlias, tokenOptions) => {
    const identity = getIdentity()
    // Initial issuance uses the durable paired login, not this TUI process ID.
    // Renewal uses the admitted token subject through the SDK's exact issuer.
    const issued = await issueKernelCloudRelayClientToken(
      client, targetDaemonAlias, profile.clientId ?? clientId, tokenOptions?.sessionId ?? null,
      identity.publicKeyThumbprint,
    )
    if (!isLocalRelayIssuerEndpoint(client.socketPath)) return issued
    const status = await client.send<{RelayStatus: {status: {daemon_id: string}}}>({RelayStatus: null})
    return {...issued, issuer: {endpoint: client.socketPath, daemonId: status.RelayStatus.status.daemon_id}}
  }
}
