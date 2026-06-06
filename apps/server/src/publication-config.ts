import { readFileSync } from "node:fs"
import { readFile } from "node:fs/promises"
import { dirname, join, resolve } from "node:path"
import process from "node:process"

import { LocalIpcClient } from "@arroba/kernel-client/ipc"
import {
  getSessionStateRequest,
  getWorkflowPublicationRequest,
  materializeWorkflowPublicationRequest,
} from "@arroba/kernel-client/ipc-requests"
import type {
  RuntimeSession,
  WorkflowPublicationDefinition,
} from "@arroba/kernel-client/kernel-types"

import { defaultKernelEndpoint } from "./kernel-publication-client.js"
import {
  resolvePublicationProviderModelBindings,
  type ProviderModelBindingPrompt,
} from "./publication-bindings.js"
import { validatePublicationRequirements } from "./publication-requirements.js"
import { ensurePublicationRuntimeAttached } from "./publication-runtime-pump.js"
import type {
  InputSchema,
  KernelLookupClient,
  ParserConfig,
  PublicationHookConfig,
  PublicationTraceContext,
  PublicationTraceExposurePolicy,
  TlsConfig,
  WorkflowPublicationPackage,
  WorkflowPublicationRequirements,
  WorkflowPublicationSnapshot,
  WorkflowPublicationConfig,
} from "./publication-types.js"

export function defaultPublicationConfig(): WorkflowPublicationConfig {
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

export function resolveHttpsOptions(tls: TlsConfig | undefined) {
  if (!tls || tls.enabled === false) return undefined
  if (!tls.key_file || !tls.cert_file) {
    throw new Error("HTTPS requires tls.key_file and tls.cert_file")
  }
  return {
    key: readFileSync(tls.key_file),
    cert: readFileSync(tls.cert_file),
  }
}

export async function loadPublicationConfig(path: string) {
  return JSON.parse(await readFile(path, "utf8")) as WorkflowPublicationConfig
}

export async function loadPublicationPackageConfig(
  path: string,
  options: {
    kernelEndpoint?: string
    hookId?: string
    materialize?: boolean
    validateRequirements?: boolean
    validateProviderBindings?: boolean
    promptProviderModelReplacement?: ProviderModelBindingPrompt | false
    client?: KernelLookupClient
  } = {},
): Promise<WorkflowPublicationConfig> {
  const packagePath = path.endsWith(".json") ? path : join(path, "publication.json")
  const root = dirname(resolve(packagePath))
  const publicationPackage = JSON.parse(await readFile(packagePath, "utf8")) as WorkflowPublicationPackage
  const snapshotPath = join(root, "workflow.snapshot.json")
  const snapshot = JSON.parse(await readFile(snapshotPath, "utf8")) as WorkflowPublicationSnapshot
  const config = publicationConfigFromPackage(
    publicationPackage,
    snapshot,
    options.kernelEndpoint ?? defaultKernelEndpoint(),
    options.hookId,
  )
  if (!options.materialize) return config
  const requirements = await loadPublicationRequirements(root)
  const ownedClient = options.client ?? new LocalIpcClient(config.kernel_endpoint ?? defaultKernelEndpoint())
  try {
    let materializationSnapshot = clonePublicationSnapshot(snapshot)
    if (options.validateProviderBindings !== false) {
      const bindingsPath = join(root, publicationPackage.default_bindings_path ?? "bindings.local.json")
      const bindingOptions: { promptReplacement?: ProviderModelBindingPrompt | false } = {}
      if (options.promptProviderModelReplacement !== undefined) {
        bindingOptions.promptReplacement = options.promptProviderModelReplacement
      }
      const resolved = await resolvePublicationProviderModelBindings(materializationSnapshot, bindingsPath, ownedClient, bindingOptions)
      materializationSnapshot = resolved.snapshot
    }
    if (options.validateRequirements !== false) {
      await validatePublicationRequirements(requirements, ownedClient, snapshot.source_session?.workspace_id)
    }
    const materializedConfig = await materializePublicationConfig(config, materializationSnapshot, ownedClient)
    await ensurePublicationRuntimeAttached(ownedClient, materializedConfig)
    return materializedConfig
  } finally {
    if (!options.client) {
      await ownedClient.close?.().catch(() => {})
    }
  }
}

function clonePublicationSnapshot(snapshot: WorkflowPublicationSnapshot): WorkflowPublicationSnapshot {
  return JSON.parse(JSON.stringify(snapshot)) as WorkflowPublicationSnapshot
}

async function loadPublicationRequirements(root: string) {
  try {
    return JSON.parse(await readFile(join(root, "requirements.json"), "utf8")) as WorkflowPublicationRequirements
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") {
      throw new Error("publication package is missing requirements.json")
    }
    throw error
  }
}

