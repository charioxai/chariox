import { createHmac, createPublicKey, timingSafeEqual, verify as verifySignature } from "node:crypto"
import process from "node:process"

import { authenticatePublicationSender } from "./kernel-publication-client.js"
import type {
  ApiKeyPrincipal,
  ArrobaAuthConfig,
  AuthConfig,
  BearerPrincipal,
  ConnectorConfig,
  ConnectorKind,
  ExternalIdentityBinding,
  GatewayDeps,
  GatewayRequest,
  PrincipalRef,
  VerifiedExternalIdentity,
  WorkflowPublicationConfig,
} from "./publication-types.js"

type AuthResult = { ok: true; caller: Record<string, unknown> } | { ok: false; message: string }
type GatewayReply = { code: (code: number) => unknown; headers: (headers: Record<string, string>) => unknown }

export async function authenticateRequest(
  request: GatewayRequest,
  publication: WorkflowPublicationConfig,
  config: AuthConfig,
  deps: GatewayDeps,
): Promise<AuthResult> {
  if (config.mode === "anonymous") {
    return { ok: true, caller: { type: "anonymous" } }
  }
  if (config.mode === "bearer") {
    const expected = readRequiredEnv(config.token_env)
    const actual = firstHeaderValue(request.headers.authorization)
    if (!safeEqualString(actual, `Bearer ${expected}`)) {
      return { ok: false, message: "missing or invalid bearer token" }
    }
    return { ok: true, caller: principalCaller(config.principal, { auth: "bearer", token_env: config.token_env, connector: "http" }) }
  }
  if (config.mode === "api_key") {
    const expected = readRequiredEnv(config.key_env)
    const actual = headerValue(request, config.header)
    if (!safeEqualString(actual, expected)) {
      return { ok: false, message: "missing or invalid API key" }
    }
    return { ok: true, caller: principalCaller(config.principal, { auth: "api_key", key_env: config.key_env, connector: "http" }) }
  }
  return authenticateArrobaRequest(request, publication, config, deps)
}

export function handleConnectorHandshake(
  request: GatewayRequest,
  reply: GatewayReply,
  publication: WorkflowPublicationConfig,
): { handled: true; payload: unknown } | { handled: false } {
  const connectors = publication.auth?.mode === "arroba" ? publication.auth.connectors ?? [] : []
  for (const connector of connectors) {
    const response = connector.kind === "slack"
      ? handleSlackHandshake(request, connector, reply)
      : connector.kind === "discord"
        ? handleDiscordHandshake(request, connector, reply)
        : connector.kind === "whatsapp"
          ? handleWhatsAppHandshake(request, connector, reply)
          : { handled: false as const }
    if (response.handled) return response
  }
  return { handled: false }
}

export function isPairedSenderAuthEnabled(auth: AuthConfig | undefined) {
  return auth?.mode === "arroba" && auth.paired_senders?.enabled === true && auth.paired_senders.pair_endpoint !== false
}

export function objectBody(body: unknown): Record<string, unknown> {
  return body && typeof body === "object" && !Array.isArray(body) ? body as Record<string, unknown> : {}
}

