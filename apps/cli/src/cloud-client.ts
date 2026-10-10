import { CloudClientCollaboration } from "./cloud-client-collaboration.js"
import type { CloudControlProfile } from "./cloud-control-auth.js"
import { RelayAuthRenewal } from "@chariox/kernel-client/relay-auth-renewal"
import { createCliRelayIdentityStore } from "./cli-relay-identity-store.js"
import { CloudClientCredentialStore, type CloudClientCredential } from "./cloud-client-credential-store.js"
import { cloudClientRequest, CloudClientAuthError } from "./cloud-client-http.js"
import { LocalIpcClient, type RelayClientIdentity } from "./ipc.js"
import { publicRelayCloudProfile, type RelayCloudProfile } from "./preferences.js"
import { requireRelayTokenKeyBinding } from "./relay-api.js"
import type { WaitingRoomRemoteState } from "./waiting-room-types.js"

export type CloudKernelTarget = {
  daemonId?: string; daemonAlias?: string; machineId?: string; machineAlias?: string
  status: "ONLINE" | "OFFLINE" | "STALE" | "REVOKED" | "DRAINING"
  acceptingLeases?: boolean; available_providers?: string[]
}
type LoginResult = { status: string; intervalSeconds?: number; profile?: RelayCloudProfile & { enrollmentKind?: string; publicKeyThumbprint?: string }; cloudSessionToken?: string; cloudSessionExpiresAt?: string; refreshCredential?: string; kernelCredential?: string; machineCredential?: string }

/** Client authority handles Cloud bootstrap only. Runtime requests always use
 * the ordinary encrypted LocalIpcClient path to the selected kernel. */
export class CloudClient {
  readonly collaboration = new CloudClientCollaboration(request => this.authenticated(request), (credential, session) => this.store.rememberSessionScope(credential, session))
  private renewal: RelayAuthRenewal | undefined
  private clients = new Set<LocalIpcClient>()
  private onRevoked: (() => void) | undefined
  private revoked = false
  constructor(readonly store = new CloudClientCredentialStore(), private readonly identity: () => RelayClientIdentity = () => createCliRelayIdentityStore().getOrCreate()) {}

