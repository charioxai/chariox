// F2: one page, three blocks that hold a known token: Reveal then text node, Copy to clipboard, and a QR of a TOTP-style seed.
// /verify takes the captured value as Bearer and compares it with what it issued; the log holds sha256 prefixes only.
// ?swap=1 swaps the revealed node 2 s after Reveal (cell D2-5b). ?big=1 draws the text large for OCR.
import { createRequire } from "node:module"
import { randomBytes, timingSafeEqual } from "node:crypto"
import { esc, newToken, page, startOrigin, writeState } from "./common.mjs"
import { sha8 } from "./common.mjs"

function qrSvg(text) {
  const from = process.env.QR_FROM // a node_modules path that holds qrcode-terminal (vendored QRCode, no network)
  if (!from) return null
  const require = createRequire(from.endsWith("/") ? from + "x.js" : from + "/x.js")
  const QRCode = require("qrcode-terminal/vendor/QRCode"), level = require("qrcode-terminal/vendor/QRCode/QRErrorCorrectLevel")
  const qr = new QRCode(-1, level.M); qr.addData(text); qr.make()
  const n = qr.getModuleCount(), cell = 8, quiet = 4
  let rects = ""
  for (let r = 0; r < n; r++) for (let c = 0; c < n; c++) if (qr.isDark(r, c)) rects += `<rect x="${(c + quiet) * cell}" y="${(r + quiet) * cell}" width="${cell}" height="${cell}"/>`
  const size = (n + quiet * 2) * cell
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 ${size} ${size}"><rect width="100%" height="100%" fill="#fff"/><g fill="#000">${rects}</g></svg>`
}

export async function startRevealCopyQr({ stateDir, net = {} } = {}) {
  const base32 = n => Array.from(randomBytes(n), b => "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567"[b % 32]).join("")
  const tokens = { reveal: newToken("rev"), copy: newToken("cpy"), qr: base32(26) }
  const log = []
  const note = event => log.push({ at: Date.now(), ...event })
  // A QR capture returns what the code encodes: the whole otpauth URI. /verify?kind=qr accepts that payload (it contains the seed).
  const qrPayload = `otpauth://totp/Fixture:owned?secret=${tokens.qr}&issuer=Fixture`
  const svg = qrSvg(qrPayload)
  const origin = await startOrigin("reveal-copy-qr", async (req, res, url, reply) => {
    if (url.pathname === "/" && req.method === "GET") {
      const big = url.searchParams.get("big") === "1" ? " big" : ""
      const swap = url.searchParams.get("swap") === "1"
      return reply(200, page("Token shown once", `<main>
<h1>Token created</h1><p>This token is shown once. Reveal it, copy it, or scan the QR.</p>
<section id="block-reveal"><h2>Reveal</h2><span id="token-box" data-hidden="1" class="${big.trim()}" aria-label="API token (hidden)">••••••••••••••••</span><button id="reveal">Reveal token</button></section>
<section id="block-copy"><h2>Copy</h2><button id="copy">Copy token</button><p id="copied" hidden>Copied</p></section>
<section id="block-qr"><h2>QR</h2>${svg ? `<div id="qr">${svg}</div>` : `<p id="qr-missing">QR not available (set QR_FROM)</p>`}</section>
</main><script>
const shown=async kind=>(await (await fetch('/show?kind='+kind,{method:'POST'})).json()).value;
document.getElementById('reveal').onclick=async()=>{const revealValue=await shown('reveal');const box=document.getElementById('token-box');box.textContent=revealValue;box.dataset.hidden='0';box.setAttribute('aria-label','API token');
 ${swap ? `setTimeout(()=>{const n=document.createElement('span');n.id='token-box';n.className=box.className;n.textContent='x'.repeat(8)+'-swapped';box.replaceWith(n)},2000)` : ""}};
document.getElementById('copy').onclick=async()=>{await navigator.clipboard.writeText(await shown('copy'));document.getElementById('copied').hidden=false};
</script>`), "text/html")
    }
    if (url.pathname === "/verify" && req.method === "POST") {
      const kind = url.searchParams.get("kind")
      const wanted = kind === "qr" ? qrPayload : tokens[kind]
      const expected = Buffer.from(`Bearer ${wanted ?? "\u0000"}`), actual = Buffer.from(req.headers.authorization ?? "")
      const ok = wanted && actual.length === expected.length && timingSafeEqual(actual, expected)
      note({ kind: "verify", block: kind, ok: !!ok, presented_sha8: sha8((req.headers.authorization ?? "").replace(/^Bearer /, "")) })
      return reply(ok ? 200 : 401, { ok: !!ok })
    }
    if (url.pathname === "/show" && req.method === "POST") { const kind = url.searchParams.get("kind"); if (!["reveal", "copy"].includes(kind)) return reply(400, {}); note({ kind: "shown", block: kind }); return reply(200, { value: tokens[kind] }) }
    if (url.pathname === "/state") return reply(200, { log, issued_sha8: Object.fromEntries(Object.entries(tokens).map(([k, v]) => [k, sha8(v)])) })
    return false
  }, net)
  const state = stateDir ? { needle_files: { reveal: writeState(stateDir, "f2-reveal.needle", tokens.reveal), copy: writeState(stateDir, "f2-copy.needle", tokens.copy), qr: writeState(stateDir, "f2-qr.needle", tokens.qr) } } : null
  return { ...origin, state, log }
}
