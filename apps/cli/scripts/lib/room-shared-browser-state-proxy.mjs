import assert from "node:assert/strict"
import { isIP } from "node:net"

export function roomSharedBrowserStateFixtureGateway(networks) {
  const attached = networks && typeof networks === "object" && !Array.isArray(networks)
    ? Object.values(networks)
    : []
  assert.equal(attached.length, 1, "Room Browser fixture requires one slice Docker network")
  const gateway = attached[0]?.Gateway
  assert.ok(typeof gateway === "string" && isIP(gateway) && gateway !== "0.0.0.0"
    && gateway !== "::" && gateway !== "::1" && !gateway.startsWith("127."),
  "Room Browser fixture Docker network has no usable gateway")
  return gateway
}

export async function startRoomSharedBrowserStateProxy({ docker, containerName, port, waitFor }) {
  assert.ok(typeof docker === "function" && typeof waitFor === "function",
    "Room Browser fixture proxy requires Docker and wait adapters")
  assert.ok(typeof containerName === "string" && containerName.length > 0)
  assert.ok(Number.isInteger(port) && port > 0 && port <= 65535)
  const inspected = await docker(["inspect", containerName, "--format", "{{json .NetworkSettings.Networks}}"])
  const gateway = roomSharedBrowserStateFixtureGateway(JSON.parse(inspected.stdout))
  await docker(["exec", "-d", "-u", "slice", containerName, "node", "--input-type=module", "-e",
    `await (${startRoomSharedBrowserFixtureProxy.toString()})(${JSON.stringify({
      port, upstreamHost: gateway, upstreamPort: port,
    })})`])
  await waitFor(async () => {
    try {
      await docker(["exec", "-u", "slice", containerName, "curl", "--fail", "--silent", "--max-time", "2",
        "--output", "/dev/null", `http://127.0.0.1:${port}/click`], 5_000)
      return true
    } catch {
      return false
    }
  }, 15_000, "slice loopback Browser fixture proxy did not become reachable")
  return { origin: `http://127.0.0.1:${port}` }
}

// Runs inside the slice so Chromium uses a real loopback secure context while
// the deterministic fixture and its authentication ledger remain on the host.
export async function startRoomSharedBrowserFixtureProxy({ port, upstreamHost, upstreamPort }) {
  const net = await import("node:net")
  const sockets = new Set()
  const server = net.createServer((incoming) => {
    const outgoing = net.connect({ host: upstreamHost, port: upstreamPort })
    sockets.add(incoming)
    sockets.add(outgoing)
    const close = () => { incoming.destroy(); outgoing.destroy() }
    for (const socket of [incoming, outgoing]) {
      socket.setTimeout(10_000, close)
      socket.on("error", close)
      socket.once("close", () => { sockets.delete(socket); close() })
    }
    incoming.pipe(outgoing).pipe(incoming)
  })
  server.maxConnections = 16
  await new Promise((resolve, reject) => {
    server.once("error", reject)
    server.listen(port, "127.0.0.1", resolve)
  })
  return {
    address: server.address(),
    close: () => new Promise((resolve, reject) => {
      for (const socket of sockets) socket.destroy()
      server.close(error => error ? reject(error) : resolve())
    }),
  }
}
