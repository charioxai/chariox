import { spawn, type ChildProcess } from "node:child_process"
import net from "node:net"
import { setTimeout as sleep } from "node:timers/promises"

type EndpointBridge = {
  endpoint: string
  close: () => Promise<void>
}

export async function bridgeRemoteNativeProviderEndpoint(
  endpoint: string,
  label: string,
): Promise<EndpointBridge> {
  const sshHost = process.env.ARROBA_NATIVE_PROVIDER_ENDPOINT_SSH_HOST?.trim()
  if (!sshHost) {
    return { endpoint, close: async () => {} }
  }

  const url = new URL(endpoint)
  if (!isLoopbackHost(url.hostname)) {
    return { endpoint, close: async () => {} }
  }
  if (!url.port) {
    throw new Error(`cannot bridge ${label} native provider endpoint without a port: ${endpoint}`)
  }

  const localPort = await reservePort()
  const remoteHost = process.env.ARROBA_NATIVE_PROVIDER_ENDPOINT_SSH_REMOTE_HOST?.trim() || "127.0.0.1"
  const sshKey = process.env.ARROBA_NATIVE_PROVIDER_ENDPOINT_SSH_KEY?.trim() || undefined
  const child = spawn("ssh", sshForwardArgs({
    sshHost,
    ...(sshKey ? { sshKey } : {}),
    localPort,
    remoteHost,
    remotePort: Number(url.port),
  }), {
    stdio: ["ignore", "ignore", "inherit"],
  })
  await waitForForward(child, localPort, label)

  url.hostname = "127.0.0.1"
  url.port = String(localPort)
  return {
    endpoint: url.toString(),
    close: async () => {
      await terminateChild(child)
    },
  }
}

function sshForwardArgs(options: {
  sshHost: string
  sshKey?: string
  localPort: number
  remoteHost: string
  remotePort: number
}): string[] {
  const args = [
    "-o",
    "BatchMode=yes",
    "-o",
    "StrictHostKeyChecking=accept-new",
    "-N",
    "-L",
    `127.0.0.1:${options.localPort}:${options.remoteHost}:${options.remotePort}`,
  ]
  if (options.sshKey) {
    args.unshift("-i", options.sshKey)
  }
  args.push(options.sshHost)
  return args
}

async function waitForForward(child: ChildProcess, port: number, label: string): Promise<void> {
  const deadline = Date.now() + 15_000
  const exited: { value?: { code: number | null, signal: NodeJS.Signals | null } } = {}
  child.once("exit", (code, signal) => {
    exited.value = { code, signal }
  })
  while (Date.now() < deadline) {
    if (exited.value) {
      throw new Error(`${label} native provider endpoint SSH bridge exited before becoming ready: ${exited.value.signal ?? exited.value.code}`)
    }
    if (await portIsReachable(port)) return
    await sleep(100)
  }
  throw new Error(`${label} native provider endpoint SSH bridge did not become ready on 127.0.0.1:${port}`)
}

async function reservePort(): Promise<number> {
  return await new Promise((resolve, reject) => {
    const server = net.createServer()
    server.once("error", reject)
    server.listen(0, "127.0.0.1", () => {
      const address = server.address()
      if (!address || typeof address === "string") {
        server.close(() => reject(new Error("endpoint bridge port reservation did not expose a TCP address")))
        return
      }
      const port = address.port
      server.close(() => resolve(port))
    })
  })
}

async function portIsReachable(port: number): Promise<boolean> {
  return await new Promise((resolve) => {
    const socket = net.connect({ host: "127.0.0.1", port })
    socket.once("connect", () => {
      socket.destroy()
      resolve(true)
    })
    socket.once("error", () => {
      socket.destroy()
      resolve(false)
    })
  })
}

async function terminateChild(child: ChildProcess): Promise<void> {
  if (child.exitCode != null || child.signalCode != null) return
  child.kill("SIGTERM")
  await Promise.race([
    new Promise((resolve) => child.once("exit", resolve)),
    sleep(3_000),
  ])
  if (child.exitCode == null && child.signalCode == null) {
    child.kill("SIGKILL")
  }
}

function isLoopbackHost(host: string): boolean {
  return host === "127.0.0.1" || host === "localhost" || host === "::1"
}