async function authenticateArrobaRequest(
  request: GatewayRequest,
  publication: WorkflowPublicationConfig,
  config: ArrobaAuthConfig,
  deps: GatewayDeps,
): Promise<AuthResult> {
  const pairedSender = await authenticatePairedSender(request, publication, config, deps)
  if (pairedSender) return pairedSender

  const bearer = authenticateBearerPrincipals(request, config.bearer_tokens ?? [])
  if (bearer) return bearer

  const apiKey = authenticateApiKeyPrincipals(request, config.api_keys ?? [])
  if (apiKey) return apiKey

  const identity = verifyConnectorIdentity(request, config.connectors ?? [])
  if (identity) {
    const binding = findExternalIdentityBinding(config.external_identities ?? [], identity)
    if (!binding) {
      return {
        ok: false,
        message: `verified ${identity.connector} identity is not linked to an Arroba principal`,
      }
    }
    if (!principalAllowsConnector(binding.principal, identity.connector)) {
      return {
        ok: false,
        message: `principal is not allowed through ${identity.connector}`,
      }
    }
    return {
      ok: true,
      caller: principalCaller(binding.principal, {
        auth: "connector",
        connector: identity.connector,
        external_id: identity.external_id,
        metadata: identity.metadata ?? {},
      }),
    }
  }

  if (config.allow_anonymous) {
    return {
      ok: true,
      caller: principalCaller(config.anonymous_principal ?? { id: "anonymous", type: "anonymous" }, {
        auth: "anonymous",
        connector: "http",
      }),
    }
  }

  return { ok: false, message: "request is not authorized for this publication" }
}

async function authenticatePairedSender(
  request: GatewayRequest,
  publication: WorkflowPublicationConfig,
  config: ArrobaAuthConfig,
  deps: GatewayDeps,
): Promise<AuthResult | null> {
  if (!config.paired_senders?.enabled) return null
  const header = config.paired_senders.header ?? "authorization"
  const rawCredential = headerValue(request, header)
  const credential = header.toLowerCase() === "authorization"
    ? rawCredential?.replace(/^Bearer\s+/i, "")
    : rawCredential
  if (!credential) return null
  try {
    const transport = config.paired_senders.transport ?? "http"
    const sender = deps.authenticatePublicationSender
      ? await deps.authenticatePublicationSender(publication, credential, transport)
      : await authenticatePublicationSender(publication, credential, transport)
    const principal: PrincipalRef = {
      id: sender.sender_id,
      type: "sender",
    }
    if (sender.display_name) principal.display_name = sender.display_name
    if (sender.allowed_transports?.length) {
      principal.allowed_connectors = sender.allowed_transports as ConnectorKind[]
    }
    return {
      ok: true,
      caller: principalCaller(principal, {
        auth: "paired_sender",
        connector: transport,
        sender_id: sender.sender_id,
      }),
    }
  } catch (error) {
    return {
      ok: false,
      message: error instanceof Error ? error.message : "sender credential was not authorized",
    }
  }
}

function authenticateBearerPrincipals(request: GatewayRequest, tokens: BearerPrincipal[]) {
  const actual = firstHeaderValue(request.headers.authorization)
  for (const token of tokens) {
    const expected = `Bearer ${readRequiredEnv(token.token_env)}`
    if (safeEqualString(actual, expected)) {
      return { ok: true as const, caller: principalCaller(token.principal, { auth: "bearer", token_env: token.token_env, connector: "http" }) }
    }
  }
  return null
}

function authenticateApiKeyPrincipals(request: GatewayRequest, apiKeys: ApiKeyPrincipal[]) {
  for (const apiKey of apiKeys) {
    const actual = headerValue(request, apiKey.header)
    const expected = readRequiredEnv(apiKey.key_env)
    if (safeEqualString(actual, expected)) {
      return { ok: true as const, caller: principalCaller(apiKey.principal, { auth: "api_key", key_env: apiKey.key_env, connector: "http" }) }
    }
  }
  return null
}

function verifyConnectorIdentity(request: GatewayRequest, connectors: ConnectorConfig[]): VerifiedExternalIdentity | null {
  for (const connector of connectors) {
    const identity = verifySingleConnectorIdentity(request, connector)
    if (identity) return identity
  }
  return null
}

function handleSlackHandshake(
  request: GatewayRequest,
  connector: Extract<ConnectorConfig, { kind: "slack" }>,
  reply: GatewayReply,
): { handled: true; payload: unknown } | { handled: false } {
  const body = objectBody(request.body)
  if (body.type !== "url_verification" || typeof body.challenge !== "string") {
    return { handled: false }
  }
  if (connector.signing_secret_env && !verifySlackSignature(request, readRequiredEnv(connector.signing_secret_env))) {
    reply.code(401)
    reply.headers({ "content-type": "application/json" })
    return { handled: true, payload: { error: "invalid slack signature" } }
  }
  reply.code(200)
  reply.headers({ "content-type": "text/plain" })
  return { handled: true, payload: body.challenge }
}

