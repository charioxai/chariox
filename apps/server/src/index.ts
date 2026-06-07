import process from "node:process"

import Fastify from "fastify"
import { LocalIpcClient } from "@arroba/kernel-client/ipc"
import {
  registerWorkflowPublicationEndpointRequest,
} from "@arroba/kernel-client/ipc-requests"

import {
  defaultKernelEndpoint,
  invokeKernelWorkflow,
} from "./kernel-publication-client.js"
import { ArrobaLogger, createProcessLogger } from "./logging.js"
import {
  installApiSseJsonRoutes,
  isApiSseJsonPublication,
} from "./publication-api-sse.js"
import {
  forwardHumanHttpResult,
  HUMAN_HTTP_FORM_INVOKE_PATH,
  installHumanHttpRoutes,
  shouldReturnHumanHtml,
} from "./publication-human-http.js"
import {
  defaultPublicationConfig,
  loadGatewayPublicationConfig,
  resolveHttpsOptions,
} from "./publication-config.js"
import {
  registerCloudPublicationDeploymentBackend,
} from "./publication-cloud-deployment.js"
import { forwardWorkflowResult } from "./publication-http-response.js"
import {
  isParseErrorPayload,
  parseAndValidateRequest,
  validateInput,
} from "./publication-parser.js"
import {
  installPublicationMcpRoutes,
  isMcpPublication,
} from "./publication-mcp.js"
import { installRawBodyParsers } from "./publication-raw-body-parsers.js"
import { pumpPublicationRuntime } from "./publication-runtime-pump.js"
import { publicationStatusPayload } from "./publication-status.js"
import { installPublicationViewerRoutes } from "./publication-viewer.js"
import type {
  GatewayDeps,
  GatewayRequest,
  NormalizedInvocation,
  PublicationInvocationOptions,
  WorkflowInvocationResult,
  WorkflowPublicationConfig,
} from "./publication-types.js"
import { installPublicationWebSocket } from "./publication-websocket.js"

export type {
  PublicationInvocationOptions,
  WorkflowPublicationConfig,
} from "./publication-types.js"
export {
  loadGatewayPublicationConfig,
  loadPublicationConfig,
  loadPublicationConfigFromKernel,
  loadPublicationPackageConfig,
  materializePublicationConfig,
  publicationConfigFromKernelRecord,
  publicationConfigFromPackage,
} from "./publication-config.js"

export const buildServer = (config?: WorkflowPublicationConfig, deps: GatewayDeps = {}) => {
  const processLogger = createProcessLogger("workflow-gateway")
  const logger = processLogger.child("gateway.http")
  const publication = config ?? defaultPublicationConfig()
  const httpsOptions = resolveHttpsOptions(publication.tls)
  const app = Fastify({ logger: false, ...(httpsOptions ? { https: httpsOptions } : {}) } as never)
  const webSocketServer = installPublicationWebSocket(app, publication, deps)
  installRawBodyParsers(app)
  installPublicationViewerRoutes(app, publication)
  installHumanHttpRoutes(app, publication)
  installApiSseJsonRoutes(app, publication, deps)
  installPublicationMcpRoutes(app, publication, deps)

  app.get("/health", async () => {
    logger.debug("handled health request")
    return { status: "ok" }
  })

  app.get("/.well-known/arroba/publication/status", async () => publicationStatusPayload(publication, deps))

  app.post(HUMAN_HTTP_FORM_INVOKE_PATH, async (request, reply) => {
    if (publication.transport && publication.transport !== "human_http") {
      reply.code(404)
      return { error: "not found" }
    }
    const parsed = parseHumanHttpFormBody(request.body)
    if (!parsed.ok) {
      reply.code(400).headers({ "content-type": "application/json" })
      return { error: parsed.error }
    }
    const invocation: NormalizedInvocation = {
      publication_id: publication.publication_id,
      request_id: `req_${Date.now()}_${Math.random().toString(16).slice(2)}`,
      caller: { type: "anonymous", proof: { transport: "human_http_form" } },
      input: parsed.input,
      mode: publication.mode ?? "sync",
    }
    const result = deps.invokeWorkflow
      ? await deps.invokeWorkflow(invocation)
      : await invokeKernelWorkflow(publication, invocation)
    return forwardHumanHttpResult(reply, publication, result, invocation.request_id)
  })

  if (!isApiSseJsonPublication(publication) && !isMcpPublication(publication)) {
    const methods = publication.methods?.length ? publication.methods : ["GET", "POST"]
    for (const method of methods) {
      app.route({
        method,
        url: publication.route ?? "/*",
        handler: async (request, reply) => {
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
            caller: { type: "anonymous" },
            input: parsed,
            mode: publication.mode ?? "sync",
          }
          const result = deps.invokeWorkflow
            ? await deps.invokeWorkflow(invocation)
            : await invokeKernelWorkflow(publication, invocation)
          if (shouldReturnHumanHtml(request as unknown as GatewayRequest, publication)) {
            return forwardHumanHttpResult(reply, publication, result, invocation.request_id)
          }
          return forwardWorkflowResult(reply, result)
        },
      })
    }
  }

  app.addHook("onClose", async () => {
    webSocketServer.close()
    logger.info("gateway closed")
  })

  return { app, logger }
}

