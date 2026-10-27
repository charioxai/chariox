import type { CollaborationLevel, RuntimeSession } from "./cli-types.js"
import type { RelayCloudProfile } from "./preferences.js"

type CloudSessionInvitePayload = {
  invite: { invite_token: string; invite: { invite_id: string } }
  session: RuntimeSession
}

type CloudSessionJoinPayload = {
  member: { user_id: string }
  session: RuntimeSession
}

export type CloudSessionCommandHandlerDeps = {
  isAttached: () => boolean
  sessionState: () => RuntimeSession
  applySessionState: (session: RuntimeSession) => void
  attachBinding: (session: Pick<RuntimeSession, "id">, createdSession: boolean) => Promise<void>
  flashFooter: (message: string, tone: "info" | "error") => void
  appendNotice: (message: string) => void
  isRelayConnection?: () => boolean
  openExternalUrl?: (url: string) => Promise<boolean>
  createSessionInvite?: (
    sessionId: string,
    expiresInMs: number | null,
    maxUses: number | null,
    collaborationLevel?: CollaborationLevel,
  ) => Promise<CloudSessionInvitePayload>
  joinSessionInvite?: (
    inviteToken: string,
    userId: string,
  ) => Promise<CloudSessionJoinPayload>
  createCloudSessionInvite?: (
    sessionId: string,
    options: {
      displayName?: string | null
      expiresInMs?: number | null
      maxUses?: number | null
      collaborationLevel?: CollaborationLevel
    },
  ) => Promise<Record<string, unknown>>
  acceptCloudSessionInvite?: (inviteToken: string) => Promise<Record<string, unknown>>
  listCloudSessionMembers?: (sessionId: string) => Promise<Record<string, unknown>>
  listCloudCollaborators?: () => Promise<Record<string, unknown>[]>
}

export async function handleCloudSessionCommand(
  deps: CloudSessionCommandHandlerDeps,
  profile: RelayCloudProfile,
  area: string,
  action: string | undefined,
  args: string[],
): Promise<boolean> {
  if (area === "invite" && action === "create") {
    await createCloudInvite(deps, profile, args)
    return true
  }
  if (area === "invite" && action === "accept") {
    await acceptCloudInvite(deps, profile, args)
    return true
  }
  if ((area === "members" && !action) || (area === "members" && action === "list")) {
    await listCloudMembers(deps)
    return true
  }
  if ((area === "collaborators" && !action) || (area === "collaborators" && action === "list")) {
    await listCloudCollaborators(deps)
    return true
  }
  return false
}

async function createCloudInvite(
  deps: CloudSessionCommandHandlerDeps,
  profile: RelayCloudProfile,
  args: string[],
): Promise<void> {
  if (!deps.isAttached()) {
    deps.flashFooter("attach to a session before creating a cloud invite", "error")
    return
  }
  if (!deps.createCloudSessionInvite || !deps.createSessionInvite) {
    deps.flashFooter("cloud invite creation is unavailable in this build", "error")
    return
  }
  const collaborationLevel = parseCollaborationLevel(args)
  if (!collaborationLevel) {
    deps.flashFooter("usage: /cloud invite create [max-uses|--max-uses n] [--level private|transparent|full]", "error")
    return
  }
  const maxUsesIndex = args.findIndex((value) => value === "--max-uses")
  const maxUses = parsePositiveInt(maxUsesIndex >= 0 ? args[maxUsesIndex + 1] : args[0]) ?? 1
  if (maxUses <= 0) {
    deps.flashFooter("usage: /cloud invite create [max-uses|--max-uses n] [--level private|transparent|full]", "error")
    return
  }
  const session = deps.sessionState()
  const local = await deps.createSessionInvite(session.id, null, maxUses, collaborationLevel)
  deps.applySessionState(local.session)
  const cloud = await deps.createCloudSessionInvite(session.id, {
    displayName: session.alias ?? session.id,
    maxUses,
    collaborationLevel,
  })
  const cloudInvite = cloud.invite as { invite_id?: string; invite_token?: string }
  if (!cloudInvite.invite_token) {
    deps.flashFooter("cloud invite response was incomplete", "error")
    return
  }
  const inviteUrl = buildCloudInviteUrl(profile.apiUrl, cloudInvite.invite_token, local.invite.invite_token)
  const opened = await deps.openExternalUrl?.(inviteUrl)
  deps.appendNotice(
    [
      "cloud session invite",
      `url=${inviteUrl}`,
      `cloud_invite=${cloudInvite.invite_token}`,
      `local_invite=${local.invite.invite_token}`,
      `level=${collaborationLevel}`,
      `cloud_invite_id=${cloudInvite.invite_id ?? "-"}`,
      opened ? "browser=opened" : "browser=manual",
    ].join("\n"),
  )
  deps.flashFooter(opened ? "opened cloud invite link" : "cloud invite created", "info")
}

