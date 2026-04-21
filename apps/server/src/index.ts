import { spawn } from "node:child_process"
import { createHmac, createPublicKey, timingSafeEqual, verify as verifySignature } from "node:crypto"
import { readFileSync } from "node:fs"
import { readFile } from "node:fs/promises"
import process from "node:process"

import Fastify from "fastify"

import { LocalIpcClient } from "@arroba/kernel-client/ipc"
import type {
  WorkflowPublicationDefinition,
  WorkflowPublicationSenderCredential,
  WorkflowPublicationTrustedSender,
} from "@arroba/kernel-client/kernel-types"
import {
  authenticateWorkflowPublicationSenderRequest,
  getWorkflowPublicationRequest,
  getWorkflowRunRequest,
  invokeWorkflowEndpointRequest,
  redeemWorkflowPublicationPairCodeRequest,
} from "@arroba/kernel-client/ipc-requests"

import { createProcessLogger } from "./logging.js"

type ParserKind =
  | "json"
  | "query_params"
  | "headers"
  | "regex"
  | "path_template"
  | "webhook"
  | "custom_command"

type AuthConfig =
  | { mode: "anonymous" }
  | { mode: "bearer"; token_env: string; principal?: PrincipalRef }
  | { mode: "api_key"; header: string; key_env: string; principal?: PrincipalRef }
  | ArrobaAuthConfig

type PrincipalRef = {
  id: string
  type?: "user" | "team" | "sender" | "service" | "anonymous"
  teams?: string[]
  display_name?: string
  allowed_connectors?: ConnectorKind[]
}

type ExternalIdentityBinding = {
  connector: ConnectorKind
  external_id: string
  principal: PrincipalRef
}

type BearerPrincipal = {
  token_env: string
  principal: PrincipalRef
}

type ApiKeyPrincipal = {
  header: string
  key_env: string
  principal: PrincipalRef
}

type ConnectorKind = "http" | "slack" | "discord" | "telegram" | "whatsapp" | "signal"

type ConnectorConfig =
  | { kind: "http"; principal?: PrincipalRef }
  | { kind: "slack"; signing_secret_env?: string }
  | { kind: "discord"; public_key_env?: string }
  | { kind: "telegram"; webhook_secret_env?: string }
  | { kind: "whatsapp"; app_secret_env?: string; verify_token_env?: string }
  | { kind: "signal"; webhook_secret_env?: string }

type ArrobaAuthConfig = {
  mode: "arroba"
  allow_anonymous?: boolean
  anonymous_principal?: PrincipalRef
  paired_senders?: PairedSendersAuthConfig
  bearer_tokens?: BearerPrincipal[]
  api_keys?: ApiKeyPrincipal[]
  external_identities?: ExternalIdentityBinding[]
  connectors?: ConnectorConfig[]
}

type PairedSendersAuthConfig = {
  enabled?: boolean
  header?: string
  transport?: ConnectorKind
  pair_endpoint?: boolean
}

type ParserConfig = {
  kind: ParserKind
  source?: "body" | "path" | "query" | "headers" | "request"
  pattern?: string
  template?: string
  command?: string
  args?: string[]
}

type InputSchema = {
  type?: "object"
  required?: string[]
  properties?: Record<string, { type?: "string" | "number" | "boolean" | "object" | "array" }>
}

type TlsConfig = {
  enabled?: boolean
  key_file?: string
  cert_file?: string
}

export type WorkflowPublicationConfig = {
  publication_id: string
  session_id: string
  workflow_ref: string
  endpoint_ref: string
  kernel_endpoint?: string
  route?: string
  methods?: Array<"GET" | "POST">
  auth?: AuthConfig
  parser?: ParserConfig
  input_schema?: InputSchema
  tls?: TlsConfig
  mode?: "sync" | "async"
  sync_timeout_ms?: number
  poll_ms?: number
}

type NormalizedInvocation = {
  publication_id: string
  request_id: string
  caller: Record<string, unknown>
  input: unknown
  mode: "sync" | "async"
}

type WorkflowRun = {
  id: string
  status: string
  workflow_id?: string
  endpoint_id?: string
  final_output?: { message: string; artifacts?: unknown[] } | null
}

