import { withCloudClientProfileLock } from "./cloud-client-profile-lock.js"
import { cloudClientCallerScope, validCloudClientSessionScopes, type CloudClientSessionScope } from "./cloud-client-collaboration-scope.js"
import { randomBytes } from "node:crypto"
import { constants } from "node:fs"
import { mkdir, open, rename, rm } from "node:fs/promises"
import path from "node:path"
import { preferencesPath, publicRelayCloudProfile, type RelayCloudProfile } from "./preferences.js"
import { cloudClientRequest, CloudClientAuthError } from "./cloud-client-http.js"
export { CloudClientAuthError } from "./cloud-client-http.js"

export type CloudClientCredential = {
  profile: RelayCloudProfile
  clientId: string
  publicKeyThumbprint: string
  refreshCredential: string
  accessToken: string
  expiresAtMs: number
  pendingRotationId?: string
  loginId?: string
  collaborationScopes?: CloudClientSessionScope[]
}

/** This file contains client authority only; never expose it as kernel status
 * or preferences. Profile processes serialize rotation against the same file. */
export class CloudClientCredentialStore {
  constructor(readonly filePath = path.join(path.dirname(preferencesPath()), "relay", "cloud-client.json")) {}
  async load(): Promise<CloudClientCredential | null> {
    let handle
    try { handle = await open(this.filePath, constants.O_RDONLY | constants.O_NOFOLLOW) }
    catch (error) { if ((error as NodeJS.ErrnoException).code === "ENOENT") return null; throw error }
    try {
      const stat = await handle.stat()
      if (!stat.isFile() || stat.nlink !== 1 || stat.size > 64*1024 || (stat.mode & 0o777) !== 0o600 || (process.getuid && stat.uid !== process.getuid())) throw new CloudClientAuthError("unsafe_credential_file")
      let value: CloudClientCredential
      try { value = JSON.parse(await handle.readFile("utf8")) as CloudClientCredential } catch { throw new CloudClientAuthError("invalid_credential_file") }
      if (!value.profile?.accountId || !value.profile.apiUrl || !value.clientId || !/^[0-9a-f]{64}$/.test(value.publicKeyThumbprint) || !value.refreshCredential || !value.accessToken || !Number.isFinite(value.expiresAtMs)) throw new CloudClientAuthError("invalid_credential_file")
      if (!validCloudClientSessionScopes(value.collaborationScopes)) throw new CloudClientAuthError("invalid_credential_file")
      if (value.loginId !== undefined && !/^[0-9a-f]{32}$/.test(value.loginId)) throw new CloudClientAuthError("invalid_credential_file")
      return value
    } finally { await handle.close() }
  }
  async saveLogin(value: CloudClientCredential): Promise<void> {
    await this.lock(async () => {
      const previous = await this.load()
      if (previous && cloudClientCallerScope(previous) !== cloudClientCallerScope(value)) throw new CloudClientAuthError("profile_conflict")
      await this.write({...value, loginId: previous?.loginId ?? randomBytes(16).toString("hex"), ...(previous?.collaborationScopes ? {collaborationScopes: previous.collaborationScopes} : {})})
    })
  }
  async rememberSessionScope(credential: CloudClientCredential, session: {sessionId: string; accountId: string}): Promise<void> {
    if (![session.sessionId, session.accountId].every(field => typeof field === "string" && field.length > 0)) throw new CloudClientAuthError("invalid_collaboration_scope")
    await this.lock(async () => {
      const current = await this.load()
      if (!current) throw new CloudClientAuthError("login_required")
      const caller = cloudClientCallerScope(credential)
      if (cloudClientCallerScope(current) !== caller || current.loginId !== credential.loginId) throw new CloudClientAuthError("profile_conflict")
      // Merge against the latest file under the refresh/logout lock. Retain
      // previous namespaces for contacts; the last hint selects member lookup.
      const scopes = (current.collaborationScopes ?? []).filter(scope => !(scope.caller === caller && scope.sessionId === session.sessionId && scope.accountId === session.accountId))
      current.collaborationScopes = [...scopes, {caller, sessionId: session.sessionId, accountId: session.accountId}]
      await this.write(current)
    })
  }
  async clear(): Promise<void> { await this.lock(() => rm(this.filePath, { force: true })) }
  async session(publicKeyThumbprint: string, force = false, rejectedAccessToken?: string): Promise<CloudClientCredential> {
    return this.lock(async () => {
      let value = await this.load()
      if (!value) throw new CloudClientAuthError("login_required")
      if (value.publicKeyThumbprint !== publicKeyThumbprint) throw new CloudClientAuthError("profile_conflict")
      if (!force && value.accessToken !== rejectedAccessToken && !value.pendingRotationId && value.expiresAtMs > Date.now()+60_000) return value
      // An idempotent retry may recover a valid refresh successor whose
      // original access token has expired. Commit that successor before trying
      // its own rotation, so a second lost reply/restart remains recoverable.
      for (let attempt = 0; attempt < 2; attempt++) {
        value.pendingRotationId ??= randomBytes(32).toString("hex")
        await this.write(value)
        let next: { refreshCredential: string; cloudSessionToken: string; cloudSessionExpiresAt: string }
        try {
          next = await cloudClientRequest(value.profile.apiUrl, "/auth/client/refresh", { body: { accountId: value.profile.accountId, clientId: value.clientId, publicKeyThumbprint, refreshCredential: value.refreshCredential, rotationId: value.pendingRotationId } })
        } catch (error) {
          if (error instanceof CloudClientAuthError && ["client_revoked", "refresh_reuse_detected"].includes(error.code)) await rm(this.filePath, { force: true })
          throw error
        }
        const expiresAtMs = Date.parse(next.cloudSessionExpiresAt)
        if (typeof next.refreshCredential !== "string" || !next.refreshCredential || typeof next.cloudSessionToken !== "string" || !next.cloudSessionToken || !Number.isFinite(expiresAtMs)) throw new CloudClientAuthError("invalid_refresh_response")
        value = { ...value, refreshCredential: next.refreshCredential, accessToken: next.cloudSessionToken, expiresAtMs }
        delete value.pendingRotationId
        await this.write(value)
        if (expiresAtMs > Date.now()) return value
      }
      throw new CloudClientAuthError("invalid_refresh_response")
    })
  }
  private async write(value: CloudClientCredential) {
    const serialized = JSON.stringify({ ...value, profile: publicRelayCloudProfile(value.profile) })
    if (Buffer.byteLength(serialized) > 64*1024) throw new CloudClientAuthError("credential_file_too_large")
    await mkdir(path.dirname(this.filePath), { recursive: true, mode: 0o700 })
    const temporary = `${this.filePath}.${randomBytes(16).toString("hex")}.tmp`
    const handle = await open(temporary, "wx", 0o600)
    try {
      try { await handle.writeFile(serialized); await handle.sync() }
      finally { await handle.close() }
      await rename(temporary, this.filePath)
      const directory = await open(path.dirname(this.filePath), constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW)
      try { await directory.sync() } finally { await directory.close() }
    } finally { await rm(temporary, { force: true }) }
  }
  private async lock<T>(operation: () => Promise<T>): Promise<T> {
    await mkdir(path.dirname(this.filePath), { recursive: true, mode: 0o700 })
    return withCloudClientProfileLock(`${this.filePath}.lock`, operation)
  }
}
