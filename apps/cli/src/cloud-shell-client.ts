// MP-08 / MP-11: shared-shell human controls use this terminal's private login.
import type { CloudClient } from "./cloud-client.js"
import type { LocalIpcClient } from "./ipc.js"
import { createCloudClientInvite } from "./cloud-client-invite.js"
import type { ShellExecutorDeps } from "@chariox/kernel-client/shell-executor"
export function cloudShellClient(client: CloudClient | undefined, kernel: LocalIpcClient): ShellExecutorDeps["cloudAccountClient"] {
  if (!client) return undefined
  return {ensureAuthenticated: () => client.ensureAuthenticated(), async send(request) {
    const value = request as Record<string, any>
    if ("CreateCloudSessionInvite" in value) {
      const r = value.CreateCloudSessionInvite
      return {CloudSessionInviteCreated:await createCloudClientInvite(client, kernel, r.session_id, {displayName:r.display_name, expiresInMs:r.expires_in_ms, maxUses:r.max_uses, collaborationLevel:r.collaboration_level})}
    }
    if ("AcceptCloudSessionInvite" in value) return {CloudSessionInviteAccepted:await client.collaboration.acceptSessionInvite(value.AcceptCloudSessionInvite.invite_token)}
    if ("ListCloudSessionMembers" in value) return {CloudSessionMembersListed:await client.collaboration.sessionMembers(value.ListCloudSessionMembers.session_id)}
    if ("ListCloudCollaborators" in value) return {CloudCollaboratorsListed:{collaborators:await client.collaboration.collaborators()}}
    if ("CloudRelayStatus" in value) {
      const profile = await client.profile()
      return {CloudRelayStatus:{profile:profile ? {account_slug:profile.accountSlug, email:profile.email} : null}}
    }
    throw new Error("Unsupported client Cloud account command")
  }}
}
