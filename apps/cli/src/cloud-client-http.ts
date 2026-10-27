export class CloudClientAuthError extends Error {
  constructor(readonly code: string) { super(code === "profile_conflict" ? "Cloud profile conflict (profile_conflict); use a separate CHARIOX_HOME" : `Cloud client authentication failed (${code})`) }
}

/** Cloud ended this login: revoked client, reused refresh, or the account signed out. */
export const endsCloudLogin = (code: unknown): boolean => code === "client_revoked" || code === "refresh_reuse_detected" || code === "signed_out"

/** BY-03: `details.reason = "signed_out"` is the definitive answer whatever the status code. */
export function cloudErrorCode(body: { error?: { code?: unknown; details?: { reason?: unknown } } } | null | undefined): unknown {
  return body?.error?.details?.reason === "signed_out" ? "signed_out" : body?.error?.code
}

export async function cloudClientRequest<T>(apiUrl: string, pathname: string, options: { body?: unknown; accessToken?: string } = {}): Promise<T> {
  const url = new URL(pathname, apiUrl)
  if (url.username || url.password || (url.protocol !== "https:" && !(url.protocol === "http:" && ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname)))) throw new CloudClientAuthError("insecure_auth_endpoint")
  const response = await fetch(url, {
    method: options.body === undefined ? "GET" : "POST", redirect: "error",
    headers: { "content-type": "application/json", ...(options.accessToken ? { authorization: `Bearer ${options.accessToken}` } : {}) },
    ...(options.body === undefined ? {} : { body: JSON.stringify(options.body) }), signal: AbortSignal.timeout(20_000),
  })
  if (!response.ok) {
    const code = cloudErrorCode(await response.json().catch(() => null))
    throw new CloudClientAuthError(typeof code === "string" && /^[a-z_]{1,64}$/.test(code) ? code : `http_${response.status}`)
  }
  return (response.status === 204 ? undefined : await response.json()) as T
}
