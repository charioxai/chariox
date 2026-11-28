// Shared bits for the owned fixtures: one node http server per origin, state written mode 600 outside the checkout, hashes only in logs.
import { createServer as createHttp } from "node:http"
import { createServer as createHttps } from "node:https"
import { createHash, randomBytes, randomUUID } from "node:crypto"
import { chmodSync, mkdirSync, writeFileSync } from "node:fs"
import { join } from "node:path"

export const sha8 = value => createHash("sha256").update(String(value)).digest("hex").slice(0, 8)
export const newToken = prefix => `${prefix}_${randomBytes(24).toString("hex")}`
export const esc = text => String(text).replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c])

export function page(title, body, { head = "" } = {}) {
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>${esc(title)}</title>
<style>body{font:18px system-ui;margin:0;background:#f5f3f0}main{box-sizing:border-box;max-width:520px;padding:24px;margin:16px auto;background:#fff;border:1px solid #ccc;border-radius:8px}input,button{font:inherit;box-sizing:border-box;padding:10px;width:100%;margin:8px 0}.big{font:700 44px/1.2 monospace;word-break:break-all}</style>${head}</head><body>${body}</body></html>`
}

export async function readBody(req, limit = 8192) {
  let body = ""
  for await (const chunk of req) { body += chunk; if (body.length > limit) { req.destroy(); throw new Error("body too large") } }
  return body
}

/** Starts one origin. handler(req, res, url, reply) returns true when it answered. */
export async function startOrigin(name, handler, { host = "127.0.0.1", port = 0, tls = null, publicHost = null } = {}) {
  const reply = (res, status, value, type = "application/json") => {
    res.writeHead(status, { "content-type": type + (type.startsWith("text/") ? "; charset=utf-8" : ""), "cache-control": "no-store" })
    res.end(typeof value === "string" ? value : JSON.stringify(value))
  }
  const onRequest = async (req, res) => {
    const url = new URL(req.url, "http://fixture.invalid")
    try { if (!(await handler(req, res, url, (status, value, type) => reply(res, status, value, type)))) reply(res, 404, { error: "not found" }) }
    catch (error) { if (!res.headersSent) reply(res, 500, { error: String(error.message) }) }
  }
  const server = tls ? createHttps(tls, onRequest) : createHttp(onRequest)
  await new Promise(resolve => server.listen(port, host, resolve))
  const shownHost = publicHost ?? (host.includes(":") ? `[${host}]` : host)
  const origin = `${tls ? "https" : "http"}://${shownHost}:${server.address().port}`
  return { name, origin, close: async () => { server.closeAllConnections(); await new Promise(resolve => server.close(resolve)) } }
}

/** Mode-600 state file under the evidence dir (never inside the checkout). Holds issued tokens so the validator never reads the Vault. */
export function writeState(stateDir, file, value) {
  mkdirSync(stateDir, { recursive: true, mode: 0o700 })
  const path = join(stateDir, file)
  writeFileSync(path, typeof value === "string" ? value : JSON.stringify(value, null, 2), { mode: 0o600 })
  chmodSync(path, 0o600)
  return path
}

export const documentId = () => randomUUID().slice(0, 8)