type WorkflowInvocationResult = {
  accepted: boolean
  queued?: boolean
  workflow_run?: WorkflowRun
  response?: unknown
}

type GatewayDeps = {
  invokeWorkflow?: (invocation: NormalizedInvocation) => Promise<WorkflowInvocationResult>
  redeemPublicationPairCode?: (
    publication: WorkflowPublicationConfig,
    pairCode: string,
    displayName?: string | null,
  ) => Promise<WorkflowPublicationSenderCredential>
  authenticatePublicationSender?: (
    publication: WorkflowPublicationConfig,
    credential: string,
    transport: ConnectorKind,
  ) => Promise<WorkflowPublicationTrustedSender>
}

type KernelLookupClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  close?: () => Promise<void>
}

type VerifiedExternalIdentity = {
  connector: ConnectorKind
  external_id: string
  metadata?: Record<string, unknown>
}

type GatewayRequest = {
  method: string
  url: string
  headers: Record<string, string | string[] | undefined>
  body?: unknown
  query?: unknown
  raw: unknown
}

export const buildServer = (config?: WorkflowPublicationConfig, deps: GatewayDeps = {}) => {
  const processLogger = createProcessLogger("workflow-gateway")
  const logger = processLogger.child("gateway.http")
  const publication = config ?? defaultPublicationConfig()
  const httpsOptions = resolveHttpsOptions(publication.tls)
  const app = Fastify({ logger: false, ...(httpsOptions ? { https: httpsOptions } : {}) } as never)
  installRawBodyParsers(app)

  app.get("/health", async () => {
    logger.debug("handled health request")
    return { status: "ok" }
  })

  if (isPairedSenderAuthEnabled(publication.auth)) {
    app.post("/.well-known/arroba/publication/pair", async (request, reply) => {
      const body = objectBody(request.body)
      const pairCode = String(body.pair_code ?? "")
      if (!pairCode) {
        reply.code(400)
        return { error: "missing pair_code" }
      }
      const senderCredential = deps.redeemPublicationPairCode
        ? await deps.redeemPublicationPairCode(publication, pairCode, optionalString(body.display_name))
        : await redeemPublicationPairCode(publication, pairCode, optionalString(body.display_name))
      return {
        sender: senderCredential.sender,
        credential: senderCredential.credential,
      }
    })
  } else {
    app.post("/.well-known/arroba/publication/pair", async (_request, reply) => {
      reply.code(404)
      return { error: "publication pairing is not enabled" }
    })
  }

  const methods = publication.methods?.length ? publication.methods : ["GET", "POST"]
  for (const method of methods) {
    app.route({
      method,
      url: publication.route ?? "/*",
      handler: async (request, reply) => {
        const handshake = handleConnectorHandshake(request as unknown as GatewayRequest, reply, publication)
        if (handshake.handled) return handshake.payload

        const auth = await authenticateRequest(
          request as unknown as GatewayRequest,
          publication,
          publication.auth ?? { mode: "anonymous" },
          deps,
        )
        if (!auth.ok) {
          reply.code(401).headers({ "content-type": "application/json" })
          return { error: auth.message }
        }

        const parsed = await parseAndValidateRequest(request as unknown as GatewayRequest, publication).catch((error) => {
          reply.code(400).headers({ "content-type": "application/json" })
          return { __arroba_parse_error: error instanceof Error ? error.message : String(error) }
        })
        if (isParseErrorPayload(parsed)) {
          return { error: parsed.__arroba_parse_error }
        }
        const invocation: NormalizedInvocation = {
          publication_id: publication.publication_id,
          request_id: `req_${Date.now()}_${Math.random().toString(16).slice(2)}`,
          caller: auth.caller,
          input: parsed,
          mode: publication.mode ?? "sync",
        }
        const result = deps.invokeWorkflow
          ? await deps.invokeWorkflow(invocation)
          : await invokeKernelWorkflow(publication, invocation)
        return forwardWorkflowResult(reply, result)
      },
    })
  }

  app.addHook("onClose", async () => {
    logger.info("gateway closed")
  })

  return { app, logger }
}