export function publicationConfigFromPackage(
  publicationPackage: WorkflowPublicationPackage,
  snapshot: WorkflowPublicationSnapshot,
  kernelEndpoint = defaultKernelEndpoint(),
  hookId?: string,
): WorkflowPublicationConfig {
  if (publicationPackage.schema_version !== 1) {
    throw new Error(`unsupported publication package schema_version ${publicationPackage.schema_version}`)
  }
  if (snapshot.schema_version !== 1) {
    throw new Error(`unsupported workflow snapshot schema_version ${snapshot.schema_version}`)
  }
  const hook = selectPublicationHook(publicationPackage.hooks, hookId)
  const sessionId = publicationPackage.source_session_id ?? snapshot.source_session?.id
  const workflowId = publicationPackage.workflow_id ?? snapshot.workflow?.id
  const endpointId = hook.endpoint_id ?? snapshot.endpoint?.id
  if (!sessionId) throw new Error("publication package is missing source_session_id")
  if (!workflowId) throw new Error("publication package is missing workflow_id")
  if (!endpointId) throw new Error("publication hook is missing endpoint_id")
  const traceExposure = validatePublicationTraceExposure(hook.trace_exposure ?? undefined, snapshot)
  const config: WorkflowPublicationConfig = {
    publication_id: hook.publication_id ?? publicationPackage.publication_id,
    session_id: sessionId,
    source_session_id: sessionId,
    workflow_ref: workflowId,
    endpoint_ref: endpointId,
    hook_id: hook.id,
    queue_ref: hook.queue_ref ?? "default",
    kernel_endpoint: kernelEndpoint,
    transport: hook.transport,
    route: hook.route ?? "/*",
    parser: hook.parser ?? { kind: "json" },
    mode: hook.mode === "async" ? "async" : "sync",
  }
  if (hook.sync_timeout_ms != null) config.sync_timeout_ms = hook.sync_timeout_ms
  if (hook.poll_ms != null) config.poll_ms = hook.poll_ms
  if (traceExposure) {
    config.trace_exposure = traceExposure
    config.trace_context = publicationTraceContextFromSnapshot(snapshot)
  }
  const methods = normalizeHttpMethods(hook.methods)
  if (methods) config.methods = methods
  if (hook.input_schema) config.input_schema = hook.input_schema
  return config
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
    const config = publicationConfigFromKernelRecord(publication, kernelEndpoint)
    if (config.trace_exposure) {
      const sessionResponse = await ownedClient.send(getSessionStateRequest(publication.session_id))
      const session = (sessionResponse.SessionState as { session?: RuntimeSession } | undefined)?.session
      if (session) {
        config.trace_context = publicationTraceContextFromSession(session, publication.workflow_id)
        validatePublicationTraceExposureForNodeIds(config.trace_exposure, Object.keys(config.trace_context.nodes))
      }
    }
    await ensurePublicationRuntimeAttached(ownedClient, config)
    return config
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
  const traceExposure = asTraceExposure(publication.trace_exposure)
  const config: WorkflowPublicationConfig = {
    publication_id: publication.id,
    session_id: publication.session_id,
    workflow_ref: publication.workflow_id,
    endpoint_ref: publication.endpoint_id,
    queue_ref: publication.queue_ref ?? "default",
    kernel_endpoint: kernelEndpoint,
    route: publication.route ?? "/*",
    parser: asParserConfig(publication.parser) ?? { kind: "json" },
    mode: publication.mode === "async" ? "async" : "sync",
  }
  if (publication.sync_timeout_ms != null) config.sync_timeout_ms = publication.sync_timeout_ms
  if (publication.poll_ms != null) config.poll_ms = publication.poll_ms
  if (traceExposure) config.trace_exposure = traceExposure
  const transport = publication.transport as { kind?: unknown } | null | undefined
  if (typeof transport?.kind === "string") config.transport = transport.kind
  const methods = normalizeHttpMethods(publication.methods)
  if (methods) config.methods = methods
  const inputSchema = asInputSchema(publication.input_schema)
  if (inputSchema) config.input_schema = inputSchema
  return config
}

