import {createHash, timingSafeEqual} from "node:crypto"
import {readFileSync, statSync} from "node:fs"
import {MAX_RELAY_TOKEN_LENGTH} from "@chariox/kernel-client/relay-authorization"
import type {AgentTriggerRelayBootstrap} from "@chariox/kernel-client/agent-trigger-ingress-client"
import type {AgentTriggerBinding} from "./hosted-agent-trigger-invoke.js"

export type TriggerDeployment = AgentTriggerBinding & {id: string}
export type AdmissionAccess = "read" | "operate"
export type AdmissionCall = {deploymentId: string; access: AdmissionAccess; method: string; headers: Record<string, string>; thumbprint?: string}
/** What the ingress needs back: a status, headers and cookies to relay to the caller, and on success the binding. */
export type AdmissionReply = {status: number; headers: Record<string, string>; cookies: string[]; value?: {deployment?: TriggerDeployment; bootstrap?: AgentTriggerRelayBootstrap}}
/**
 * Decides who may open or fire a trigger deployment and where its kernel is. Only authentication and binding
 * metadata cross this seam, never request bodies or query values.
 */
export type TriggerAdmission = {
  /** Request headers (lower-case) handed to `admit`. */
  forwardedHeaders: readonly string[]
  admit(call: AdmissionCall): Promise<AdmissionReply>
}

const loopbackHosts = ["127.0.0.1", "localhost", "[::1]"]
export const isLoopbackUrl = (url: URL) => loopbackHosts.includes(url.hostname)

/** The admission service is reached at a configured URL; `{id}` stands for the deployment id. */
export function remoteAdmission(options: {url: string; publicOrigin: URL; forwardedHeaders: readonly string[]}): TriggerAdmission {
  if (!options.url.includes("{id}")) throw Error("Set the admission URL with an {id} placeholder")
  const probe = new URL(options.url.replaceAll("{id}", "probe"))
  if (probe.protocol !== "https:" && !(probe.protocol === "http:" && isLoopbackUrl(probe))) throw Error("Admission requires HTTPS or a loopback endpoint")
  const {publicOrigin} = options
  return {
    forwardedHeaders: options.forwardedHeaders,
    async admit(call) {
      const target = new URL(options.url.replaceAll("{id}", encodeURIComponent(call.deploymentId)))
      target.searchParams.set("access", call.access); target.searchParams.set("method", call.method)
      const headers = {"content-type": "application/json", host: publicOrigin.host, "x-forwarded-host": publicOrigin.host, "x-forwarded-proto": "https", ...call.headers}
      const response = await fetch(target, {method: "POST", headers, body: JSON.stringify(call.thumbprint ? {publicKeyThumbprint: call.thumbprint} : {}), redirect: "manual", signal: AbortSignal.timeout(15_000)})
      return {status: response.status, headers: Object.fromEntries(response.headers), cookies: response.headers.getSetCookie(), ...(response.ok ? {value: await response.json() as NonNullable<AdmissionReply["value"]>} : {})}
    },
  }
}

export type SelfHostedAdmissionConfig = {
  relay: {url: string; token: string}
  deployments: Array<TriggerDeployment & {accessToken: string}>
}
const digest = (value: string) => createHash("sha256").update(value).digest()

/** Reads and checks the self-hosted deployments file once, at start, so a bad file stops the process. */
export function loadSelfHostedAdmissionConfig(path: string): SelfHostedAdmissionConfig {
  if ((statSync(path).mode & 0o077) !== 0) throw Error(`${path} holds access tokens: make it private (chmod 600)`)
  const raw = JSON.parse(readFileSync(path, "utf8")) as Partial<SelfHostedAdmissionConfig>
  const text = (value: unknown, what: string, min = 1) => {
    if (typeof value !== "string" || value.length < min || value.length >= 256) throw Error(`${path}: ${what} must be a string of ${min} to 255 characters`)
    return value
  }
  // A signed client token (base64url segments joined by dots) is far longer than an id; opaque shared tokens use the same characters or base64.
  const relayToken = (value: unknown) => {
    if (typeof value !== "string" || value.length < 1 || value.length > MAX_RELAY_TOKEN_LENGTH) throw Error(`${path}: relay.token must be a string of 1 to ${MAX_RELAY_TOKEN_LENGTH} characters`)
    if (!/^[A-Za-z0-9._~+/=-]+$/.test(value)) throw Error(`${path}: relay.token may only hold letters, digits and . _ ~ + / = - characters: paste the token without spaces or line breaks`)
    return value
  }
  const relayUrl = new URL(text(raw.relay?.url, "relay.url"))
  if (relayUrl.protocol !== "wss:" && !(relayUrl.protocol === "ws:" && isLoopbackUrl(relayUrl))) throw Error(`${path}: relay.url must be wss:// (ws:// only on loopback)`)
  const relay = {url: relayUrl.href, token: relayToken(raw.relay?.token)}
  if (!Array.isArray(raw.deployments) || raw.deployments.length === 0) throw Error(`${path}: list at least one deployment`)
  const ids = new Set<string>()
  const deployments = raw.deployments.map((item, index) => {
    const label = `deployments[${index}]`
    const id = text(item?.id, `${label}.id`)
    if (ids.has(id)) throw Error(`${path}: deployment id ${id} is listed twice`)
    ids.add(id)
    return {id, kernelId: text(item.kernelId, `${label}.kernelId`), sessionId: text(item.sessionId, `${label}.sessionId`), triggerId: text(item.triggerId, `${label}.triggerId`), accessToken: text(item.accessToken, `${label}.accessToken`, 32)}
  })
  return {relay, deployments}
}

/**
 * Self-hosted rule: a deployment is admitted for a caller that presents its access token as `Authorization: Bearer`.
 * The relay credential of the operator's own relay is handed to the ingress connection and never leaves this process.
 */
export function selfHostedAdmission(config: SelfHostedAdmissionConfig): TriggerAdmission {
  const byId = new Map(config.deployments.map(item => [item.id, item]))
  return {
    forwardedHeaders: ["authorization"],
    async admit(call) {
      const found = byId.get(call.deploymentId)
      if (!found) return {status: 404, headers: {}, cookies: []}
      const presented = /^Bearer (\S+)$/.exec(call.headers.authorization ?? "")?.[1]
      if (!presented || !timingSafeEqual(digest(presented), digest(found.accessToken))) return {status: 401, headers: {}, cookies: []}
      const {accessToken: _token, ...deployment} = found
      return {status: 200, headers: {}, cookies: [], value: {deployment, ...(call.thumbprint ? {bootstrap: {relayUrl: config.relay.url, relayToken: config.relay.token, relayTokenExpiresAt: new Date(Date.now() + 60_000).toISOString(), target: {daemonId: found.kernelId, kernelRef: found.kernelId}}} : {})}}
    },
  }
}