async function invokeKernelWorkflow(
  publication: WorkflowPublicationConfig,
  invocation: NormalizedInvocation,
): Promise<WorkflowInvocationResult> {
  const client = new LocalIpcClient(publication.kernel_endpoint ?? defaultKernelEndpoint())
  try {
    const prompt = JSON.stringify(invocation, null, 2)
    const response = await client.send<Record<string, unknown>>(invokeWorkflowEndpointRequest(
      publication.session_id,
      publication.workflow_ref,
      publication.endpoint_ref,
      prompt,
    ))
    if ("WorkflowRunQueued" in response) {
      return { accepted: true, queued: true, response: response.WorkflowRunQueued }
    }
    const invoked = response.WorkflowRunInvoked as { workflow_run: WorkflowRun } | undefined
    if (!invoked?.workflow_run) {
      throw new Error(`unexpected workflow invoke response: ${JSON.stringify(response)}`)
    }
    if ((publication.mode ?? "sync") === "async") {
      return { accepted: true, workflow_run: invoked.workflow_run }
    }
    const workflowRun = await waitForWorkflowRun(client, publication, invoked.workflow_run.id)
    return { accepted: true, workflow_run: workflowRun }
  } finally {
    await client.close().catch(() => {})
  }
}

async function redeemPublicationPairCode(
  publication: WorkflowPublicationConfig,
  pairCode: string,
  displayName?: string | null,
): Promise<WorkflowPublicationSenderCredential> {
  const client = new LocalIpcClient(publication.kernel_endpoint ?? defaultKernelEndpoint())
  try {
    const response = await client.send<Record<string, unknown>>(redeemWorkflowPublicationPairCodeRequest(
      publication.session_id,
      publication.publication_id,
      pairCode,
      displayName ?? null,
      ["http"],
    ))
    const payload = response.WorkflowPublicationSenderPaired as { sender_credential?: WorkflowPublicationSenderCredential } | undefined
    if (!payload?.sender_credential) {
      throw new Error(`unexpected publication pair response: ${JSON.stringify(response)}`)
    }
    return payload.sender_credential
  } finally {
    await client.close().catch(() => {})
  }
}

async function authenticatePublicationSender(
  publication: WorkflowPublicationConfig,
  credential: string,
  transport: ConnectorKind,
): Promise<WorkflowPublicationTrustedSender> {
  const client = new LocalIpcClient(publication.kernel_endpoint ?? defaultKernelEndpoint())
  try {
    const response = await client.send<Record<string, unknown>>(authenticateWorkflowPublicationSenderRequest(
      publication.session_id,
      publication.publication_id,
      credential,
      transport,
    ))
    const payload = response.WorkflowPublicationSenderAuthenticated as { sender?: WorkflowPublicationTrustedSender } | undefined
    if (!payload?.sender) {
      throw new Error(`unexpected publication sender auth response: ${JSON.stringify(response)}`)
    }
    return payload.sender
  } finally {
    await client.close().catch(() => {})
  }
}

async function waitForWorkflowRun(
  client: LocalIpcClient,
  publication: WorkflowPublicationConfig,
  workflowRunId: string,
) {
  const timeoutMs = publication.sync_timeout_ms ?? 30_000
  const pollMs = publication.poll_ms ?? 500
  const deadline = Date.now() + timeoutMs
  let latest: WorkflowRun | null = null
  while (Date.now() < deadline) {
    const response = await client.send<Record<string, unknown>>(
      getWorkflowRunRequest(publication.session_id, workflowRunId),
    )
    latest = (response.WorkflowRun as { workflow_run: WorkflowRun } | undefined)?.workflow_run ?? null
    if (latest && isTerminalWorkflowRunStatus(latest.status)) return latest
    await sleep(pollMs)
  }
  return latest ?? { id: workflowRunId, status: "unknown" }
}