async function acceptCloudInvite(
  deps: CloudSessionCommandHandlerDeps,
  profile: RelayCloudProfile,
  args: string[],
): Promise<void> {
  if (!deps.acceptCloudSessionInvite || !deps.joinSessionInvite) {
    deps.flashFooter("cloud invite acceptance is unavailable in this build", "error")
    return
  }
  const inviteRef = args[0]
  if (!inviteRef) {
    deps.flashFooter("usage: /cloud invite accept <invite-token-or-url>", "error")
    return
  }
  const parsed = parseCloudInviteReference(inviteRef)
  if (parsed.localInviteToken && deps.isRelayConnection?.() === false && profile.userId !== "local") {
    deps.flashFooter("cloud invite accepted only through relay identity; reconnect with the relay invite link", "error")
    return
  }
  const accepted = await deps.acceptCloudSessionInvite(parsed.cloudInviteToken)
  const acceptance = accepted.acceptance as { user_id?: string }
  const userId = acceptance.user_id ?? profile.userId
  if (!parsed.localInviteToken) {
    deps.appendNotice("cloud invite accepted, but no local_invite token was present; use the local session invite token to join the kernel session")
    deps.flashFooter(`cloud invite accepted as ${userId}`, "info")
    return
  }
  const joined = await deps.joinSessionInvite(parsed.localInviteToken, userId)
  deps.applySessionState(joined.session)
  await deps.attachBinding(joined.session, false)
  deps.flashFooter(`joined cloud session as ${joined.member.user_id}`, "info")
}

async function listCloudMembers(deps: CloudSessionCommandHandlerDeps): Promise<void> {
  if (!deps.isAttached()) {
    deps.flashFooter("attach to a session before listing cloud members", "error")
    return
  }
  if (!deps.listCloudSessionMembers) {
    deps.flashFooter("cloud member listing is unavailable in this build", "error")
    return
  }
  const listed = await deps.listCloudSessionMembers(deps.sessionState().id)
  const members = (listed.members as Array<{ user_id: string; email: string; display_name?: string | null }> | undefined) ?? []
  deps.appendNotice(members.length === 0
    ? "No cloud members in this session."
    : members.map((member) => `${member.user_id} ${member.email}${member.display_name ? ` (${member.display_name})` : ""}`).join("\n"))
  deps.flashFooter(`listed ${members.length} cloud member${members.length === 1 ? "" : "s"}`, "info")
}

async function listCloudCollaborators(deps: CloudSessionCommandHandlerDeps): Promise<void> {
  if (!deps.listCloudCollaborators) {
    deps.flashFooter("cloud collaborator listing is unavailable in this build", "error")
    return
  }
  const collaborators = await deps.listCloudCollaborators()
  deps.appendNotice(collaborators.length === 0
    ? "No recent cloud collaborators."
    : collaborators.map((collaborator) => {
      const email = typeof collaborator.email === "string" ? collaborator.email : "-"
      const userId = typeof collaborator.user_id === "string" ? collaborator.user_id : "-"
      const count = typeof collaborator.shared_session_count === "number" ? collaborator.shared_session_count : 0
      return `${userId} ${email} shared_sessions=${count}`
    }).join("\n"))
  deps.flashFooter(`listed ${collaborators.length} cloud collaborator${collaborators.length === 1 ? "" : "s"}`, "info")
}

export function buildCloudInviteUrl(apiUrl: string, cloudInviteToken: string, localInviteToken: string): string {
  const url = new URL("/sessions/invites", apiUrl)
  url.searchParams.set("cloud_invite", cloudInviteToken)
  url.searchParams.set("local_invite", localInviteToken)
  return url.toString()
}

export function parseCloudInviteReference(value: string): { cloudInviteToken: string; localInviteToken?: string } {
  try {
    const url = new URL(value)
    const cloudInviteToken = url.searchParams.get("cloud_invite")
      ?? url.searchParams.get("invite")
      ?? url.pathname.split("/").filter(Boolean).at(-1)
      ?? ""
    const localInviteToken = url.searchParams.get("local_invite") ?? undefined
    if (cloudInviteToken) {
      return { cloudInviteToken, ...(localInviteToken ? { localInviteToken } : {}) }
    }
  } catch {
    // Plain token fallback below.
  }
  return { cloudInviteToken: value }
}

function parsePositiveInt(value: string | undefined): number | null | undefined {
  if (value === undefined) return undefined
  const parsed = Number.parseInt(value, 10)
  return Number.isFinite(parsed) && parsed > 0 ? parsed : null
}

export function parseCollaborationLevel(args: readonly string[]): CollaborationLevel | null {
  const levelIndex = args.findIndex((value) => value === "--level")
  const raw = levelIndex >= 0 ? args[levelIndex + 1] : undefined
  if (raw === undefined) return "private"
  if (raw === "private" || raw === "transparent" || raw === "full") return raw
  return null
}