  async profile(): Promise<RelayCloudProfile | null> { return publicRelayCloudProfile((await this.store.load())?.profile) }
  async humanProfile(): Promise<CloudControlProfile | null> {
    const value = await this.store.load()
    if (!value) return null
    const profile = publicRelayCloudProfile(value.profile)!
    return {...profile, authenticatedFetch: (url, options) => this.authenticated(async credential => {
      const target = new URL(url)
      if (credential.profile.accountId !== profile.accountId || credential.profile.userId !== profile.userId || credential.clientId !== value.clientId || new URL(credential.profile.apiUrl).origin !== new URL(profile.apiUrl).origin || target.origin !== new URL(profile.apiUrl).origin) throw new CloudClientAuthError("profile_conflict")
      if (target.username || target.password || (target.protocol !== "https:" && !(target.protocol === "http:" && ["localhost", "127.0.0.1", "[::1]"].includes(target.hostname)))) throw new CloudClientAuthError("insecure_auth_endpoint")
      const headers = new Headers(options?.headers)
      headers.set("accept", "application/json"); headers.set("content-type", "application/json")
      headers.set("authorization", `Bearer ${credential.accessToken}`)
      const response = await fetch(target, {...options, headers, redirect: "error", signal: options?.signal ?? AbortSignal.timeout(20_000)})
      if (!response.ok) {
        const body = await response.clone().json().catch(() => null)
        const code = body?.error?.code
        if (["session_invalid", "client_revoked", "refresh_reuse_detected"].includes(code)) throw new CloudClientAuthError(code)
      }
      return response
    })}
  }
  async login(apiUrl: string, show: (verification: { verificationUrl: string; userCode: string }) => Promise<void> | void, expectedAccountId?: string): Promise<RelayCloudProfile> {
    const previous = await this.store.load()
    if (previous) {
      if (expectedAccountId && previous.profile.accountId !== expectedAccountId) throw new CloudClientAuthError("profile_conflict")
      if (new URL(previous.profile.apiUrl).origin !== new URL(apiUrl).origin) throw new CloudClientAuthError("profile_conflict")
      const current = await this.store.session(this.identity().publicKeyThumbprint)
      this.startRenewal(current)
      return publicRelayCloudProfile(current.profile)!
    }
    const identity = this.identity(), clientId = `cli:${identity.publicKeyThumbprint}`
    const started = await cloudClientRequest<{ deviceCode: string; userCode: string; verificationUrl: string; expiresAt: string; intervalSeconds: number }>(apiUrl, "/auth/device/start", { body: { enrollmentKind: "CLIENT", clientId, clientAlias: "Chariox CLI", publicKeyThumbprint: identity.publicKeyThumbprint } })
    await show({ verificationUrl: started.verificationUrl, userCode: started.userCode })
    let interval = started.intervalSeconds
    while (Date.now() < Date.parse(started.expiresAt)) {
      const result = await cloudClientRequest<LoginResult>(apiUrl, "/auth/device/poll", { body: { deviceCode: started.deviceCode } })
      if (result.status === "approved") {
        if (!result.profile || result.profile.enrollmentKind !== "CLIENT"
          || result.profile.publicKeyThumbprint !== identity.publicKeyThumbprint || result.profile.clientId !== clientId
          || result.profile.machineId !== undefined || result.profile.kernelId !== undefined || result.kernelCredential || result.machineCredential
          || ["accountId", "userId", "accountSlug", "realmId", "relayUrl", "issuerId", "email"].some(field => typeof result.profile![field as keyof RelayCloudProfile] !== "string")
          || typeof result.refreshCredential !== "string" || !result.refreshCredential
          || typeof result.cloudSessionToken !== "string" || !result.cloudSessionToken
          || typeof result.cloudSessionExpiresAt !== "string") throw new CloudClientAuthError("invalid_client_enrollment")
        if (expectedAccountId && result.profile.accountId !== expectedAccountId) {
          await cloudClientRequest(apiUrl, "/auth/logout", { body: { sessionToken: result.cloudSessionToken, accountId: result.profile.accountId, clientId, revokeClient: true } }).catch(() => {})
          throw new CloudClientAuthError("profile_conflict")
        }
        const credential: CloudClientCredential = { profile: { ...result.profile, apiUrl }, clientId, publicKeyThumbprint: identity.publicKeyThumbprint, refreshCredential: result.refreshCredential, accessToken: result.cloudSessionToken, expiresAtMs: Date.parse(result.cloudSessionExpiresAt) }
        if (!Number.isFinite(credential.expiresAtMs)) throw new CloudClientAuthError("invalid_client_enrollment")
        await this.store.saveLogin(credential)
        this.startRenewal(credential)
        return publicRelayCloudProfile(credential.profile)!
      }
      if (result.status === "expired_token") break
      if (result.status !== "authorization_pending") throw new CloudClientAuthError("login_denied")
      interval = result.intervalSeconds ?? interval
      await new Promise(resolve => setTimeout(resolve, Math.max(1, interval) * 1_000))
    }
    throw new CloudClientAuthError("device_authorization_expired")
  }
  async resume(onRevoked?: () => void): Promise<void> {
    this.onRevoked = onRevoked
    const credential = await this.store.load()
    if (credential) this.startRenewal(credential)
  }
  private startRenewal(credential: CloudClientCredential) {
    if (this.renewal) return
    this.revoked = false
    this.renewal = new RelayAuthRenewal(credential.expiresAtMs, async () => {
      try {
        const fresh = await this.store.session(this.identity().publicKeyThumbprint)
        return { token: fresh.accessToken, expiresAtMs: fresh.expiresAtMs }
      } catch (error) {
        if (error instanceof CloudClientAuthError && error.code === "login_required") throw new CloudClientAuthError("client_revoked")
        throw error
      }
    }, () => {}, () => this.invalidate(new CloudClientAuthError("client_revoked")))
  }
  private invalidate(error: CloudClientAuthError) {
    if (this.revoked) return
    this.revoked = true
    this.renewal?.stop(); this.renewal = undefined
    for (const client of this.clients) client.invalidateRelayAuthorization(error)
    this.onRevoked?.()
  }
  stop() {
    this.renewal?.stop(); this.renewal = undefined
    for (const client of this.clients) client.destroy()
    this.clients.clear()
  }
  async logout(): Promise<void> {
    if (!await this.store.load()) return
    try { await this.authenticated(async credential => cloudClientRequest(credential.profile.apiUrl, "/auth/logout", { body: { sessionToken: credential.accessToken, accountId: credential.profile.accountId, clientId: credential.clientId, revokeClient: true } })) }
    catch (error) { if (!(error instanceof CloudClientAuthError) || !["client_revoked", "refresh_reuse_detected"].includes(error.code)) throw error }
    await this.store.clear()
    for (const client of this.clients) client.invalidateRelayAuthorization(new CloudClientAuthError("client_revoked"))
    this.stop()
  }
  async directory(): Promise<CloudKernelTarget[]> {
    return this.authenticated(async credential => {
      const query = new URLSearchParams({ accountId: credential.profile.accountId, realmId: credential.profile.realmId })
      const result = await cloudClientRequest<{ targets: CloudKernelTarget[] }>(credential.profile.apiUrl, `/relay/targets?${query}`, { accessToken: credential.accessToken })
      return result.targets.filter(target => target.daemonId && target.status !== "REVOKED")
    })
  }
  async issue(kernelId: string) {
    return this.authenticated(async credential => {
      const result = await cloudClientRequest<{ token: string; expiresAt: string }>(credential.profile.apiUrl, "/relay/token", { body: { sessionToken: credential.accessToken, accountId: credential.profile.accountId, realmId: credential.profile.realmId, subject: credential.clientId, subjectKind: "client", clientId: credential.clientId, userId: credential.profile.userId, publicKeyThumbprint: credential.publicKeyThumbprint, allowedTargets: [kernelId], ttlMs: 300_000 } })
      requireRelayTokenKeyBinding(result.token, credential.publicKeyThumbprint, "detached client connection")
      const expiresAtMs = Date.parse(result.expiresAt)
      if (!Number.isFinite(expiresAtMs) || expiresAtMs <= Date.now()) throw new CloudClientAuthError("invalid_grant_response")
      return { token: result.token, expiresAtMs, relayUrl: credential.profile.relayUrl }
    })
  }
  async connect(kernelId: string): Promise<LocalIpcClient> {
    const target = (await this.directory()).find(target => target.daemonId === kernelId)
    if (!target || target.status !== "ONLINE") throw new CloudClientAuthError("kernel_offline")
    const grant = await this.issue(kernelId)
    const profile = await this.profile()
    if (!profile) throw new CloudClientAuthError("login_required")
    if (new URL(profile.apiUrl).protocol === "https:" && new URL(grant.relayUrl).protocol !== "wss:") throw new CloudClientAuthError("insecure_relay_endpoint")
    const client = new LocalIpcClient(grant.relayUrl, { relayAuthToken: grant.token, relayIdentity: this.identity(), targetDaemonId: kernelId })
    this.clients.add(client)
    client.startRelayAuthRenewal(grant.expiresAtMs, () => this.issue(kernelId), () => { this.clients.delete(client) })
    return client
  }
  private async authenticated<T>(request: (credential: CloudClientCredential) => Promise<T>): Promise<T> {
    if (this.revoked) throw new CloudClientAuthError("client_revoked")
    try {
      let credential = await this.store.session(this.identity().publicKeyThumbprint)
      try { return await request(credential) }
      catch (error) {
        if (!(error instanceof CloudClientAuthError) || error.code !== "session_invalid") throw error
        // A different profile process may have rotated between load and request.
        credential = await this.store.session(this.identity().publicKeyThumbprint, false, credential.accessToken)
        return await request(credential)
      }
    } catch (error) {
      // A concurrent refresh/logout may remove the private credential before
      // its revocation callback runs. Existing admission must settle as revoked.
      const authError = error instanceof CloudClientAuthError && error.code === "login_required" && (this.renewal || this.revoked)
        ? new CloudClientAuthError("client_revoked") : error
      if (authError instanceof CloudClientAuthError && ["client_revoked", "refresh_reuse_detected"].includes(authError.code)) this.invalidate(authError)
      throw authError
    }
  }
}