function handleDiscordHandshake(
  request: GatewayRequest,
  connector: Extract<ConnectorConfig, { kind: "discord" }>,
  reply: GatewayReply,
): { handled: true; payload: unknown } | { handled: false } {
  const body = objectBody(request.body)
  if (body.type !== 1) {
    return { handled: false }
  }
  if (connector.public_key_env && !verifyDiscordSignature(request, readRequiredEnv(connector.public_key_env))) {
    reply.code(401)
    reply.headers({ "content-type": "application/json" })
    return { handled: true, payload: { error: "invalid discord signature" } }
  }
  reply.code(200)
  reply.headers({ "content-type": "application/json" })
  return { handled: true, payload: { type: 1 } }
}

function handleWhatsAppHandshake(
  request: GatewayRequest,
  connector: Extract<ConnectorConfig, { kind: "whatsapp" }>,
  reply: GatewayReply,
): { handled: true; payload: unknown } | { handled: false } {
  const query = objectBody(request.query)
  if (request.method.toUpperCase() !== "GET" || query["hub.mode"] !== "subscribe") {
    return { handled: false }
  }
  const challenge = typeof query["hub.challenge"] === "string" ? query["hub.challenge"] : ""
  const expectedToken = connector.verify_token_env ? readRequiredEnv(connector.verify_token_env) : null
  const actualToken = typeof query["hub.verify_token"] === "string" ? query["hub.verify_token"] : undefined
  if (expectedToken && !safeEqualString(actualToken, expectedToken)) {
    reply.code(401)
    reply.headers({ "content-type": "application/json" })
    return { handled: true, payload: { error: "invalid whatsapp verify token" } }
  }
  reply.code(200)
  reply.headers({ "content-type": "text/plain" })
  return { handled: true, payload: challenge }
}

function verifySingleConnectorIdentity(request: GatewayRequest, connector: ConnectorConfig): VerifiedExternalIdentity | null {
  if (connector.kind === "http") {
    return connector.principal ? { connector: "http", external_id: connector.principal.id } : null
  }
  if (connector.kind === "telegram") return verifyTelegramIdentity(request, connector)
  if (connector.kind === "slack") return verifySlackIdentity(request, connector)
  if (connector.kind === "discord") return verifyDiscordIdentity(request, connector)
  if (connector.kind === "whatsapp") return verifyWhatsAppIdentity(request, connector)
  return verifySignalIdentity(request, connector)
}

function verifyWhatsAppIdentity(request: GatewayRequest, connector: Extract<ConnectorConfig, { kind: "whatsapp" }>): VerifiedExternalIdentity | null {
  if (connector.app_secret_env) {
    const appSecret = readRequiredEnv(connector.app_secret_env)
    if (!verifyWhatsAppSignature(request, appSecret)) return null
  }
  const value = firstWhatsAppValue(objectBody(request.body))
  const message = Array.isArray(value?.messages) ? value.messages[0] as Record<string, unknown> | undefined : undefined
  const contact = Array.isArray(value?.contacts) ? value.contacts[0] as Record<string, unknown> | undefined : undefined
  const externalId = String(message?.from ?? contact?.wa_id ?? "")
  if (!externalId) return null
  const metadata = value?.metadata as Record<string, unknown> | undefined
  return {
    connector: "whatsapp",
    external_id: externalId,
    metadata: {
      phone_number_id: metadata?.phone_number_id,
      display_phone_number: metadata?.display_phone_number,
    },
  }
}

