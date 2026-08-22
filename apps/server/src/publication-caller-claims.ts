import { createHmac, createPublicKey, timingSafeEqual, verify } from "node:crypto"
import {
  closeSync,
  constants as fsConstants,
  fstatSync,
  lstatSync,
  openSync,
  readFileSync,
  unlinkSync,
} from "node:fs"
import { join } from "node:path"

import {
  resolveWorkflowPublicationDeploymentContract,
  workflowPublicationDeploymentContractPath,
} from "@chariox/kernel-client/workflow-publication-deployment-contract"

import type { WorkflowPublicationConfig } from "./publication-types.js"

const PUBLICATION_CLAIMS_ISSUER = "chariox-cloud"
const MAX_CALLER_CLAIMS_BYTES = 16 * 1024
const MAX_CALLER_CLAIMS_CONFIG_BYTES = 16 * 1024
const MIN_CALLER_CLAIMS_SECRET_BYTES = 32
const MAX_CALLER_CLAIMS_SECRET_BYTES = 8 * 1024
const MAX_CALLER_CLAIMS_TTL_SECONDS = 60
const MAX_CALLER_CLAIMS_CLOCK_SKEW_SECONDS = 30
const MAX_REPLAY_IDENTIFIERS = 200_000

type RequestHeaders = Record<string, string | string[] | undefined>

export type PublicationCallerClaimsRuntimeConfig = {
  readonly deploymentId: string
  readonly environmentId: string
  readonly secret?: string | Uint8Array
  readonly publicKeyPem?: string
  readonly now?: () => Date
}

type NormalizedPublicationCallerClaimsRuntimeConfig = {
  readonly deploymentId: string
  readonly environmentId: string
  readonly verifier:
    | { readonly kind: "hmac"; readonly secret: Buffer }
    | { readonly kind: "ed25519"; readonly publicKey: ReturnType<typeof createPublicKey> }
  readonly now: () => Date
}

export type VerifiedPublicationCallerClaims = {
  readonly accountId: string | null
  readonly deploymentId: string
  readonly environmentId: string
  readonly expiresAt: Date
  readonly invocationId: string
  readonly issuedAt: Date
  readonly nonce: string
  readonly roles: readonly string[]
  readonly subject: string
}

export class PublicationCallerClaimsError extends Error {
  constructor(
    message: string,
    readonly statusCode: 401 | 403 = 401,
    readonly code = "caller_claims_invalid",
  ) {
    super(message)
    this.name = "PublicationCallerClaimsError"
  }
}

const replayExpirations = new Map<string, number>()
const environmentRuntimeConfig = consumePublicationCallerClaimsRuntimeConfigFromEnv()
let testRuntimeConfig: NormalizedPublicationCallerClaimsRuntimeConfig | null | undefined
let requestCallers = new WeakMap<object, VerifiedPublicationCallerClaims>()
let publicationRequiredRoles = new WeakMap<object, readonly string[]>()

export function publicationCallerClaimsRuntimeConfigured(): boolean {
  return currentRuntimeConfig() !== null
}

export function hasPublicationCallerClaimsHeader(headers: RequestHeaders): boolean {
  return Object.keys(headers).some((name) => name.toLowerCase() === "x-chariox-caller-claims")
}

export function authorizePublicationCaller(
  headers: RequestHeaders,
  publication: WorkflowPublicationConfig,
): VerifiedPublicationCallerClaims | null {
  if (!publicationCallerClaimsRuntimeConfigured() && !hasPublicationCallerClaimsHeader(headers)) return null
  const caller = verifyPublicationCallerClaims(headers)
  requirePublicationCallerRole(publication, caller)
  return caller
}

export function authorizePublicationCallerRequest(
  request: object & { readonly headers: RequestHeaders },
  publication: WorkflowPublicationConfig,
): VerifiedPublicationCallerClaims | null {
  const caller = authorizePublicationCaller(request.headers, publication)
  if (caller) requestCallers.set(request, caller)
  return caller
}

export function publicationCallerForRequest(request: object): VerifiedPublicationCallerClaims | null {
  const caller = requestCallers.get(request) ?? null
  if (!caller && publicationCallerClaimsRuntimeConfigured()) {
    throw new PublicationCallerClaimsError("Publication request has no verified caller context")
  }
  return caller
}

export function publicationInvocationCaller(
  caller: VerifiedPublicationCallerClaims | null,
  proof: Record<string, unknown>,
): Record<string, unknown> {
  if (!caller) return { type: "anonymous", proof }
  return {
    type: "authenticated",
    proof: {
      ...proof,
      publication_caller: {
        account_id: caller.accountId,
        deployment_id: caller.deploymentId,
        environment_id: caller.environmentId,
        invocation_id: caller.invocationId,
        roles: caller.roles,
        subject: caller.subject,
      },
    },
  }
}