export function cloudDirectoryProjection(targets: CloudKernelTarget[]): Required<Pick<WaitingRoomRemoteState, "machines" | "kernels">> {
  const machineIds = [...new Set(targets.flatMap(target => target.machineId ? [target.machineId] : []))]
  return {
    machines: machineIds.map(machineId => {
      const kernels = targets.filter(target => target.machineId === machineId)
      return { machine_id: machineId, machine_alias: kernels[0]?.machineAlias ?? null, kernel_count: kernels.length, online: kernels.some(target => target.status === "ONLINE"), trust_status: "approved" }
    }),
    kernels: targets.filter(target => target.status === "ONLINE" && target.machineId).map(target => ({ kernel_id: target.daemonId!, machine_id: target.machineId!, machine_alias: target.machineAlias ?? null, kernel_alias: target.daemonAlias ?? null, available_providers: target.available_providers ?? [], accepting_remote_leases: target.acceptingLeases ?? false })),
  }
}

/** One-shot human control commands renew from the private CLI profile. */
export async function loadCloudClientControlProfile() {
  const client = new CloudClient()
  try {
    const profile = await client.humanProfile()
    if (!profile) throw new Error("Terminal is signed out. Run chariox cloud login first.")
    return profile
  } finally { client.stop() }
}


/** Resolve an enrolled-kernel pairing link using the receiving terminal's own
 * client profile. No kernel transport token or kernel credential is exported. */
export async function issueCloudPairingBootstrapToken(relayUrl: string, kernelId: string, identity: RelayClientIdentity): Promise<string> {
  const client = new CloudClient(undefined, () => identity)
  try {
    const profile = await client.profile()
    if (!profile) throw new Error("Cloud terminal pairing requires a signed-in terminal; run chariox cloud login first, or supply a separate CLIENT token with --relay-token-env NAME")
    if (profile.relayUrl !== relayUrl) throw new CloudClientAuthError("profile_conflict")
    if (new URL(profile.apiUrl).protocol === "https:" && new URL(relayUrl).protocol !== "wss:") throw new CloudClientAuthError("insecure_relay_endpoint")
    return (await client.issue(kernelId)).token
  } finally { client.stop() }
}