function firstWhatsAppValue(body: Record<string, unknown>): Record<string, unknown> | undefined {
  const entry = Array.isArray(body.entry) ? body.entry[0] as Record<string, unknown> | undefined : undefined
  const change = Array.isArray(entry?.changes) ? entry.changes[0] as Record<string, unknown> | undefined : undefined
  return change?.value as Record<string, unknown> | undefined
}

function verifySignalIdentity(request: GatewayRequest, connector: Extract<ConnectorConfig, { kind: "signal" }>): VerifiedExternalIdentity | null {
  if (connector.webhook_secret_env) {
    const expected = readRequiredEnv(connector.webhook_secret_env)
    const actual = headerValue(request, "x-signal-webhook-secret")
    if (!safeEqualString(actual, expected)) return null
  }
  const body = objectBody(request.body)
  const envelope = body.envelope as Record<string, unknown> | undefined
  const externalId = String(
    envelope?.sourceUuid
      ?? envelope?.sourceNumber
      ?? envelope?.source
      ?? body.sourceUuid
      ?? body.sourceNumber
      ?? body.source
      ?? "",
  )
  if (!externalId) return null
  return {
    connector: "signal",
    external_id: externalId,
    metadata: {
      source_number: envelope?.sourceNumber ?? body.sourceNumber,
      source_uuid: envelope?.sourceUuid ?? body.sourceUuid,
    },
  }
}

function verifyDiscordIdentity(request: GatewayRequest, connector: Extract<ConnectorConfig, { kind: "discord" }>): VerifiedExternalIdentity | null {
  if (connector.public_key_env) {
    const publicKeyHex = readRequiredEnv(connector.public_key_env)
    if (!verifyDiscordSignature(request, publicKeyHex)) return null
  }
  const body = objectBody(request.body)
  const member = body.member as Record<string, unknown> | undefined
  const user = (member?.user as Record<string, unknown> | undefined)
    ?? (body.user as Record<string, unknown> | undefined)
  const userId = String(user?.id ?? "")
  if (!userId) return null
  const guildId = String(body.guild_id ?? "")
  return {
    connector: "discord",
    external_id: guildId ? `${guildId}:${userId}` : userId,
    metadata: {
      guild_id: guildId || undefined,
      user_id: userId,
      username: user?.username,
    },
  }
}

function verifyTelegramIdentity(request: GatewayRequest, connector: Extract<ConnectorConfig, { kind: "telegram" }>): VerifiedExternalIdentity | null {
  if (connector.webhook_secret_env) {
    const expected = readRequiredEnv(connector.webhook_secret_env)
    const actual = headerValue(request, "x-telegram-bot-api-secret-token")
    if (!safeEqualString(actual, expected)) return null
  }
  const body = objectBody(request.body)
  const from = (body.message as Record<string, unknown> | undefined)?.from
    ?? (body.callback_query as Record<string, unknown> | undefined)?.from
    ?? (body.edited_message as Record<string, unknown> | undefined)?.from
  const userId = String((from as Record<string, unknown> | undefined)?.id ?? "")
  if (!userId) return null
  return {
    connector: "telegram",
    external_id: userId,
    metadata: {
      username: (from as Record<string, unknown> | undefined)?.username,
      chat_id: (body.message as Record<string, unknown> | undefined)?.chat && String(((body.message as Record<string, unknown>).chat as Record<string, unknown>).id ?? ""),
    },
  }
}

function verifySlackIdentity(request: GatewayRequest, connector: Extract<ConnectorConfig, { kind: "slack" }>): VerifiedExternalIdentity | null {
  if (connector.signing_secret_env) {
    const expected = readRequiredEnv(connector.signing_secret_env)
    if (!verifySlackSignature(request, expected)) return null
  }
  const body = objectBody(request.body)
  const event = body.event as Record<string, unknown> | undefined
  const teamId = String(body.team_id ?? (body.team as Record<string, unknown> | undefined)?.id ?? "")
  const userId = String(body.user_id ?? body.user ?? event?.user ?? event?.user_id ?? "")
  if (!userId) return null
  return {
    connector: "slack",
    external_id: teamId ? `${teamId}:${userId}` : userId,
    metadata: { team_id: teamId || undefined, user_id: userId },
  }
}