export async function loadGatewayPublicationConfig(): Promise<WorkflowPublicationConfig | undefined> {
  if (process.env.ARROBA_PUBLICATION_PACKAGE) {
    const packageOptions: { kernelEndpoint?: string; hookId?: string } = {
      kernelEndpoint: defaultKernelEndpoint(),
    }
    if (process.env.ARROBA_PUBLICATION_HOOK_ID) {
      packageOptions.hookId = process.env.ARROBA_PUBLICATION_HOOK_ID
    }
    return withEnvTlsConfig(await loadPublicationPackageConfig(process.env.ARROBA_PUBLICATION_PACKAGE, {
      ...packageOptions,
      materialize: true,
    }))
  }
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

export async function materializePublicationConfig(
  config: WorkflowPublicationConfig,
  snapshot: WorkflowPublicationSnapshot,
  client?: KernelLookupClient,
): Promise<WorkflowPublicationConfig> {
  const ownedClient = client ?? new LocalIpcClient(config.kernel_endpoint ?? defaultKernelEndpoint())
  try {
    const response = await ownedClient.send(
      materializeWorkflowPublicationRequest(config.publication_id, snapshot),
    )
    const materialized = response.WorkflowPublicationMaterialized as { session?: { id?: string } } | undefined
    const runtimeSessionId = materialized?.session?.id
    if (!runtimeSessionId) {
      throw new Error(`unexpected workflow publication materialization response: ${JSON.stringify(response)}`)
    }
    return {
      ...config,
      source_session_id: config.source_session_id ?? config.session_id,
      session_id: runtimeSessionId,
    }
  } finally {
    if (!client) {
      await ownedClient.close?.().catch(() => {})
    }
  }
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

function selectPublicationHook(hooks: PublicationHookConfig[], hookId?: string) {
  if (!Array.isArray(hooks) || hooks.length === 0) {
    throw new Error("publication package must include at least one hook")
  }
  if (!hookId) return hooks[0] as PublicationHookConfig
  const hook = hooks.find((candidate) => candidate.id === hookId)
  if (!hook) throw new Error(`publication hook ${hookId} was not found`)
  return hook
}

function publicationTraceContextFromSnapshot(snapshot: WorkflowPublicationSnapshot): PublicationTraceContext {
  const agents = new Map((snapshot.agents ?? []).map((agent) => [agent.id, agent]))
  const nodes = Object.fromEntries((snapshot.workflow.nodes ?? []).map((node) => {
    const agent = agents.get(node.agent_id)
    return [node.id, {
      node_id: node.id,
      node_label: node.public_label ?? node.id,
      agent_id: node.agent_id,
      agent_alias: agent?.alias ?? agent?.agent_ref ?? node.agent_id,
    }]
  }))
  return { nodes }
}

function publicationTraceContextFromSession(session: RuntimeSession, workflowId: string): PublicationTraceContext {
  const workflow = (session.workflows ?? []).find((candidate) => candidate.id === workflowId)
  const agents = new Map((session.agents ?? []).map((agent) => [agent.id, agent]))
  const nodes = Object.fromEntries((workflow?.nodes ?? []).map((node) => {
    const agent = agents.get(node.agent_id)
    return [node.id, {
      node_id: node.id,
      node_label: node.public_label ?? node.id,
      agent_id: node.agent_id,
      agent_alias: agent?.alias ?? agent?.agent_ref ?? node.agent_id,
    }]
  }))
  return { nodes }
}

function validatePublicationTraceExposure(
  traceExposure: PublicationTraceExposurePolicy | undefined,
  snapshot: WorkflowPublicationSnapshot,
) {
  const policy = traceExposure ? asTraceExposure(traceExposure) : undefined
  validatePublicationTraceExposureForNodeIds(policy, (snapshot.workflow.nodes ?? []).map((node) => node.id))
  return policy
}

function validatePublicationTraceExposureForNodeIds(
  traceExposure: PublicationTraceExposurePolicy | undefined,
  nodeIds: string[],
) {
  if (!traceExposure) return
  const allowedLevels = new Set(["output_summary", "assistant_messages", "thinking", "tool_use"])
  const knownNodeIds = new Set(nodeIds)
  for (const [nodeId, levels] of Object.entries(traceExposure.nodes ?? {})) {
    if (!knownNodeIds.has(nodeId)) throw new Error(`publication trace_exposure references unknown workflow node ${nodeId}`)
    if (!Array.isArray(levels)) throw new Error(`publication trace_exposure levels for node ${nodeId} must be an array`)
    for (const level of levels) {
      if (!allowedLevels.has(level)) throw new Error(`publication trace_exposure level ${level} for node ${nodeId} is unsupported`)
    }
  }
}

function asParserConfig(value: unknown): ParserConfig | undefined {
  return isPlainObject(value) && typeof value.kind === "string" ? value as ParserConfig : undefined
}

function asInputSchema(value: unknown): InputSchema | undefined {
  return isPlainObject(value) ? value as InputSchema : undefined
}

function asTraceExposure(value: unknown): PublicationTraceExposurePolicy | undefined {
  return isPlainObject(value) ? value as PublicationTraceExposurePolicy : undefined
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value)
}

function requiredProcessEnv(name: string) {
  const value = process.env[name]
  if (!value) throw new Error(`required env ${name} is not set`)
  return value
}