function forwardWorkflowResult(reply: { code: (code: number) => unknown; headers: (headers: Record<string, string>) => unknown }, result: WorkflowInvocationResult) {
  const workflowRun = result.workflow_run
  const finalMessage = workflowRun?.final_output?.message
  const transportResponse = parseTransportResponse(finalMessage)
  if (transportResponse) {
    reply.code(transportResponse.status)
    reply.headers(transportResponse.headers)
    return transportResponse.body ?? ""
  }
  if (result.queued) {
    reply.code(202)
    return { accepted: true, queued: true, result: result.response }
  }
  if (workflowRun && !isTerminalWorkflowRunStatus(workflowRun.status)) {
    reply.code(202)
    return { accepted: true, workflow_run: workflowRun }
  }
  reply.code(200)
  return {
    accepted: result.accepted,
    workflow_run: workflowRun ?? null,
    final_output: workflowRun?.final_output ?? null,
  }
}

function parseTransportResponse(message: string | undefined | null): { status: number; headers: Record<string, string>; body: unknown } | null {
  if (!message) return null
  try {
    const parsed = JSON.parse(message) as {
      kind?: string
      status?: number
      headers?: Record<string, string>
      body?: unknown
    }
    if (parsed.kind !== "http_response") return null
    return {
      status: parsed.status ?? 200,
      headers: parsed.headers ?? {},
      body: parsed.body ?? "",
    }
  } catch {
    return null
  }
}

