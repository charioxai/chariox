#!/usr/bin/env bun
// Stands in for a generator whose action replies are lost (V1-INT-08). Every
// request is proxied to the real generator, except POST /v1/actions: its
// action is recorded (the effect) and the connection is then dropped with no
// response, as if the reply had been lost after the generator acted.
// usage: bun lossy-generator.mjs --listen PORT --upstream URL --record FILE
import { appendFileSync } from "node:fs"
import http from "node:http"

const args = Object.fromEntries(process.argv.slice(2).reduce((pairs, value, index, all) =>
  index % 2 ? pairs : [...pairs, [value.replace(/^--/, ""), all[index + 1]]], []))
if (!args.listen || !args.upstream || !args.record) throw new Error("usage: --listen PORT --upstream URL --record FILE")

const read = (request) => new Promise((resolve, reject) => {
  const chunks = []
  request.on("data", (chunk) => chunks.push(chunk))
  request.on("end", () => resolve(Buffer.concat(chunks)))
  request.on("error", reject)
})

http.createServer(async (request, response) => {
  const body = await read(request)
  if (request.method === "POST" && request.url === "/v1/actions") {
    const action = JSON.parse(body.toString("utf8"))
    // Never the authorization header: only what identifies the action.
    appendFileSync(args.record, JSON.stringify({ at_ms: Date.now(), action_id: action.action_id,
      connection_id: action.connection_id, idempotency_key: action.idempotency_key, input: action.input }) + "\n")
    request.socket.destroy()
    return
  }
  const upstream = await fetch(new URL(request.url, args.upstream), {
    method: request.method, headers: request.headers, body: body.length ? body : undefined,
  })
  response.writeHead(upstream.status, Object.fromEntries(upstream.headers))
  response.end(Buffer.from(await upstream.arrayBuffer()))
}).listen(Number(args.listen), "127.0.0.1")
