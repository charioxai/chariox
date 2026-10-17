import assert from "node:assert/strict"
import http from "node:http"
import { once } from "node:events"
import { setTimeout as sleep } from "node:timers/promises"
import test from "node:test"
import type { LocalIpcClient } from "../ipc.js"
import { startOpenCodeProxy } from "./opencode-proxy.js"

// MP-08/MP-10/MP-11: real HTTP streams exercise transport ownership only;
// this regression fixture is supplementary to the official native TUI drill.
for (const [closeClient, route] of [[true, "/event"], [false, "/event"], [true, "/content"]] as const) {
  test(`MP-08 native OpenCode ${closeClient ? "disconnect" : "shutdown"} releases ${route}`, async (t) => {
    let upstreamClosed = false
    const upstream = http.createServer((_request, response) => {
      response.writeHead(200, { "content-type": route === "/event" ? "text/event-stream" : "application/json" })
      response.write(route === "/event" ? ": ready\n\n" : JSON.stringify({ text: "public response ".repeat(20) }))
      response.once("close", () => { upstreamClosed = true })
    })
    upstream.listen(0, "127.0.0.1")
    await once(upstream, "listening")
    const address = upstream.address()
    assert(address && typeof address !== "string")
    const proxy = await startOpenCodeProxy({
      upstreamBaseUrl: `http://127.0.0.1:${address.port}`,
      client: { send: async () => { throw new Error("disconnect must not mutate kernel state") } } as unknown as LocalIpcClient,
      sessionId: "session", attachmentId: "attachment", agentId: "agent",
      inlineLocalAttachments: false, debug: () => {},
    })
    const proxyAddress = proxy.server.address()
    assert(proxyAddress && typeof proxyAddress !== "string")
    const request = http.get(`http://127.0.0.1:${proxyAddress.port}${route}`)
    const [response] = await once(request, "response")
    response.resume()
    await once(response, "data")
    t.after(async () => {
      response.destroy()
      request.destroy()
      proxy.server.closeAllConnections()
      upstream.closeAllConnections()
      await Promise.all([
        new Promise<void>((resolve) => proxy.server.close(() => resolve())),
        new Promise<void>((resolve) => upstream.close(() => resolve())),
      ])
    })
    if (closeClient) {
      response.destroy()
    } else {
      const lifetime = proxy as typeof proxy & { close?: () => Promise<void> }
      const closed = lifetime.close?.() ?? new Promise<void>((resolve) => proxy.server.close(() => resolve()))
      assert.equal(await Promise.race([closed.then(() => true), sleep(500).then(() => false)]), true,
        "proxy shutdown must finish with a connected SSE client")
    }
    for (let n = 0; n < 50 && !upstreamClosed; n++) await sleep(10)
    assert.equal(upstreamClosed, true, "the abandoned upstream stream must close")
  })
}