export function publicationCallerAuthorizationFailure(error: PublicationCallerClaimsError): {
  readonly statusCode: 401 | 403
  readonly body: {
    readonly error: {
      readonly code: string
      readonly message: string
    }
  }
} {
  return {
    statusCode: error.statusCode,
    body: {
      error: {
        code: error.code,
        message: error.statusCode === 403
          ? "Publication caller does not have the required role"
          : "Publication caller authentication is required",
      },
    },
  }
}

export function verifyPublicationCallerClaims(
  headers: RequestHeaders,
): VerifiedPublicationCallerClaims {
  const config = currentRuntimeConfig()
  if (!config) {
    throw new PublicationCallerClaimsError("Publication caller claims runtime is not configured")
  }
  const token = requiredHeader(headers, "x-chariox-caller-claims")
  if (Buffer.byteLength(token, "utf8") > MAX_CALLER_CLAIMS_BYTES) {
    throw new PublicationCallerClaimsError("Publication caller claims are malformed")
  }
  const segments = token.split(".")
  if (segments.length !== 3 || segments.some((segment) => segment.length === 0)) {
    throw new PublicationCallerClaimsError("Publication caller claims are malformed")
  }
  const [encodedHeader, encodedPayload, suppliedSignature] = segments as [string, string, string]
  const unsigned = `${encodedHeader}.${encodedPayload}`
  const supplied = decodeBase64Url(suppliedSignature, "signature")
  const header = jsonObject(encodedHeader)
  const validSignature = config.verifier.kind === "ed25519"
    ? header.alg === "EdDSA" && verify(null, Buffer.from(unsigned), config.verifier.publicKey, supplied)
    : header.alg === "HS256" && validHmacSignature(unsigned, supplied, config.verifier.secret)
  if (!validSignature) {
    throw new PublicationCallerClaimsError("Publication caller claims signature is invalid")
  }

  const payload = jsonObject(encodedPayload)
  if (header.typ !== "JWT" || payload.iss !== PUBLICATION_CLAIMS_ISSUER) {
    throw new PublicationCallerClaimsError("Publication caller claims header is invalid")
  }

  const deploymentId = requiredString(payload.deployment_id, "deployment_id", 320)
  const environmentId = requiredString(payload.environment_id, "environment_id", 320)
  const audience = requiredString(payload.aud, "aud", 320)
  if (audience !== deploymentId || deploymentId !== config.deploymentId) {
    throw new PublicationCallerClaimsError("Publication caller claims deployment is invalid")
  }
  if (environmentId !== config.environmentId) {
    throw new PublicationCallerClaimsError("Publication caller claims environment is invalid")
  }

  const issuedAtSeconds = requiredInteger(payload.iat, "iat")
  const expiresAtSeconds = requiredInteger(payload.exp, "exp")
  const nowSeconds = Math.floor(config.now().getTime() / 1_000)
  if (
    expiresAtSeconds <= nowSeconds
    || expiresAtSeconds <= issuedAtSeconds
    || expiresAtSeconds - issuedAtSeconds > MAX_CALLER_CLAIMS_TTL_SECONDS
  ) {
    throw new PublicationCallerClaimsError("Publication caller claims are expired or not short-lived")
  }
  if (issuedAtSeconds > nowSeconds + MAX_CALLER_CLAIMS_CLOCK_SKEW_SECONDS) {
    throw new PublicationCallerClaimsError("Publication caller claims were issued in the future")
  }

  const invocationId = requiredString(payload.invocation_id, "invocation_id", 320)
  if (requiredHeader(headers, "x-chariox-invocation-id") !== invocationId) {
    throw new PublicationCallerClaimsError("Publication caller claims invocation is invalid")
  }
  const nonce = requiredString(payload.nonce, "nonce", 320)
  const roles = normalizedRoles(payload.roles, "Publication caller claims roles")
  const subject = requiredString(payload.sub, "sub", 1_024)
  const accountId = payload.org === null ? null : requiredString(payload.org, "org", 320)
  consumeReplayIdentifiers(config, invocationId, nonce, expiresAtSeconds, nowSeconds)

  return {
    accountId,
    deploymentId,
    environmentId,
    expiresAt: new Date(expiresAtSeconds * 1_000),
    invocationId,
    issuedAt: new Date(issuedAtSeconds * 1_000),
    nonce,
    roles,
    subject,
  }
}

