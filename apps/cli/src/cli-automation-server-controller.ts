import type {
  CliAutomationRequest,
  CliAutomationServer,
} from "./cli-automation.js"

export type CliAutomationServerLogger = {
  info: (message: string, fields?: Record<string, unknown>) => void
  error: (message: string, fields?: Record<string, unknown>) => void
}

export type CliAutomationServerControllerDeps = {
  socketPath: string | undefined
  handleRequest: (request: CliAutomationRequest) => Promise<unknown> | unknown
  startServer: (options: {
    socketPath: string
    handleRequest: (request: CliAutomationRequest) => Promise<unknown> | unknown
    formatError: (error: unknown) => string
    onListening?: (socketPath: string) => void
  }) => Promise<CliAutomationServer>
  stopServer: (server: CliAutomationServer, socketPath: string) => void
  formatError: (error: unknown) => string
  logger?: CliAutomationServerLogger | null
  flashFooter: (message: string, tone: "error") => void
}

export type CliAutomationServerController = {
  start(): void
  stop(): void
}

export function createCliAutomationServerController(
  deps: CliAutomationServerControllerDeps,
): CliAutomationServerController {
  let server: CliAutomationServer | null = null

  return {
    start() {
      const socketPath = deps.socketPath
      if (!socketPath) {
        return
      }
      void deps.startServer({
        socketPath,
        handleRequest: deps.handleRequest,
        formatError: deps.formatError,
        onListening: (listeningSocketPath) => {
          deps.logger?.info("cli automation socket listening", { socket_path: listeningSocketPath })
        },
      })
        .then((startedServer) => {
          server = startedServer
        })
        .catch((error) => {
          deps.logger?.error("failed to start cli automation socket", {
            socket_path: socketPath,
            error: deps.formatError(error),
          })
          deps.flashFooter(`automation socket failed: ${deps.formatError(error)}`, "error")
        })
    },

    stop() {
      const socketPath = deps.socketPath
      if (!server || !socketPath) {
        return
      }
      deps.stopServer(server, socketPath)
      server = null
    },
  }
}
