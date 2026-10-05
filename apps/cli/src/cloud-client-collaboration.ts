import type { CollaborationLevel } from "./cli-types.js"
import type { CloudClientCredential } from "./cloud-client-credential-store.js"
import { cloudClientRequest } from "./cloud-client-http.js"

type AuthenticatedRequest = <T>(request: (credential: CloudClientCredential) => Promise<T>) => Promise<T>
type Member = {userId: string; email: string; displayName?: string | null; invitedByUserId?: string | null; joinedAt?: string}

/** Human-authenticated Cloud control plane only. Local invites, membership and
 * attachment changes remain on the ordinary kernel IPC path. */
export class CloudClientCollaboration {
  constructor(private readonly authenticated: AuthenticatedRequest) {}
  async createSessionInvite(sessionId: string, options: {displayName?: string | null; expiresInMs?: number | null; maxUses?: number | null; collaborationLevel?: CollaborationLevel}) {
    return this.authenticated(async credential => {
      const invite = await cloudClientRequest<{inviteId: string; inviteToken: string; sessionId: string; accountId: string; createdByUserId: string; expiresAt?: string | null; maxUses?: number | null}>(credential.profile.apiUrl, "/sessions/invites", {body: {...options, sessionId, sessionToken: credential.accessToken, accountId: credential.profile.accountId}})
      return {invite: {invite_id: invite.inviteId, invite_token: invite.inviteToken, session_id: invite.sessionId, account_id: invite.accountId, created_by_user_id: invite.createdByUserId, expires_at: invite.expiresAt, max_uses: invite.maxUses}}
    })
  }
  async acceptSessionInvite(inviteToken: string) {
    return this.authenticated(async credential => {
      const acceptance = await cloudClientRequest<{sessionId: string; accountId: string; userId: string; invitedByUserId: string; joinedAt: string}>(credential.profile.apiUrl, `/sessions/invites/${encodeURIComponent(inviteToken)}/accept`, {body: {sessionToken: credential.accessToken}})
      return {acceptance: {session_id: acceptance.sessionId, account_id: acceptance.accountId, user_id: acceptance.userId, invited_by_user_id: acceptance.invitedByUserId, joined_at: acceptance.joinedAt}}
    })
  }
  async sessionMembers(sessionId: string) {
    return this.authenticated(async credential => {
      const query = new URLSearchParams({accountId: credential.profile.accountId, sessionId})
      const listed = await cloudClientRequest<{sessionId: string; members: Member[]}>(credential.profile.apiUrl, `/sessions/members?${query}`, {accessToken: credential.accessToken})
      return {session_id: listed.sessionId, members: listed.members.map(member => ({user_id: member.userId, email: member.email, display_name: member.displayName, invited_by_user_id: member.invitedByUserId, joined_at: member.joinedAt}))}
    })
  }
  async collaborators() {
    return this.authenticated(async credential => {
      const query = new URLSearchParams({accountId: credential.profile.accountId})
      const listed = await cloudClientRequest<{collaborators: (Member & {lastCollaboratedAt: string; sharedSessionCount: number})[]}>(credential.profile.apiUrl, `/collaborators/recent?${query}`, {accessToken: credential.accessToken})
      return listed.collaborators.map(member => ({user_id: member.userId, email: member.email, display_name: member.displayName, last_collaborated_at: member.lastCollaboratedAt, shared_session_count: member.sharedSessionCount}))
    })
  }
}
