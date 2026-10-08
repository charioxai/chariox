import { createCliRelayIdentityStore } from "./cli-relay-identity-store.js"
import type { LocalIpcClient } from "./ipc.js"
import { issueKernelCloudRelayClientToken } from "./relay-api.js"
import type { RelayCloudCommandHandlerDeps } from "./relay-cloud-command-handlers.js"

export function createInitialCloudClientTokenIssuer(
  client: LocalIpcClient,
  clientId: string,
  getIdentity: () => { publicKeyThumbprint: string } = () => createCliRelayIdentityStore().getOrCreate(),
): NonNullable<RelayCloudCommandHandlerDeps["issueCloudClientRelayToken"]> {
  return async (profile, targetDaemonAlias, tokenOptions) => {
    const identity = getIdentity()
    // Initial issuance uses the durable paired login, not this TUI process ID.
    // Renewal uses the admitted token subject through the SDK's exact issuer.
    return issueKernelCloudRelayClientToken(
      client, targetDaemonAlias, profile.clientId ?? clientId, tokenOptions?.sessionId ?? null,
      identity.publicKeyThumbprint,
    )
  }
}
