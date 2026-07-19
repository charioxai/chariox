import type { WorkflowPublicationDefinition } from "@arroba/kernel-client"

import type {
  DeploymentSetupConfiguration,
  DeploymentSetupInitialAccess,
  DeploymentSetupRuntimeMode,
} from "./deployed-workflow-setup-api.js"

export type PublicationTransport = "human_http" | "api_sse_json" | "websocket_json" | "mcp"
export type AgentAppManipulationLevel = "none" | "state" | "overlay" | "state_and_overlay" | "full_ephemeral"

export interface ParsedSetupOptions {
  readonly slug?: string
  readonly name?: string
  readonly mode?: DeploymentSetupRuntimeMode
  readonly transport?: PublicationTransport
  readonly region: "eu-central"
  readonly clientRequestId?: string
  readonly access: DeploymentSetupInitialAccess
  readonly agentApp: boolean
  readonly agentAppAssets?: string
  readonly appRoute: string
  readonly manipulationLevel: AgentAppManipulationLevel
  readonly replicas: number
}

export const deploymentSetupUsage = [
  "usage: deployments setup list",
  "       deployments setup show <setup-id>",
  "       deployments setup resume <setup-id> [--agent-app-assets path]",
  "       deployments setup draft <workflow-ref> <endpoint-ref> --slug value --transport human-http|api-sse-json|websocket-json|mcp --mode local-runtime|hosted-container [agent-app options]",
  "       deployments setup publication <publication-ref> --slug value --mode local-runtime|hosted-container [agent-app options]",
  "       access options: [--access current-account|email|verified-domain|public] [--access-subject value]",
].join("\n")

export function parseSetupOptions(
  argv: readonly string[],
  constraints: { readonly requireDeployment: boolean; readonly allowTransport: boolean },
): ParsedSetupOptions {
  let slug: string | undefined
  let name: string | undefined
  let mode: DeploymentSetupRuntimeMode | undefined
  let transport: PublicationTransport | undefined
  let clientRequestId: string | undefined
  let accessKind: DeploymentSetupInitialAccess["kind"] = "current_account"
  let accessSubject: string | undefined
  let agentApp = false
  let agentAppAssets: string | undefined
  let appRoute = "/app/*"
  let manipulationLevel: AgentAppManipulationLevel = "state_and_overlay"
  let replicas = 1
  for (let index = 0; index < argv.length; index += 1) {
    const option = argv[index]
    if (option === "--agent-app") {
      agentApp = true
      continue
    }
    const value = requiredArg(argv[index + 1])
    index += 1
    switch (option) {
      case "--slug": slug = value; break
      case "--name": name = value; break
      case "--mode": mode = parseRuntimeMode(value); break
      case "--transport": transport = parseTransport(value); break
      case "--region":
        if (value !== "eu-central") throw new Error("deployment region must be eu-central")
        break
      case "--client-request-id": clientRequestId = value; break
      case "--access": accessKind = parseAccessKind(value); break
      case "--access-subject": accessSubject = value; break
      case "--agent-app-assets": agentApp = true; agentAppAssets = value; break
      case "--app-route": agentApp = true; appRoute = value; break
      case "--manipulation-level": agentApp = true; manipulationLevel = parseManipulationLevel(value); break
      case "--replicas": agentApp = true; replicas = parseReplicas(value); break
      default: throw new Error(`unknown deployments setup option ${option}`)
    }
  }
  if (!constraints.allowTransport && transport) {
    throw new Error("published deployments use their immutable publication transport")
  }
  if (constraints.requireDeployment) {
    if (!slug?.trim()) throw new Error("deployment setup requires --slug")
    if (!mode) throw new Error("deployment setup requires --mode")
    if (constraints.allowTransport && !transport) throw new Error("draft deployment setup requires --transport")
  }
  return {
    ...(slug ? { slug: slug.trim() } : {}),
    ...(name ? { name: name.trim() } : {}),
    ...(mode ? { mode } : {}),
    ...(transport ? { transport } : {}),
    region: "eu-central",
    ...(clientRequestId ? { clientRequestId } : {}),
    access: deploymentInitialAccess(accessKind, accessSubject),
    agentApp,
    ...(agentAppAssets ? { agentAppAssets } : {}),
    appRoute,
    manipulationLevel,
    replicas,
  }
}