function findExternalIdentityBinding(bindings: ExternalIdentityBinding[], identity: VerifiedExternalIdentity) {
  return bindings.find((binding) =>
    binding.connector === identity.connector && binding.external_id === identity.external_id
  )
}

function principalAllowsConnector(principal: PrincipalRef, connector: ConnectorKind) {
  return !principal.allowed_connectors?.length || principal.allowed_connectors.includes(connector)
}

function principalCaller(principal: PrincipalRef | undefined, proof: Record<string, unknown>) {
  if (!principal) return { type: proof.auth, ...proof }
  return {
    type: principal.type ?? "user",
    principal_id: principal.id,
    teams: principal.teams ?? [],
    display_name: principal.display_name,
    allowed_connectors: principal.allowed_connectors,
    proof,
  }
}

function verifySlackSignature(request: GatewayRequest, signingSecret: string) {
  const timestamp = headerValue(request, "x-slack-request-timestamp")
  const signature = headerValue(request, "x-slack-signature")
  const rawBody = rawRequestBody(request)
  if (!timestamp || !signature || rawBody === undefined) return false
  const timestampSeconds = Number(timestamp)
  if (!Number.isFinite(timestampSeconds)) return false
  const maxSkewSeconds = 60 * 5
  if (Math.abs(Math.floor(Date.now() / 1000) - timestampSeconds) > maxSkewSeconds) return false
  const base = `v0:${timestamp}:${rawBody}`
  const expected = `v0=${createHmac("sha256", signingSecret).update(base).digest("hex")}`
  return safeEqualString(signature, expected)
}

function verifyDiscordSignature(request: GatewayRequest, publicKeyHex: string) {
  const timestamp = headerValue(request, "x-signature-timestamp")
  const signatureHex = headerValue(request, "x-signature-ed25519")
  const rawBody = rawRequestBody(request)
  if (!timestamp || !signatureHex || rawBody === undefined) return false
  try {
    const publicKey = createPublicKey({
      key: Buffer.concat([
        Buffer.from("302a300506032b6570032100", "hex"),
        Buffer.from(publicKeyHex, "hex"),
      ]),
      format: "der",
      type: "spki",
    })
    return verifySignature(
      null,
      Buffer.from(`${timestamp}${rawBody}`),
      publicKey,
      Buffer.from(signatureHex, "hex"),
    )
  } catch {
    return false
  }
}

function verifyWhatsAppSignature(request: GatewayRequest, appSecret: string) {
  const signature = headerValue(request, "x-hub-signature-256")
  const rawBody = rawRequestBody(request)
  if (!signature || rawBody === undefined) return false
  const expected = `sha256=${createHmac("sha256", appSecret).update(rawBody).digest("hex")}`
  return safeEqualString(signature, expected)
}

function headerValue(request: GatewayRequest, header: string) {
  const value = request.headers[header.toLowerCase()]
  return firstHeaderValue(value)
}

function firstHeaderValue(value: string | string[] | undefined) {
  return Array.isArray(value) ? value[0] : value
}

function readRequiredEnv(name: string) {
  const value = process.env[name]
  if (!value) throw new Error(`required env ${name} is not set`)
  return value
}

function rawRequestBody(request: GatewayRequest) {
  return (request.raw as { arrobaRawBody?: string } | undefined)?.arrobaRawBody
}

function safeEqualString(actual: string | undefined, expected: string) {
  if (!actual) return false
  const actualBuffer = Buffer.from(actual)
  const expectedBuffer = Buffer.from(expected)
  if (actualBuffer.length !== expectedBuffer.length) return false
  return timingSafeEqual(actualBuffer, expectedBuffer)
}