export function configurePublicationCallerClaimsRuntimeForTests(
  config: PublicationCallerClaimsRuntimeConfig | null | undefined,
): void {
  testRuntimeConfig = config === undefined
    ? undefined
    : config === null
      ? null
      : normalizeRuntimeConfig(config)
  clearPublicationCallerClaimsStateForTests()
}

export function clearPublicationCallerClaimsStateForTests(): void {
  replayExpirations.clear()
  requestCallers = new WeakMap()
  publicationRequiredRoles = new WeakMap()
}

export function readPrivatePublicationCallerClaimsConfigFile(
  path: string,
): PublicationCallerClaimsRuntimeConfig {
  const raw = readPrivateCallerConfigFile(path)
  let parsed: unknown
  try {
    parsed = JSON.parse(raw) as unknown
  } catch {
    throw new Error("publication caller claims config file must contain valid JSON")
  }
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("publication caller claims config file must contain an object")
  }
  const config = parsed as Record<string, unknown>
  const expectedKeys = new Set([
    "schema_version",
    "deployment_id",
    "environment_id",
    "secret",
    "public_key_pem",
  ])
  if (Object.keys(config).some((key) => !expectedKeys.has(key)) || config.schema_version !== 1) {
    throw new Error("publication caller claims config file schema is invalid")
  }
  if (
    typeof config.deployment_id !== "string"
    || typeof config.environment_id !== "string"
    || (typeof config.secret === "string") === (typeof config.public_key_pem === "string")
  ) {
    throw new Error("publication caller claims config file fields are invalid")
  }
  return {
    deploymentId: config.deployment_id,
    environmentId: config.environment_id,
    ...(typeof config.secret === "string"
      ? { secret: config.secret }
      : { publicKeyPem: config.public_key_pem as string }),
  }
}

function consumePublicationCallerClaimsRuntimeConfigFromEnv(): NormalizedPublicationCallerClaimsRuntimeConfig | null {
  const configFile = process.env.CHARIOX_PUBLICATION_CALLER_CLAIMS_CONFIG_FILE
  delete process.env.CHARIOX_PUBLICATION_CALLER_CLAIMS_CONFIG_FILE
  const normalizedPath = configFile?.trim()
  if (configFile !== undefined && !normalizedPath) {
    throw new Error("publication caller claims config file path must not be empty")
  }
  if (!normalizedPath) {
    if (process.env.CHARIOX_PUBLICATION_CLOUD_DEPLOYMENT_ID?.trim()) {
      throw new Error("Cloud publication runtimes require a caller claims config file")
    }
    return null
  }
  return normalizeRuntimeConfig(readPrivatePublicationCallerClaimsConfigFile(normalizedPath))
}

function currentRuntimeConfig(): NormalizedPublicationCallerClaimsRuntimeConfig | null {
  return testRuntimeConfig === undefined ? environmentRuntimeConfig : testRuntimeConfig
}

function normalizeRuntimeConfig(
  config: PublicationCallerClaimsRuntimeConfig,
): NormalizedPublicationCallerClaimsRuntimeConfig {
  const deploymentId = config.deploymentId.trim()
  const environmentId = config.environmentId.trim()
  if (!deploymentId || !environmentId || deploymentId.length > 320 || environmentId.length > 320) {
    throw new Error("publication caller claims runtime identities are invalid")
  }
  if ((config.secret !== undefined) === (config.publicKeyPem !== undefined)) {
    throw new Error("publication caller claims runtime requires exactly one verifier")
  }
  const verifier = config.publicKeyPem !== undefined
    ? { kind: "ed25519" as const, publicKey: canonicalEd25519PublicKey(config.publicKeyPem) }
    : { kind: "hmac" as const, secret: normalizedCallerClaimsSecret(config.secret!) }
  return {
    deploymentId,
    environmentId,
    verifier,
    now: config.now ?? (() => new Date()),
  }
}

function normalizedCallerClaimsSecret(secret: string | Uint8Array): Buffer {
  const value = typeof secret === "string" ? Buffer.from(secret, "utf8") : Buffer.from(secret)
  if (value.length < MIN_CALLER_CLAIMS_SECRET_BYTES || value.length > MAX_CALLER_CLAIMS_SECRET_BYTES) {
    throw new Error(
      `publication caller claims secret must be between ${MIN_CALLER_CLAIMS_SECRET_BYTES} and ${MAX_CALLER_CLAIMS_SECRET_BYTES} bytes`,
    )
  }
  return value
}

