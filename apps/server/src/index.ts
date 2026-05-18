import process from "node:process"

import Fastify from "fastify"

import {
  invokeKernelWorkflow,
  redeemPublicationPairCode,
} from "./kernel-publication-client.js"
import { createProcessLogger } from "./logging.js"
import {
  authenticateRequest,
  handleConnectorHandshake,
  isPairedSenderAuthEnabled,
  objectBody,
} from "./publication-auth.js"
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
  publicationConfigFromKernelRecord,
} from "./publication-config.js"

export const buildServer = (config?: WorkflowPublicationConfig, deps: GatewayDeps = {}) => {
  const processLogger = createProcessLogger("workflow-gateway")
  const logger = processLogger.child("gateway.http")
  const publication = config ?? defaultPublicationConfig()
  const httpsOptions = resolveHttpsOptions(publication.tls)
  const app = Fastify({ logger: false, ...(httpsOptions ? { https: httpsOptions } : {}) } as never)
  const webSocketServer = installPublicationWebSocket(app, publication, deps)
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
    webSocketServer.close()
    logger.info("gateway closed")
  })

  return { app, logger }
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
      proof: { auth: "ipc", connector: "ipc" },
    },
    input: options.input,
    mode: options.mode ?? publication.mode ?? "sync",
  }
  return options.deps?.invokeWorkflow
    ? await options.deps.invokeWorkflow(invocation)
    : await invokeKernelWorkflow(publication, invocation)
}

function optionalString(value: unknown) {
  return typeof value === "string" && value.trim() ? value.trim() : null
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
