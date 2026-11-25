#!/usr/bin/env node
// Starts F1..F5 and prints their origins. One process, started and stopped by the validator.
//   node serve.mjs --state-dir /abs/evidence/fixtures [--host 127.0.0.1|::] [--public-host NAME] [--base-port 8441] [--alt-host NAME] [--tls-key K --tls-cert C]
// Each fixture is its own origin (own port), so cross-origin and popup cases are real. State (issued tokens) goes to <state-dir>, mode 600.
import { readFileSync } from "node:fs"
import { writeState } from "./common.mjs"
import { startHumanCheck } from "./human-check-page.mjs"
import { startLoginForm } from "./login-form-page.mjs"
import { startOauthPopup } from "./oauth-popup-idp.mjs"
import { startRevealCopyQr } from "./reveal-copy-qr-page.mjs"
import { startSignup } from "./signup-page.mjs"

export async function startAll({ stateDir, host = "127.0.0.1", publicHost = null, basePort = 8441, tls = null, altHost = null }) {
  const net = offset => ({ host, port: basePort + offset, tls, publicHost: publicHost ?? undefined })
  const proto = tls ? "https" : "http", shown = publicHost ?? (host.includes(":") ? `[${host}]` : host)
  const at = offset => `${proto}://${shown}:${basePort + offset}`
  // altHost: a second host name for the SAME server (localhost for 127.0.0.1, a sslip.io sub-name on the Boat), so idp, grid and the post-login page are cross-SITE.
  const atAlt = offset => altHost ? `${proto}://${altHost}:${basePort + offset}` : at(offset)
  const signup = await startSignup({ net: net(0) })
  const reveal = await startRevealCopyQr({ stateDir, net: net(1) })
  const login = await startLoginForm({ net: net(2), welcomeOrigin: altHost ? atAlt(2) : "" })
  const oauth = await startOauthPopup({ rpNet: net(3), idpNet: net(4), rpPublic: at(3), idpPublic: atAlt(4) })
  const human = await startHumanCheck({ parentNet: net(5), gridNet: net(6), gridPublic: atAlt(6) })
  const origins = { signup: signup.origin, reveal: reveal.origin, login: login.origin, oauth_rp: oauth.rp.origin, oauth_idp: atAlt(4), human: human.parent.origin, grid: atAlt(6) }
  if (stateDir) writeState(stateDir, "origins.json", origins)
  return { origins, close: async () => { for (const s of [signup, reveal, login, oauth, human]) await s.close() }, reveal, signup, login, oauth, human }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const flags = new Map(); for (let i = 2; i < process.argv.length; i += 2) flags.set(process.argv[i], process.argv[i + 1])
  const tls = flags.get("--tls-key") ? { key: readFileSync(flags.get("--tls-key")), cert: readFileSync(flags.get("--tls-cert")) } : null
  const all = await startAll({ stateDir: flags.get("--state-dir"), host: flags.get("--host") ?? "127.0.0.1", publicHost: flags.get("--public-host"), basePort: Number(flags.get("--base-port") ?? 8441), tls, altHost: flags.get("--alt-host") })
  console.log(JSON.stringify(all.origins))
  for (const signal of ["SIGINT", "SIGTERM"]) process.on(signal, async () => { await all.close(); process.exit(0) })
}
