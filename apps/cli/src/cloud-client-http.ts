export class CloudClientAuthError extends Error {
  constructor(readonly code: string, readonly legacyDevicePollSchemaRejected = false) { super(code === "profile_conflict" ? "Cloud profile conflict (profile_conflict); use a separate CHARIOX_HOME" : `Cloud client authentication failed (${code})`) }
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
    const error = await response.json().catch(() => null) as { error?: { code?: string; message?: string } } | null
    const code = error?.error?.code
    // MP-08/MP-11: keep raw error bodies private while recognizing #898's
    // exact legacy device-poll schema rejection for one capability retry.
    const legacyDevicePollSchemaRejected = pathname === "/auth/device/poll"
      && response.status === 400 && code === "invalid_request"
      && error?.error?.message === "Request validation failed"
    throw new CloudClientAuthError(code && /^[a-z_]{1,64}$/.test(code) ? code : `http_${response.status}`, legacyDevicePollSchemaRejected)
  }
  return (response.status === 204 ? undefined : await response.json()) as T
}