function parseHumanHttpFormBody(body: unknown): { ok: true; input: Record<string, unknown> } | { ok: false; error: string } {
  if (!body || typeof body !== "object" || Array.isArray(body)) {
    return { ok: false, error: "expected JSON form payload" }
  }
  const record = body as Record<string, unknown>
  const prompt = typeof record.prompt === "string" ? record.prompt : ""
  const artifacts = Array.isArray(record.artifacts)
    ? record.artifacts.filter(isHumanHttpArtifact)
    : []
  if (!prompt.trim() && artifacts.length === 0) {
    return { ok: false, error: "prompt or artifact is required" }
  }
  return {
    ok: true,
    input: artifacts.length > 0 ? { prompt, artifacts } : { prompt },
  }
}

function isHumanHttpArtifact(value: unknown): value is Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false
  const record = value as Record<string, unknown>
  return typeof record.name === "string"
    && typeof record.type === "string"
    && typeof record.size_bytes === "number"
    && typeof record.data_url === "string"
}

export async function invokePublicationInput(
  publication: WorkflowPublicationConfig,
  options: PublicationInvocationOptions,
): Promise<WorkflowInvocationResult> {
  validateInput(options.input, publication.input_schema)
  const invocation: NormalizedInvocation = {
    publication_id: publication.publication_id,
    request_id: `${options.requestIdPrefix ?? "ipc"}_${Date.now()}_${Math.random().toString(16).slice(2)}`,
    caller: options.caller ?? {
      type: "ipc",
      proof: { transport: "ipc" },
    },
    input: options.input,
    mode: options.mode ?? publication.mode ?? "sync",
  }
  return options.deps?.invokeWorkflow
    ? await options.deps.invokeWorkflow(invocation)
    : await invokeKernelWorkflow(publication, invocation)
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const config = await loadGatewayPublicationConfig()
  const { app, logger } = buildServer(config)
  const host = process.env.HOST ?? "0.0.0.0"
  const port = Number(process.env.PORT ?? 3000)
  logger.info("starting workflow gateway", { host, port })

  app
    .listen({ host, port })
    .then(async (address) => {
      logger.info("workflow gateway listening", { host, port, address })
      await registerServedPublicationEndpoint(config, host, port, logger)
      schedulePublicationEndpointRegistration(config, host, port, logger)
      schedulePublicationRuntimePump(config, logger)
    })
    .catch((error) => {
      logger.error("workflow gateway failed to start", { error: error.message, host, port })
      process.exit(1)
    })
}

function schedulePublicationEndpointRegistration(
  publication: WorkflowPublicationConfig | undefined,
  host: string,
  port: number,
  logger: ArrobaLogger,
) {
  if (!publication) return
  const interval = setInterval(() => {
    void registerServedPublicationEndpoint(publication, host, port, logger)
  }, 5 * 60 * 1_000)
  interval.unref?.()
}