function canonicalEd25519PublicKey(publicKeyPem: string) {
  try {
    const key = createPublicKey(publicKeyPem)
    if (key.asymmetricKeyType !== "ed25519") throw new Error("not Ed25519")
    if (key.export({ type: "spki", format: "pem" }).toString() !== publicKeyPem) {
      throw new Error("non-canonical key")
    }
    return key
  } catch {
    throw new Error("publication caller claims public key must be canonical Ed25519 SPKI PEM")
  }
}

function validHmacSignature(value: string, supplied: Buffer, secret: Buffer): boolean {
  const expected = Buffer.from(signature(value, secret), "base64url")
  return supplied.length === expected.length && timingSafeEqual(supplied, expected)
}

function requirePublicationCallerRole(
  publication: WorkflowPublicationConfig,
  caller: VerifiedPublicationCallerClaims,
): void {
  const requiredRoles = requiredRolesForPublication(publication)
  if (requiredRoles.length === 0 || requiredRoles.some((role) => caller.roles.includes(role))) return
  throw new PublicationCallerClaimsError(
    "Publication caller does not have a role required by this route",
    403,
    "caller_role_denied",
  )
}

function requiredRolesForPublication(publication: WorkflowPublicationConfig): readonly string[] {
  const cached = publicationRequiredRoles.get(publication)
  if (cached) return cached
  let roles: readonly string[]
  try {
    roles = deploymentContractRoles(publication)
      ?? configuredPublicationRoles(publication)
      ?? ["public"]
  } catch (error) {
    throw roleConfigurationError(error)
  }
  publicationRequiredRoles.set(publication, roles)
  return roles
}

function configuredPublicationRoles(publication: WorkflowPublicationConfig): readonly string[] | null {
  const routes = publication.agent_app?.routes ?? []
  const selected = routes.find((route) => (
    Boolean(publication.hook_id) && route.hook_id === publication.hook_id
  )) ?? routes.find((route) => (
    Boolean(publication.route) && route.path === publication.route
  )) ?? routes[0]
  if (selected) {
    return normalizedRoles([selected.required_role ?? "public"], "agent app route required_role")
  }
  return null
}

function deploymentContractRoles(publication: WorkflowPublicationConfig): readonly string[] | null {
  if (!publication.package_root) return null
  let publicationPackage: Record<string, unknown>
  try {
    publicationPackage = JSON.parse(
      readFileSync(join(publication.package_root, "publication.json"), "utf8"),
    ) as Record<string, unknown>
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return null
    throw roleConfigurationError(error)
  }
  try {
    const contractPath = workflowPublicationDeploymentContractPath(publicationPackage)
    if (!contractPath) return null
    const contractValue = JSON.parse(readFileSync(join(publication.package_root, contractPath), "utf8")) as unknown
    const resolution = resolveWorkflowPublicationDeploymentContract(publicationPackage, contractValue)
    if (resolution.kind !== "native") return null
    const routes = resolution.contract.routes
      .filter((route): route is Record<string, unknown> => Boolean(route) && typeof route === "object" && !Array.isArray(route))
    const selected = publication.hook_id
      ? routes.find((route) => route.id === publication.hook_id)
      : routes.find((route) => Boolean(publication.route) && route.path === publication.route) ?? routes[0]
    if (publication.hook_id && !selected) {
      throw new Error(`deployment contract has no route for hook ${publication.hook_id}`)
    }
    if (!selected) return []
    return normalizedRoles(selected.required_roles, "deployment contract route required_roles")
  } catch (error) {
    throw roleConfigurationError(error)
  }
}

function roleConfigurationError(error: unknown): PublicationCallerClaimsError {
  if (
    error instanceof PublicationCallerClaimsError
    && error.code === "caller_role_configuration_invalid"
  ) return error
  return new PublicationCallerClaimsError(
    `Publication route role configuration is invalid: ${error instanceof Error ? error.message : String(error)}`,
    403,
    "caller_role_configuration_invalid",
  )
}

function consumeReplayIdentifiers(
  config: NormalizedPublicationCallerClaimsRuntimeConfig,
  invocationId: string,
  nonce: string,
  expiresAtSeconds: number,
  nowSeconds: number,
): void {
  for (const [key, expiration] of replayExpirations) {
    if (expiration <= nowSeconds) replayExpirations.delete(key)
  }
  const scope = `${config.deploymentId}\u0000${config.environmentId}\u0000`
  const identifiers = [`${scope}invocation:${invocationId}`, `${scope}nonce:${nonce}`]
  if (identifiers.some((identifier) => replayExpirations.has(identifier))) {
    throw new PublicationCallerClaimsError("Publication caller claims were replayed")
  }
  if (replayExpirations.size + identifiers.length > MAX_REPLAY_IDENTIFIERS) {
    throw new PublicationCallerClaimsError("Publication caller claims replay protection is at capacity")
  }
  for (const identifier of identifiers) replayExpirations.set(identifier, expiresAtSeconds)
}