export function draftConfiguration(
  endpointId: string,
  revision: number,
  options: ParsedSetupOptions,
): DeploymentSetupConfiguration {
  const slug = requiredText(options.slug, "deployment slug")
  const mode = requiredRuntimeMode(options.mode)
  const transport = requiredTransport(options.transport)
  return {
    endpointId,
    access: options.access,
    publication: publicationConfiguration(transport, slug, revision),
    deployment: deploymentConfiguration(slug, mode, options),
    agentApp: agentAppConfiguration(options),
  }
}

export function publishedConfiguration(
  publication: WorkflowPublicationDefinition,
  options: ParsedSetupOptions,
): DeploymentSetupConfiguration {
  const slug = requiredText(options.slug, "deployment slug")
  const mode = requiredRuntimeMode(options.mode)
  publicationTransportKind(publication.transport)
  return {
    endpointId: publication.endpoint_id,
    access: options.access,
    publication: {
      alias: publication.alias?.trim() || publication.id,
      kind: publication.kind?.trim() || "ingress",
      queueRef: publication.queue_ref ?? null,
      route: publication.route ?? null,
      methods: publication.methods ?? [],
      transport: publication.transport ?? null,
      parser: publication.parser ?? null,
      inputSchema: publication.input_schema ?? null,
      traceExposure: publication.trace_exposure ?? null,
      mode: publication.mode ?? null,
      syncTimeoutMs: publication.sync_timeout_ms ?? null,
      pollMs: publication.poll_ms ?? null,
    },
    deployment: deploymentConfiguration(slug, mode, options),
    agentApp: agentAppConfiguration(options),
  }
}

export function publicationTransportKind(value: unknown): PublicationTransport {
  const kind = objectRecord(value)?.kind
  if (typeof kind !== "string") throw new Error("publication transport is unavailable")
  return parseTransport(kind)
}

function deploymentConfiguration(
  slug: string,
  mode: DeploymentSetupRuntimeMode,
  options: ParsedSetupOptions,
): DeploymentSetupConfiguration["deployment"] {
  return {
    name: options.name?.trim() || slug,
    slug,
    kind: options.agentApp ? "agent_app" : "workflow_endpoint",
    runtimeMode: mode,
    region: mode === "hosted_container" ? options.region : null,
  }
}

function agentAppConfiguration(options: ParsedSetupOptions): {
  readonly enabled: boolean
  readonly routePath: string
  readonly manipulationLevel: AgentAppManipulationLevel
  readonly replicaCount: number
} | null {
  return options.agentApp
    ? {
      enabled: true,
      routePath: options.appRoute,
      manipulationLevel: options.manipulationLevel,
      replicaCount: options.replicas,
    }
    : null
}

function publicationConfiguration(
  transport: PublicationTransport,
  slug: string,
  revision: number,
): DeploymentSetupConfiguration["publication"] {
  return {
    alias: versionedPublicationAlias(slug, revision),
    kind: "ingress",
    route: publicationRoute(transport),
    methods: publicationMethods(transport),
    transport: { kind: transport },
    parser: publicationParser(transport),
    traceExposure: null,
    mode: transport === "mcp" ? "sync" : "async",
  }
}

function versionedPublicationAlias(slug: string, revision: number): string {
  const suffix = `-r${revision}`
  return `${slug.slice(0, Math.max(1, 72 - suffix.length)).replace(/-+$/, "")}${suffix}`
}

