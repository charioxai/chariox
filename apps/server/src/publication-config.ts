import { readFileSync } from "node:fs"
import { readFile } from "node:fs/promises"
import { dirname, join, resolve } from "node:path"
import process from "node:process"

import { LocalIpcClient } from "@arroba/kernel-client/ipc"
import { getWorkflowPublicationRequest } from "@arroba/kernel-client/ipc-requests"
import type {
  WorkflowPublicationDefinition,
} from "@arroba/kernel-client/kernel-types"

import { defaultKernelEndpoint } from "./kernel-publication-client.js"
import type {
  InputSchema,
  KernelLookupClient,
  ParserConfig,
  PublicationHookConfig,
  TlsConfig,
  WorkflowPublicationPackage,
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
  options: { kernelEndpoint?: string; hookId?: string } = {},
): Promise<WorkflowPublicationConfig> {
  const packagePath = path.endsWith(".json") ? path : join(path, "publication.json")
  const root = dirname(resolve(packagePath))
  const publicationPackage = JSON.parse(await readFile(packagePath, "utf8")) as WorkflowPublicationPackage
  const snapshotPath = join(root, "workflow.snapshot.json")
  const snapshot = JSON.parse(await readFile(snapshotPath, "utf8")) as WorkflowPublicationSnapshot
  return publicationConfigFromPackage(
    publicationPackage,
    snapshot,
    options.kernelEndpoint ?? defaultKernelEndpoint(),
    options.hookId,
  )
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
  const config: WorkflowPublicationConfig = {
    publication_id: hook.publication_id ?? publicationPackage.publication_id,
    session_id: sessionId,
    workflow_ref: workflowId,
    endpoint_ref: endpointId,
    kernel_endpoint: kernelEndpoint,
    route: hook.route ?? "/*",
    parser: hook.parser ?? { kind: "json" },
    mode: hook.mode === "async" ? "async" : "sync",
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
  if (process.env.ARROBA_PUBLICATION_PACKAGE) {
    const packageOptions: { kernelEndpoint?: string; hookId?: string } = {
      kernelEndpoint: defaultKernelEndpoint(),
    }
    if (process.env.ARROBA_PUBLICATION_HOOK_ID) {
      packageOptions.hookId = process.env.ARROBA_PUBLICATION_HOOK_ID
    }
    return withEnvTlsConfig(await loadPublicationPackageConfig(process.env.ARROBA_PUBLICATION_PACKAGE, packageOptions))
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

function asParserConfig(value: unknown): ParserConfig | undefined {
  return isPlainObject(value) && typeof value.kind === "string" ? value as ParserConfig : undefined
}

function asInputSchema(value: unknown): InputSchema | undefined {
  return isPlainObject(value) ? value as InputSchema : undefined
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value)
}

function requiredProcessEnv(name: string) {
  const value = process.env[name]
  if (!value) throw new Error(`required env ${name} is not set`)
  return value
}
