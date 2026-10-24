import type { FastifyRequest } from "fastify"

const LOOPBACK_HOST = /^(127\.0\.0\.1|localhost|\[::1\])(:\d{1,5})?$/i

const header = (request: FastifyRequest, name: string) => {
  const value = request.headers[name]
  return Array.isArray(value) ? value[0] : value
}

export function invokesTrigger(request: FastifyRequest, base: string): boolean {
  if (request.method === "POST") return true
  const path = request.url.split("?")[0]
  return request.method === "GET" && path === base && Object.hasOwn(request.query as Record<string, unknown>, "prompt")
}

/**
 * Why: binding to loopback does not authenticate the website making a request.
 * A foreign Host is DNS rebinding; a foreign Origin or cross-site fetch is
 * another page in the person's browser. Plain navigations from other pages
 * (the Chariox History link) may read the page but never invoke the trigger.
 * Returns the refusal reason, or null to let the request through.
 */
export function foreignRequestReason(request: FastifyRequest, base: string): string | null {
  const host = header(request, "host")
  if (!host || !LOOPBACK_HOST.test(host)) return "foreign host"
  const origin = header(request, "origin")
  if (origin !== undefined && origin !== `http://${host}`) return "foreign origin"
  const site = header(request, "sec-fetch-site")
  if (site === undefined || site === "same-origin" || site === "none") return null
  if (invokesTrigger(request, base)) return "cross-site invocation"
  return header(request, "sec-fetch-mode") === "navigate" ? null : "cross-site fetch"
}
