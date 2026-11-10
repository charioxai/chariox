// Test support: the few Cloud endpoints a kernel started with its owner's profile needs, answered by one loopback server from the local relay's scoped issuer.
// It lets two drill kernels act for one owner (peer agents of other kernels) without a Cloud account. It issues the tokens Cloud's /relay/token issues and
// lists the kernels that reported presence; anything else it refuses by name so a missing endpoint shows up in the drill's notes.
import http from "node:http"
import { randomBytes } from "node:crypto"

export async function startOwnerCloud({ token, note, realm, relayPort }) {
  const kernels = new Map()
  const answer = (res, body, code = 200) => res.writeHead(code, { "content-type": "application/json" }).end(JSON.stringify(body))
  const server = http.createServer((req, res) => {
    let raw = ""
    req.on("data", c => { raw += c })
    req.on("end", () => {
      try {
        const url = new URL(req.url, "http://local")
        const ask = raw ? JSON.parse(raw) : {}
        if (req.method === "GET" && url.pathname === "/relay/targets") return answer(res, { targets: [...kernels.values()].map(k => ({ daemonId: k.kernelId, status: "ONLINE", alias: k.alias })) })
        if (url.pathname === "/auth/kernel/migrate") {
          kernels.set(ask.kernelId, { kernelId: ask.kernelId, alias: null })
          note("owner-cloud-migrate", { kernel: ask.kernelId })
          return answer(res, { kernelCredential: `kernel-credential-${randomBytes(8).toString("hex")}` })
        }
        if (url.pathname === "/kernels/presence") {
          kernels.set(ask.kernelId, { kernelId: ask.kernelId, alias: ask.kernelAlias ?? null })
          return answer(res, {})
        }
        if (url.pathname === "/relay/token") {
          const actions = ask.allowedActions ?? ["daemon.register", "daemon.heartbeat", "packet.route", "peer.request", "peer.event", "client.metadata.read"]
          note("owner-cloud-token", { subject: ask.subject, kind: ask.subjectKind, actions, bound: !!ask.publicKeyThumbprint, targets: ask.allowedTargets ?? null })
          return answer(res, { token: token(ask.subject, ask.subjectKind, actions, ask.publicKeyThumbprint, ask.allowedTargets), expiresAt: new Date(Date.now() + 7000 * 1000).toISOString() })
        }
        note("owner-cloud-unhandled", { method: req.method, url: req.url, keys: Object.keys(ask) })
        answer(res, { error: "not served by the drill" }, 404)
      } catch (e) { answer(res, { error: String(e.message) }, 400) }
    })
  })
  await new Promise(r => server.listen(0, "127.0.0.1", r))
  server.unref()
  const profile = { api_url: `http://127.0.0.1:${server.address().port}`, email: "drill@example.invalid", account_id: "drill-account", user_id: "local", account_slug: "drill", realm_id: realm, relay_url: `ws://127.0.0.1:${relayPort}`, issuer_id: realm, machine_id: "machine-drill", machine_credential: randomBytes(16).toString("hex") }
  return { CHARIOX_CLOUD_RELAY_CONFIG_JSON: JSON.stringify(profile) }
}
