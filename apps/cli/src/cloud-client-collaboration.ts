import type { CollaborationLevel } from "./cli-types.js"
import type { CloudClientCredential } from "./cloud-client-credential-store.js"
import { cloudClientRequest } from "./cloud-client-http.js"
import { cloudClientSessionScopes } from "./cloud-client-collaboration-scope.js"

type AuthenticatedRequest = <T>(request: (credential: CloudClientCredential) => Promise<T>) => Promise<T>
type Member = {userId: string; email: string; displayName?: string | null; invitedByUserId?: string | null; joinedAt?: string}
type Collaborator = Member & {lastCollaboratedAt: string; sharedSessionCount: number}

/** Human-authenticated Cloud control plane only. Local invites, membership and
 * attachment changes remain on the ordinary kernel IPC path. */
export class CloudClientCollaboration {
  constructor(private readonly authenticated: AuthenticatedRequest,
    private readonly rememberSessionScope: (credential: CloudClientCredential, session: {sessionId: string; accountId: string}) => Promise<void>) {}
  async createSessionInvite(sessionId: string, options: {displayName?: string | null; expiresInMs?: number | null; maxUses?: number | null; collaborationLevel?: CollaborationLevel}) {
    return this.authenticated(async credential => {
      const invite = await cloudClientRequest<{inviteId: string; inviteToken: string; sessionId: string; accountId: string; createdByUserId: string; expiresAt?: string | null; maxUses?: number | null}>(credential.profile.apiUrl, "/sessions/invites", {body: {...options, sessionId, sessionToken: credential.accessToken, accountId: credential.profile.accountId}})
      await this.rememberSessionScope(credential, {sessionId: invite.sessionId, accountId: invite.accountId})
      return {invite: {invite_id: invite.inviteId, invite_token: invite.inviteToken, session_id: invite.sessionId, account_id: invite.accountId, created_by_user_id: invite.createdByUserId, expires_at: invite.expiresAt, max_uses: invite.maxUses}}
    })
  }
  async acceptSessionInvite(inviteToken: string) {
    return this.authenticated(async credential => {
      const acceptance = await cloudClientRequest<{sessionId: string; accountId: string; userId: string; invitedByUserId: string; joinedAt: string}>(credential.profile.apiUrl, `/sessions/invites/${encodeURIComponent(inviteToken)}/accept`, {body: {sessionToken: credential.accessToken}})
      await this.rememberSessionScope(credential, {sessionId: acceptance.sessionId, accountId: acceptance.accountId})
      return {acceptance: {session_id: acceptance.sessionId, account_id: acceptance.accountId, user_id: acceptance.userId, invited_by_user_id: acceptance.invitedByUserId, joined_at: acceptance.joinedAt}}
    })
  }
  async sessionMembers(sessionId: string) {
    return this.authenticated(async credential => {
      // MP-08 / MP-11: the invite's account scopes membership; the caller's
      // private CLIENT credential still authenticates every request and retry.
      const accountId = cloudClientSessionScopes(credential).reverse().find(scope => scope.sessionId === sessionId)?.accountId ?? credential.profile.accountId
      const query = new URLSearchParams({accountId, sessionId})
      const listed = await cloudClientRequest<{sessionId: string; members: Member[]}>(credential.profile.apiUrl, `/sessions/members?${query}`, {accessToken: credential.accessToken})
      return {session_id: listed.sessionId, members: listed.members.map(member => ({user_id: member.userId, email: member.email, display_name: member.displayName, invited_by_user_id: member.invitedByUserId, joined_at: member.joinedAt}))}
    })
  }
  async collaborators() {
    return this.authenticated(async credential => {
      const accounts = new Set([credential.profile.accountId, ...cloudClientSessionScopes(credential).map(scope => scope.accountId)])
      const contacts = new Map<string, Collaborator>()
      for (const accountId of accounts) {
        const query = new URLSearchParams({accountId})
        const listed = await cloudClientRequest<{collaborators: Collaborator[]}>(credential.profile.apiUrl, `/collaborators/recent?${query}`, {accessToken: credential.accessToken})
        for (const member of listed.collaborators) {
          const previous = contacts.get(member.userId)
          const latest = previous && Date.parse(previous.lastCollaboratedAt) > Date.parse(member.lastCollaboratedAt) ? previous : member
          contacts.set(member.userId, {...latest, sharedSessionCount: (previous?.sharedSessionCount ?? 0) + member.sharedSessionCount})
        }
      }
      return [...contacts.values()].sort((a, b) => Date.parse(b.lastCollaboratedAt)-Date.parse(a.lastCollaboratedAt) || a.userId.localeCompare(b.userId)).slice(0, 25)
        .map(member => ({user_id: member.userId, email: member.email, display_name: member.displayName, last_collaborated_at: member.lastCollaboratedAt, shared_session_count: member.sharedSessionCount}))
    })
  }
}
