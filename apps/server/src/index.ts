import Fastify from "fastify"

import { createProcessLogger } from "./logging.js"

export const buildServer = () => {
  const processLogger = createProcessLogger("server")
  const logger = processLogger.child("server.http")
  const app = Fastify({ logger: false })

  app.get('/health', async () => {
    logger.debug("handled health request")
    return { status: "ok" }
  })

  app.addHook("onClose", async () => {
    logger.info("server closed")
  })

  return { app, logger }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const { app, logger } = buildServer()
  const host = process.env.HOST ?? "0.0.0.0"
  const port = Number(process.env.PORT ?? 3000)
  logger.info("starting server process", { host, port })

  app
    .listen({ host, port })
    .then((address) => {
      logger.info("server listening", { host, port, address })
    })
    .catch((error) => {
      logger.error("server failed to start", { error: error.message, host, port })
      process.exit(1)
    })
}
