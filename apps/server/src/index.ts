import process from "node:process"

import Fastify from "fastify"

import {
  invokeKernelWorkflow,
} from "./kernel-publication-client.js"
import { createProcessLogger } from "./logging.js"
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
import { forwardWorkflowResult } from "./publication-http-response.js"
import {
  isParseErrorPayload,
  parseAndValidateRequest,
  validateInput,
} from "./publication-parser.js"
import { installRawBodyParsers } from "./publication-raw-body-parsers.js"
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
  installHumanHttpRoutes(app, publication)
  installApiSseJsonRoutes(app, publication, deps)

  app.get("/health", async () => {
    logger.debug("handled health request")
    return { status: "ok" }
  })

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

  if (!isApiSseJsonPublication(publication)) {
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
    .then((address) => {
      logger.info("workflow gateway listening", { host, port, address })
    })
    .catch((error) => {
      logger.error("workflow gateway failed to start", { error: error.message, host, port })
      process.exit(1)
    })
}