function readPrivateCallerConfigFile(path: string): string {
  let descriptor: number
  try {
    descriptor = openSync(path, fsConstants.O_RDONLY | fsConstants.O_NOFOLLOW)
  } catch (error) {
    throw new Error(`publication caller claims config file could not be opened safely: ${String(error)}`)
  }
  try {
    const metadata = fstatSync(descriptor)
    const currentUid = process.getuid?.()
    if (
      !metadata.isFile()
      || (metadata.mode & 0o077) !== 0
      || (currentUid !== undefined && metadata.uid !== currentUid)
      || metadata.nlink !== 1
      || metadata.size > MAX_CALLER_CLAIMS_CONFIG_BYTES
    ) {
      throw new Error(
        "publication caller claims config file must be a bounded, single-link owned regular file with mode 0600",
      )
    }
    const pathMetadata = lstatSync(path)
    if (
      !pathMetadata.isFile()
      || pathMetadata.isSymbolicLink()
      || pathMetadata.dev !== metadata.dev
      || pathMetadata.ino !== metadata.ino
    ) {
      throw new Error("publication caller claims config file changed while it was being consumed")
    }
    unlinkSync(path)
    if (fstatSync(descriptor).nlink !== 0) {
      throw new Error("publication caller claims config file was not consumed from its validated descriptor")
    }
    return readFileSync(descriptor, "utf8")
  } finally {
    closeSync(descriptor)
  }
}

function signature(value: string, secret: Buffer): string {
  return createHmac("sha256", secret).update(value).digest("base64url")
}

function jsonObject(value: string): Record<string, unknown> {
  try {
    const parsed = JSON.parse(decodeBase64Url(value, "payload").toString("utf8")) as unknown
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) throw new Error("not an object")
    return parsed as Record<string, unknown>
  } catch (error) {
    if (error instanceof PublicationCallerClaimsError) throw error
    throw new PublicationCallerClaimsError("Publication caller claims payload is malformed")
  }
}

function decodeBase64Url(value: string, label: string): Buffer {
  if (!/^[A-Za-z0-9_-]+$/.test(value)) {
    throw new PublicationCallerClaimsError(`Publication caller claims ${label} is malformed`)
  }
  const decoded = Buffer.from(value, "base64url")
  if (decoded.toString("base64url") !== value) {
    throw new PublicationCallerClaimsError(`Publication caller claims ${label} is malformed`)
  }
  return decoded
}

function requiredString(value: unknown, name: string, maxLength: number): string {
  if (typeof value !== "string" || !value || value.length > maxLength) {
    throw new PublicationCallerClaimsError(`Publication caller claims ${name} is invalid`)
  }
  return value
}

function requiredInteger(value: unknown, name: string): number {
  if (!Number.isSafeInteger(value) || (value as number) < 0) {
    throw new PublicationCallerClaimsError(`Publication caller claims ${name} is invalid`)
  }
  return value as number
}

function normalizedRoles(value: unknown, label: string): readonly string[] {
  if (!Array.isArray(value) || value.length > 64) {
    throw new PublicationCallerClaimsError(`${label} is invalid`)
  }
  const roles = value.map((role) => {
    if (typeof role !== "string" || !/^[a-z0-9][a-z0-9:_-]{0,63}$/.test(role)) {
      throw new PublicationCallerClaimsError(`${label} is invalid`)
    }
    return role
  })
  if (new Set(roles).size !== roles.length) {
    throw new PublicationCallerClaimsError(`${label} is invalid`)
  }
  return roles
}

function requiredHeader(headers: RequestHeaders, name: string): string {
  const value = headerValue(headers, name)
  if (!value) {
    throw new PublicationCallerClaimsError(
      `Publication caller claims require ${name}`,
      401,
      "caller_authentication_required",
    )
  }
  return value
}

function headerValue(headers: RequestHeaders, name: string): string | null {
  const entry = Object.entries(headers).find(([candidate]) => candidate.toLowerCase() === name)
  const raw = entry?.[1]
  if (Array.isArray(raw)) {
    if (raw.length !== 1) return null
    return raw[0]?.trim() || null
  }
  return typeof raw === "string" && raw.trim() ? raw.trim() : null
}