function publicationRoute(transport: PublicationTransport): string {
  switch (transport) {
    case "human_http": return "/prompt/*"
    case "api_sse_json": return "/invoke"
    case "websocket_json": return "/socket"
    case "mcp": return "/mcp"
  }
}

function publicationMethods(transport: PublicationTransport): readonly string[] {
  switch (transport) {
    case "human_http": return ["GET"]
    case "api_sse_json":
    case "mcp": return ["POST"]
    case "websocket_json": return []
  }
}

function publicationParser(transport: PublicationTransport): unknown | null {
  if (transport === "api_sse_json") return { kind: "json" }
  if (transport === "human_http") return { kind: "path_template", template: "/prompt/:prompt" }
  return null
}

function parseRuntimeMode(value: string): DeploymentSetupRuntimeMode {
  if (value === "local-runtime" || value === "local_runtime") return "local_runtime"
  if (value === "hosted-container" || value === "hosted_container") return "hosted_container"
  throw new Error("deployment mode must be local-runtime or hosted-container")
}

function parseTransport(value: string): PublicationTransport {
  const normalized = value.replaceAll("-", "_")
  if (normalized === "human_http" || normalized === "api_sse_json" || normalized === "websocket_json" || normalized === "mcp") {
    return normalized
  }
  throw new Error("deployment transport must be human-http, api-sse-json, websocket-json, or mcp")
}

function parseManipulationLevel(value: string): AgentAppManipulationLevel {
  if (value === "none" || value === "state" || value === "overlay" || value === "state_and_overlay" || value === "full_ephemeral") {
    return value
  }
  throw new Error("Agent App manipulation level must be none, state, overlay, state_and_overlay, or full_ephemeral")
}

function parseAccessKind(value: string): DeploymentSetupInitialAccess["kind"] {
  const normalized = value.trim().toLowerCase().replaceAll("-", "_")
  if (normalized === "current_account" || normalized === "private") return "current_account"
  if (normalized === "email") return "email"
  if (normalized === "verified_domain" || normalized === "email_domain" || normalized === "domain") {
    return "email_domain"
  }
  if (normalized === "public") return "public"
  throw new Error("deployment access must be current-account, email, verified-domain, or public")
}

function deploymentInitialAccess(
  kind: DeploymentSetupInitialAccess["kind"],
  subject: string | undefined,
): DeploymentSetupInitialAccess {
  if (kind === "current_account" || kind === "public") {
    if (subject?.trim()) throw new Error(`deployment access ${kind.replaceAll("_", "-")} does not use --access-subject`)
    return { kind }
  }
  const normalized = requiredText(subject, "deployment access subject").toLowerCase()
  if (kind === "email") {
    if (!/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(normalized)) {
      throw new Error("deployment access email is invalid")
    }
    return { kind, subject: normalized }
  }
  const domain = normalized.replace(/^@/, "")
  if (!/^(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63}$/.test(domain)) {
    throw new Error("deployment access domain is invalid")
  }
  return { kind, subject: domain }
}

function parseReplicas(value: string): number {
  const replicas = Number(value)
  if (!Number.isSafeInteger(replicas) || replicas < 1 || replicas > 32) {
    throw new Error("Agent App replicas must be an integer between 1 and 32")
  }
  return replicas
}

function requiredRuntimeMode(value: DeploymentSetupRuntimeMode | undefined): DeploymentSetupRuntimeMode {
  if (!value) throw new Error("deployment runtime mode is unavailable")
  return value
}

function requiredTransport(value: PublicationTransport | undefined): PublicationTransport {
  if (!value) throw new Error("deployment transport is unavailable")
  return value
}

function requiredText(value: unknown, label: string): string {
  if (typeof value !== "string" || !value.trim()) throw new Error(`${label} is unavailable`)
  return value.trim()
}

function requiredArg(value: string | undefined): string {
  if (!value?.trim()) throw new Error(deploymentSetupUsage)
  return value.trim()
}

function objectRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null
}
