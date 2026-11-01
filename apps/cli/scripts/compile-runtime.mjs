import { mkdtemp, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import path from "node:path"
import WebSocket from "ws"

// Unix WebSocket clients were added in Bun 1.3.13. The runtime is embedded in
// the release executable, so upgrading the user's installed Bun cannot fix it.
export const minimumBunVersion = "1.3.13"

export function requireBunVersion(version) {
  const parts = /^(\d+)\.(\d+)\.(\d+)$/.exec(version ?? "")?.slice(1).map(Number)
  const minimum = minimumBunVersion.split(".").map(Number)
  const difference = parts?.findIndex((part, index) => part !== minimum[index])
  if (!parts || (difference !== -1 && parts[difference] < minimum[difference])) {
    throw new Error(`CLI compilation requires stable Bun >= ${minimumBunVersion} for Unix WebSocket access; found ${version ?? "no Bun runtime"}. Install Bun 1.4.2 or newer and rebuild the CLI.`)
  }
}

export async function checkUnixWebSocket() {
  const scratch = await mkdtemp(path.join(tmpdir(), "chx-bun-"))
  const socketPath = path.join(scratch, "k.sock")
  let server, socket, timer
  try {
    server = Bun.serve({
      unix: socketPath,
      fetch(request, server) {
        if (new URL(request.url).pathname === "/kernel" && server.upgrade(request)) return
        return new Response("Expected a /kernel WebSocket upgrade", { status: 400 })
      },
      websocket: { message(socket, message) { socket.send(message) } },
    })
    await new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error("Unix WebSocket handshake timed out")), 5_000)
      socket = new WebSocket(`ws+unix:${socketPath}:/kernel`)
      socket.onopen = () => socket.send("chariox-compile-check")
      socket.onmessage = event => event.data === "chariox-compile-check"
        ? resolve() : reject(new Error("Unexpected Unix WebSocket response"))
      socket.onerror = () => reject(new Error("Unix WebSocket connection failed"))
    })
  } catch (error) {
    throw new Error(`Bun ${Bun.version} cannot open the CLI's Unix WebSocket transport; install Bun 1.4.2 or newer and rebuild the CLI. ${error.message}`, { cause: error })
  } finally {
    clearTimeout(timer)
    socket?.close()
    await server?.stop(true)
    await rm(scratch, { recursive: true, force: true })
  }
}

export async function checkCompileRuntime() {
  requireBunVersion(globalThis.Bun?.version)
  await checkUnixWebSocket()
}