function schedulePublicationRuntimePump(
  publication: WorkflowPublicationConfig | undefined,
  logger: ArrobaLogger,
) {
  if (!publication) return
  let pumping = false
  const interval = setInterval(() => {
    if (pumping) return
    pumping = true
    const client = new LocalIpcClient(publication.kernel_endpoint ?? defaultKernelEndpoint())
    void pumpPublicationRuntime(client, publication)
      .catch((error) => {
        logger.debug("workflow gateway runtime pump failed", {
          error: error instanceof Error ? error.message : String(error),
          publication_id: publication.publication_id,
          runtime_session_id: publication.session_id,
        })
      })
      .finally(() => {
        pumping = false
        void client.close().catch(() => {})
      })
  }, Math.max(250, publication.poll_ms ?? 500))
  interval.unref?.()
}

async function registerServedPublicationEndpoint(
  publication: WorkflowPublicationConfig | undefined,
  host: string,
  port: number,
  logger: ArrobaLogger,
) {
  if (!publication) return
  const sessionId = publication.session_id
  if (!sessionId || !publication.publication_id) return
  const localUrl = servedLocalUrl(publication, host, port)
  const client = new LocalIpcClient(publication.kernel_endpoint ?? defaultKernelEndpoint())
  try {
    const response = await client.send<Record<string, unknown>>(
      registerWorkflowPublicationEndpointRequest(sessionId, publication.publication_id, localUrl, {
        runtimeSessionId: publication.session_id,
      }),
    )
    const registered = response.WorkflowPublicationEndpointRegistered as {
      open_url?: string
      access?: string
      expires_at_ms?: number | null
    } | undefined
    logger.info("registered workflow publication endpoint", {
      publication_id: publication.publication_id,
      local_url: localUrl,
      open_url: registered?.open_url ?? localUrl,
      access: registered?.access ?? "local",
      expires_at_ms: registered?.expires_at_ms ?? null,
    })
    await registerCloudDeploymentBackendIfConfigured(publication, registered?.open_url ?? localUrl, logger)
  } catch (error) {
    logger.warn("failed to register workflow publication endpoint", {
      publication_id: publication.publication_id,
      local_url: localUrl,
      error: error instanceof Error ? error.message : String(error),
    })
  } finally {
    await client.close().catch(() => {})
  }
}

async function registerCloudDeploymentBackendIfConfigured(
  publication: WorkflowPublicationConfig,
  localUrl: string,
  logger: ArrobaLogger,
) {
  const deploymentId = process.env.ARROBA_PUBLICATION_CLOUD_DEPLOYMENT_ID?.trim()
  if (!deploymentId) return
  try {
    const registered = await registerCloudPublicationDeploymentBackend({
      deploymentId,
      publication,
      localUrl,
    })
    if (registered) {
      logger.info("registered Cloud publication deployment backend", {
        publication_id: publication.publication_id,
        deployment_id: deploymentId,
        local_url: localUrl,
      })
    } else {
      logger.warn("Cloud publication deployment backend registration skipped; Cloud profile is missing", {
        publication_id: publication.publication_id,
        deployment_id: deploymentId,
      })
    }
  } catch (error) {
    logger.warn("failed to register Cloud publication deployment backend", {
      publication_id: publication.publication_id,
      deployment_id: deploymentId,
      local_url: localUrl,
      error: error instanceof Error ? error.message : String(error),
    })
  }
}

function servedLocalUrl(publication: WorkflowPublicationConfig, host: string, port: number) {
  const scheme = publication.tls?.enabled === false || !publication.tls ? "http" : "https"
  const hostname = host === "0.0.0.0" || host === "::" ? "127.0.0.1" : host
  const bracketed = hostname.includes(":") && !hostname.startsWith("[") ? `[${hostname}]` : hostname
  return `${scheme}://${bracketed}:${port}/`
}
