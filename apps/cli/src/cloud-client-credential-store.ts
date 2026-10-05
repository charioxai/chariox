import { withCloudClientProfileLock } from "./cloud-client-profile-lock.js"
import { randomBytes } from "node:crypto"
import { constants } from "node:fs"
import { mkdir, open, rename, rm } from "node:fs/promises"
import path from "node:path"
import { preferencesPath, publicRelayCloudProfile, type RelayCloudProfile } from "./preferences.js"

export type CloudClientCredential = {
  profile: RelayCloudProfile
  clientId: string
  publicKeyThumbprint: string
  refreshCredential: string
  accessToken: string
  expiresAtMs: number
  pendingRotationId?: string
}

export class CloudClientAuthError extends Error {
  constructor(readonly code: string) { super(`Cloud client authentication failed (${code})`) }
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
      return value
    } finally { await handle.close() }
  }
  async saveLogin(value: CloudClientCredential): Promise<void> {
    await this.lock(async () => {
      const previous = await this.load()
      if (previous && (previous.profile.accountId !== value.profile.accountId || previous.clientId !== value.clientId || previous.publicKeyThumbprint !== value.publicKeyThumbprint)) throw new CloudClientAuthError("profile_conflict")
      await this.write(value)
    })
  }
  async session(publicKeyThumbprint: string, force = false): Promise<CloudClientCredential> {
    return this.lock(async () => {
      let value = await this.load()
      if (!value) throw new CloudClientAuthError("login_required")
      if (value.publicKeyThumbprint !== publicKeyThumbprint) throw new CloudClientAuthError("profile_conflict")
      if (!force && !value.pendingRotationId && value.expiresAtMs > Date.now()+60_000) return value
      // Persist before the network call: a crash/lost reply retries this same
      // operation, rather than appearing as another use of an old credential.
      value.pendingRotationId ??= randomBytes(32).toString("hex")
      await this.write(value)
      const url = new URL("/auth/client/refresh", value.profile.apiUrl)
      if (url.username || url.password || (url.protocol !== "https:" && !(url.protocol === "http:" && ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname)))) throw new CloudClientAuthError("insecure_auth_endpoint")
      const response = await fetch(url, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ accountId: value.profile.accountId, clientId: value.clientId, publicKeyThumbprint, refreshCredential: value.refreshCredential, rotationId: value.pendingRotationId }), signal: AbortSignal.timeout(20_000) })
      if (!response.ok) {
        const error = await response.json().catch(() => null) as {error?: {code?: string}} | null
        const rawCode = error?.error?.code
        const code = rawCode && /^[a-z_]{1,64}$/.test(rawCode) ? rawCode : `http_${response.status}`
        if (["client_revoked", "refresh_reuse_detected"].includes(code)) await rm(this.filePath, { force: true })
        throw new CloudClientAuthError(code)
      }
      const next = await response.json() as { refreshCredential: string; cloudSessionToken: string; cloudSessionExpiresAt: string }
      const expiresAtMs = Date.parse(next.cloudSessionExpiresAt)
      if (!next.refreshCredential || !next.cloudSessionToken || !Number.isFinite(expiresAtMs) || expiresAtMs <= Date.now()) throw new CloudClientAuthError("invalid_refresh_response")
      value = { ...value, refreshCredential: next.refreshCredential, accessToken: next.cloudSessionToken, expiresAtMs }
      delete value.pendingRotationId
      await this.write(value)
      return value
    })
  }
  private async write(value: CloudClientCredential) {
    await mkdir(path.dirname(this.filePath), { recursive: true, mode: 0o700 })
    const temporary = `${this.filePath}.${randomBytes(16).toString("hex")}.tmp`
    const handle = await open(temporary, "wx", 0o600)
    try {
      try { await handle.writeFile(JSON.stringify({ ...value, profile: publicRelayCloudProfile(value.profile) })); await handle.sync() }
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
