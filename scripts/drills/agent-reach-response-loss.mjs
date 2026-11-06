// MP-08 / MP-09 / MP-10 / MP-11: lose one completed gateway reply on the real relay path.
import { createRequire } from "node:module"
const { WebSocket, WebSocketServer } = createRequire(new URL("../../packages/kernel-client/package.json", import.meta.url))("ws")

export async function responseLossProxy(relayUrl, note) {
  const server = new WebSocketServer({host: "127.0.0.1", port: 0})
  await new Promise(resolve => server.once("listening", resolve))
  const sockets = new Set()
  let remaining = null, drops = 0
  server.on("connection", (client, request) => {
    const upstream = new WebSocket(new URL(request.url, relayUrl))
    sockets.add(client); sockets.add(upstream)
    const queued = []
    const send = (socket, data, binary) => { if (socket.readyState === WebSocket.OPEN) socket.send(data, {binary}) }
    client.on("message", (data, binary) => {
      if (upstream.readyState === WebSocket.CONNECTING) queued.push([data, binary])
      else send(upstream, data, binary)
    })
    upstream.on("open", () => { for (const [data, binary] of queued) send(upstream, data, binary); queued.length = 0 })
    upstream.on("message", (data, binary) => {
      let envelope
      try {
        const frame = JSON.parse(data.toString())
        envelope = frame.kind === "transport_chunk" && frame.offset === 0 && Buffer.byteLength(frame.payload) === frame.total_bytes ? JSON.parse(frame.payload) : frame
      } catch {}
      if (remaining !== null && envelope?.kind === "client_response" && envelope.request_id?.startsWith("gw-") && envelope.encrypted_response) {
        if (remaining-- === 0) {
          remaining = null; drops++
          note("gateway-response-lost", {request_id: envelope.request_id})
          client.terminate(); upstream.terminate()
          return
        }
      }
      send(client, data, binary)
    })
    const retire = socket => { sockets.delete(socket) }
    client.on("close", () => { retire(client); upstream.terminate() })
    upstream.on("close", () => { retire(upstream); client.terminate() })
    client.on("error", () => upstream.terminate())
    upstream.on("error", () => client.terminate())
  })
  return {
    url: `ws://127.0.0.1:${server.address().port}`,
    arm: (skip = 0) => { remaining = skip },
    get drops() { return drops },
    close: async () => { for (const socket of sockets) socket.terminate(); await new Promise(resolve => server.close(resolve)) },
  }
}
