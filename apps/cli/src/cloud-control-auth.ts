import type { RelayCloudProfile } from "./preferences.js"

/** Private client/browser authority passed explicitly to control-plane adapters.
 * It is never a kernel status profile or a CLI preference. */
export type CloudControlProfile = RelayCloudProfile & {
  cloudSessionToken?: string
  authenticatedFetch?: (url: string | URL, options?: RequestInit) => Promise<Response>
}

/** MP-08 / MP-11: a long-running command resolves private authority per request.
 * The same serialized body is reused only after a rejected-access response. */
export function cloudControlFetch(profile: CloudControlProfile, url: string | URL, options: RequestInit = {}): Promise<Response> {
  if (profile.authenticatedFetch) return profile.authenticatedFetch(url, options)
  const headers = new Headers(options.headers)
  for (const [name, value] of new Headers(cloudControlHeaders(profile))) headers.set(name, value)
  return fetch(url, {...options, headers, redirect: options.redirect === "manual" ? "manual" : "error"})
}

export function cloudControlHeaders(profile: CloudControlProfile): HeadersInit {
  const session = profile.cloudSessionToken?.trim()
  if (!session) throw new Error("Cloud control-plane actions require a client/browser login; kernel enrollment does not confer human account authority")
  return {accept: "application/json", "content-type": "application/json", authorization: `Bearer ${session}`}
}
