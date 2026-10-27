import { createConnection } from "node:net"

import { describeCliError } from "./runtime.js"

export function isNoArgDefaultKernelLaunch(argv: string[]): boolean {
  return argv.length === 0
}

export async function isKernelEndpointReachable(endpoint: string): Promise<boolean> {
  let url: URL
  try {
    url = new URL(endpoint)
  } catch {
    return true
  }
  if (url.protocol !== "ws:" && url.protocol !== "wss:") {
    return true
  }

  const port = Number(url.port || (url.protocol === "wss:" ? 443 : 80))
  if (!Number.isFinite(port) || port <= 0) {
    return true
  }
  return new Promise((resolve) => {
    const socket = createConnection({ host: url.hostname, port })
    const settle = (reachable: boolean) => {
      socket.removeAllListeners()
      socket.destroy()
      resolve(reachable)
    }
    socket.setTimeout(500)
    socket.once("connect", () => settle(true))
    socket.once("error", () => settle(false))
    socket.once("timeout", () => settle(false))
  })
}

export function isKernelEndpointUnavailableError(error: unknown): boolean {
  const message = describeCliError(error)
  return /\bECONNREFUSED\b|\bENOENT\b|\bEHOSTUNREACH\b|\bENETUNREACH\b|\bETIMEDOUT\b/i.test(message)
    || /kernel transport `connect kernel websocket` failed: \[object ErrorEvent\]/i.test(message)
}