async function authenticateRequest(
  request: GatewayRequest,
  publication: WorkflowPublicationConfig,
  config: AuthConfig,
  deps: GatewayDeps,
): Promise<{ ok: true; caller: Record<string, unknown> } | { ok: false; message: string }> {
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

async function authenticateArrobaRequest(
  request: GatewayRequest,
  publication: WorkflowPublicationConfig,
  config: ArrobaAuthConfig,
  deps: GatewayDeps,
): Promise<{ ok: true; caller: Record<string, unknown> } | { ok: false; message: string }> {
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
): Promise<{ ok: true; caller: Record<string, unknown> } | { ok: false; message: string } | null> {
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

function handleConnectorHandshake(
  request: GatewayRequest,
  reply: { code: (code: number) => unknown; headers: (headers: Record<string, string>) => unknown },
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

function handleSlackHandshake(
  request: GatewayRequest,
  connector: Extract<ConnectorConfig, { kind: "slack" }>,
  reply: { code: (code: number) => unknown; headers: (headers: Record<string, string>) => unknown },
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
  reply: { code: (code: number) => unknown; headers: (headers: Record<string, string>) => unknown },
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
  reply: { code: (code: number) => unknown; headers: (headers: Record<string, string>) => unknown },
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

function verifyHeaderIdentity(request: GatewayRequest, connector: ConnectorKind): VerifiedExternalIdentity | null {
  const externalId = headerValue(request, `x-arroba-${connector}-identity`)
  if (!externalId) return null
  return { connector, external_id: externalId }
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

function safeEqualString(actual: string | undefined, expected: string) {
  if (!actual) return false
  const actualBuffer = Buffer.from(actual)
  const expectedBuffer = Buffer.from(expected)
  if (actualBuffer.length !== expectedBuffer.length) return false
  return timingSafeEqual(actualBuffer, expectedBuffer)
}

function objectBody(body: unknown): Record<string, unknown> {
  return body && typeof body === "object" && !Array.isArray(body) ? body as Record<string, unknown> : {}
}

function isParseErrorPayload(value: unknown): value is { __arroba_parse_error: string } {
  return !!value
    && typeof value === "object"
    && "__arroba_parse_error" in value
    && typeof (value as { __arroba_parse_error?: unknown }).__arroba_parse_error === "string"
}

function optionalString(value: unknown) {
  return typeof value === "string" && value.trim() ? value.trim() : null
}

function isPairedSenderAuthEnabled(auth: AuthConfig | undefined) {
  return auth?.mode === "arroba" && auth.paired_senders?.enabled === true && auth.paired_senders.pair_endpoint !== false
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

function rawRequestBody(request: GatewayRequest) {
  return (request.raw as { arrobaRawBody?: string } | undefined)?.arrobaRawBody
}

function installRawBodyParsers(app: ReturnType<typeof Fastify>) {
  app.removeContentTypeParser("application/json")
  app.addContentTypeParser("application/json", { parseAs: "string" }, (request: { raw: { arrobaRawBody?: string } }, body: string, done: (error: Error | null, body?: unknown) => void) => {
    setRawRequestBody(request, body)
    try {
      done(null, body ? JSON.parse(body) : {})
    } catch (error) {
      done(error as Error)
    }
  })

  app.addContentTypeParser("application/x-www-form-urlencoded", { parseAs: "string" }, (request: { raw: { arrobaRawBody?: string } }, body: string, done: (error: Error | null, body?: unknown) => void) => {
    setRawRequestBody(request, body)
    done(null, Object.fromEntries(new URLSearchParams(body)))
  })

  app.addContentTypeParser("text/plain", { parseAs: "string" }, (request: { raw: { arrobaRawBody?: string } }, body: string, done: (error: Error | null, body?: unknown) => void) => {
    setRawRequestBody(request, body)
    done(null, body)
  })
}

function setRawRequestBody(request: { raw: { arrobaRawBody?: string } }, body: string) {
  request.raw.arrobaRawBody = body
}

async function parseRequest(request: GatewayRequest, config: ParserConfig): Promise<unknown> {
  switch (config.kind) {
    case "json":
      return request.body ?? {}
    case "query_params":
      return request.query ?? {}
    case "headers":
      return request.headers
    case "webhook":
      return { headers: request.headers, body: request.body ?? {}, query: request.query ?? {} }
    case "regex":
      return parseRegex(sourceValue(request, config.source ?? "path"), config)
    case "path_template":
      return parsePathTemplate(String(request.url.split("?")[0] ?? "/"), config)
    case "custom_command":
      return await parseCustomCommand(request, config)
  }
}

async function parseAndValidateRequest(
  request: GatewayRequest,
  publication: WorkflowPublicationConfig,
): Promise<unknown> {
  const parsed = await parseRequest(request, publication.parser ?? { kind: "json" })
  validateInput(parsed, publication.input_schema)
  return parsed
}

function parseRegex(source: string, config: ParserConfig) {
  if (!config.pattern) throw new Error("regex parser requires pattern")
  const match = new RegExp(config.pattern).exec(source)
  if (!match) throw new Error("request did not match regex parser")
  return { ...(match.groups ?? {}) }
}

function parsePathTemplate(pathname: string, config: ParserConfig) {
  if (!config.template) throw new Error("path_template parser requires template")
  const templateParts = config.template.split("/").filter(Boolean)
  const pathParts = pathname.split("/").filter(Boolean)
  if (templateParts.length !== pathParts.length) {
    throw new Error("request path did not match template")
  }
  const output: Record<string, string> = {}
  for (let index = 0; index < templateParts.length; index += 1) {
    const template = templateParts[index] ?? ""
    const value = decodeURIComponent(pathParts[index] ?? "")
    if (template.startsWith(":")) output[template.slice(1)] = value
    else if (template !== value) throw new Error("request path did not match template")
  }
  return output
}

async function parseCustomCommand(request: GatewayRequest, config: ParserConfig) {
  if (!config.command) throw new Error("custom_command parser requires command")
  const envelope = JSON.stringify({
    method: request.method,
    url: request.url,
    headers: request.headers,
    query: request.query ?? {},
    body: request.body ?? {},
  })
  const output = await runCommand(config.command, config.args ?? [], envelope)
  return JSON.parse(output || "{}")
}

function validateInput(value: unknown, schema: InputSchema | undefined) {
  if (!schema) return
  if (schema.type === "object" && (!value || typeof value !== "object" || Array.isArray(value))) {
    throw new Error("input schema expected object")
  }
  const object = value as Record<string, unknown>
  for (const field of schema.required ?? []) {
    if (object[field] === undefined || object[field] === null) {
      throw new Error(`input schema missing required field ${field}`)
    }
  }
  for (const [field, spec] of Object.entries(schema.properties ?? {})) {
    if (object[field] === undefined || !spec.type) continue
    if (!matchesJsonType(object[field], spec.type)) {
      throw new Error(`input schema field ${field} expected ${spec.type}`)
    }
  }
}

function matchesJsonType(value: unknown, type: string) {
  if (type === "array") return Array.isArray(value)
  if (type === "object") return value != null && typeof value === "object" && !Array.isArray(value)
  return typeof value === type
}

function sourceValue(request: GatewayRequest, source: ParserConfig["source"]) {
  if (source === "body") return typeof request.body === "string" ? request.body : JSON.stringify(request.body ?? {})
  if (source === "query") return JSON.stringify(request.query ?? {})
  if (source === "headers") return JSON.stringify(request.headers)
  if (source === "request") return JSON.stringify({ method: request.method, url: request.url, body: request.body ?? {}, query: request.query ?? {} })
  return String(request.url.split("?")[0] ?? "/")
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

function defaultKernelEndpoint() {
  return process.env.ARROBA_KERNEL_URL ?? `ws://${process.env.ARROBA_KERNEL_HOST ?? "127.0.0.1"}:${process.env.ARROBA_KERNEL_PORT ?? "43118"}`
}

function defaultPublicationConfig(): WorkflowPublicationConfig {
  const config: WorkflowPublicationConfig = {
    publication_id: process.env.ARROBA_PUBLICATION_ID ?? "default",
    session_id: requiredProcessEnv("ARROBA_PUBLICATION_SESSION_ID"),
    workflow_ref: requiredProcessEnv("ARROBA_PUBLICATION_WORKFLOW"),
    endpoint_ref: requiredProcessEnv("ARROBA_PUBLICATION_ENDPOINT"),
    route: process.env.ARROBA_PUBLICATION_ROUTE ?? "/*",
    mode: process.env.ARROBA_PUBLICATION_MODE === "async" ? "async" : "sync",
  }
  if (process.env.ARROBA_KERNEL_URL) config.kernel_endpoint = process.env.ARROBA_KERNEL_URL
  const tls = tlsConfigFromEnv()
  if (tls) config.tls = tls
  return config
}

function tlsConfigFromEnv(): TlsConfig | undefined {
  const keyFile = process.env.ARROBA_PUBLICATION_TLS_KEY_FILE
  const certFile = process.env.ARROBA_PUBLICATION_TLS_CERT_FILE
  if (!keyFile && !certFile) return undefined
  const tls: TlsConfig = { enabled: process.env.ARROBA_PUBLICATION_TLS_ENABLED !== "false" }
  if (keyFile) tls.key_file = keyFile
  if (certFile) tls.cert_file = certFile
  return tls
}

function resolveHttpsOptions(tls: TlsConfig | undefined) {
  if (!tls || tls.enabled === false) return undefined
  if (!tls.key_file || !tls.cert_file) {
    throw new Error("HTTPS requires tls.key_file and tls.cert_file")
  }
  return {
    key: readFileSync(tls.key_file),
    cert: readFileSync(tls.cert_file),
  }
}

function requiredProcessEnv(name: string) {
  const value = process.env[name]
  if (!value) throw new Error(`required env ${name} is not set`)
  return value
}

async function runCommand(command: string, args: string[], input: string): Promise<string> {
  return await new Promise((resolve, reject) => {
    const child = spawn(command, args, { stdio: ["pipe", "pipe", "pipe"] })
    let stdout = ""
    let stderr = ""
    child.stdout.on("data", (chunk) => { stdout += chunk.toString() })
    child.stderr.on("data", (chunk) => { stderr += chunk.toString() })
    child.on("error", reject)
    child.on("close", (code) => {
      if (code === 0) resolve(stdout)
      else reject(new Error(`custom parser exited ${code}: ${stderr}`))
    })
    child.stdin.end(input)
  })
}

function isTerminalWorkflowRunStatus(status: string) {
  return ["completed", "failed", "stopped"].includes(status.toLowerCase())
}

async function sleep(ms: number) {
  await new Promise((resolve) => setTimeout(resolve, ms))
}

export async function loadPublicationConfig(path: string) {
  return JSON.parse(await readFile(path, "utf8")) as WorkflowPublicationConfig
}

export async function loadPublicationConfigFromKernel(
  sessionId: string,
  publicationRef: string,
  kernelEndpoint = defaultKernelEndpoint(),
  client?: KernelLookupClient,
): Promise<WorkflowPublicationConfig> {
  const ownedClient = client ?? new LocalIpcClient(kernelEndpoint)
  try {
    const response = await ownedClient.send(
      getWorkflowPublicationRequest(sessionId, publicationRef),
    )
    const publication = (response.WorkflowPublication as { publication?: WorkflowPublicationDefinition } | undefined)?.publication
    if (!publication) {
      throw new Error(`unexpected workflow publication response: ${JSON.stringify(response)}`)
    }
    return publicationConfigFromKernelRecord(publication, kernelEndpoint)
  } finally {
    if (!client) {
      await ownedClient.close?.().catch(() => {})
    }
  }
}

export function publicationConfigFromKernelRecord(
  publication: WorkflowPublicationDefinition,
  kernelEndpoint = defaultKernelEndpoint(),
): WorkflowPublicationConfig {
  const config: WorkflowPublicationConfig = {
    publication_id: publication.id,
    session_id: publication.session_id,
    workflow_ref: publication.workflow_id,
    endpoint_ref: publication.endpoint_id,
    kernel_endpoint: kernelEndpoint,
    route: publication.route ?? "/*",
    auth: asAuthConfig(publication.auth) ?? { mode: "anonymous" },
    parser: asParserConfig(publication.parser) ?? { kind: "json" },
    mode: publication.mode === "async" ? "async" : "sync",
  }
  const methods = normalizeHttpMethods(publication.methods)
  if (methods) config.methods = methods
  const inputSchema = asInputSchema(publication.input_schema)
  if (inputSchema) config.input_schema = inputSchema
  return config
}

export async function loadGatewayPublicationConfig(): Promise<WorkflowPublicationConfig | undefined> {
  if (process.env.ARROBA_PUBLICATION_CONFIG) {
    return withEnvTlsConfig(await loadPublicationConfig(process.env.ARROBA_PUBLICATION_CONFIG))
  }
  if (
    process.env.ARROBA_PUBLICATION_SESSION_ID
    && process.env.ARROBA_PUBLICATION_ID
    && (!process.env.ARROBA_PUBLICATION_WORKFLOW || !process.env.ARROBA_PUBLICATION_ENDPOINT)
  ) {
    return withEnvTlsConfig(await loadPublicationConfigFromKernel(
      process.env.ARROBA_PUBLICATION_SESSION_ID,
      process.env.ARROBA_PUBLICATION_ID,
      defaultKernelEndpoint(),
    ))
  }
  return undefined
}

function withEnvTlsConfig(config: WorkflowPublicationConfig) {
  const tls = tlsConfigFromEnv()
  if (tls) return { ...config, tls }
  return config
}

function normalizeHttpMethods(methods: string[] | undefined): Array<"GET" | "POST"> | undefined {
  const normalized = (methods ?? [])
    .map((method) => method.toUpperCase())
    .filter((method): method is "GET" | "POST" => method === "GET" || method === "POST")
  return normalized.length > 0 ? normalized : undefined
}

function asAuthConfig(value: unknown): AuthConfig | undefined {
  return isPlainObject(value) && typeof value.mode === "string" ? value as AuthConfig : undefined
}

function asParserConfig(value: unknown): ParserConfig | undefined {
  return isPlainObject(value) && typeof value.kind === "string" ? value as ParserConfig : undefined
}

function asInputSchema(value: unknown): InputSchema | undefined {
  return isPlainObject(value) ? value as InputSchema : undefined
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value)
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const config = await loadGatewayPublicationConfig()
  const { app, logger } = buildServer(config)
  const host = process.env.HOST ?? "0.0.0.0"
  const port = Number(process.env.PORT ?? 3000)
  logger.info("starting workflow gateway", { host, port })

  app
    .listen({ host, port })
    .then((address) => {
      logger.info("workflow gateway listening", { host, port, address })
    })
    .catch((error) => {
      logger.error("workflow gateway failed to start", { error: error.message, host, port })
      process.exit(1)
    })
}
