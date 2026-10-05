import type { LocalIpcClient, RelayClientIdentity } from "./ipc.js"
import { issueKernelCloudRelayClientToken, requireRelayTokenKeyBinding } from "./relay-api.js"

// Read only scheduling/binding metadata. The relay verifies the signature.
function grantMetadata(token: string, identity: RelayClientIdentity, target: string) {
  requireRelayTokenKeyBinding(token, identity.publicKeyThumbprint, "paired terminal renewal")
  const parts = token.split(".")
  const claims = JSON.parse(Buffer.from(parts[1]!, "base64url").toString("utf8"))
  const expiry = parts[0] === "chariox-scoped-v1" ? claims.expires_at_ms : claims.exp * 1_000
  const subject = claims.sub ?? claims.subject
  if (!Number.isFinite(expiry) || expiry <= Date.now() || typeof subject !== "string" || !subject
      || !Array.isArray(claims.allowed_targets) || !claims.allowed_targets.includes(target)) {
    throw new Error("paired terminal grant has invalid expiry, subject or target")
  }
  return {expiry, subject}
}

export function startPairedRelayRenewal(client: LocalIpcClient, input: {
  token: string; subject: string; endpoint: string; target: string; terminalId: string; identity: RelayClientIdentity
  createClient: (endpoint: string, options: {relayAuthToken: string; targetDaemonId: string; relayIdentity: RelayClientIdentity}) => LocalIpcClient
  bootstrapToken?: () => Promise<string>
}) {
  let token = input.token
  const initial = grantMetadata(token, input.identity, input.target)
  if (initial.subject !== input.subject) throw new Error("paired terminal grant changed its subject")
  let expiry = initial.expiry
  client.startRelayAuthRenewal(expiry, async () => {
    // A separate short-lived control client avoids asking this renewal to wait
    // on itself when the visible client's control socket needs reconnecting.
    const authToken = expiry > Date.now() ? token : await input.bootstrapToken?.()
    if (!authToken) throw new Error("paired terminal bootstrap authority has expired")
    const issuer = input.createClient(input.endpoint, {relayAuthToken: authToken, targetDaemonId: input.target, relayIdentity: input.identity})
    try {
      const fresh = await issueKernelCloudRelayClientToken(issuer, input.target, input.terminalId, null, input.identity.publicKeyThumbprint)
      const metadata = grantMetadata(fresh.relayToken, input.identity, input.target)
      if (metadata.subject !== initial.subject || fresh.relayUrl !== input.endpoint) throw new Error("paired terminal renewal changed its subject or relay")
      token = fresh.relayToken
      expiry = Math.min(metadata.expiry, fresh.tokenExpiresAtMs)
      return {token, expiresAtMs: expiry}
    } finally { await issuer.close() }
  })
}
