// F5: a relying party with "Continue with Google" that opens the IdP in a POPUP (window.open), and a tiny fake IdP on its own origin.
// Replaces the Gitea dependency of oauth-relying-party.mjs for the card cells. The popup hands the code to its opener with postMessage and closes;
// the relying party exchanges it server side and logs "OAuth login OK". The logs hold no code values.
import { randomBytes } from "node:crypto"
import { page, readBody, startOrigin } from "./common.mjs"

/** rpPublic / idpPublic are the origins the pages will be reached at (the caller fixes the ports); rpNet / idpNet are startOrigin options. */
export async function startOauthPopup({ rpNet = {}, idpNet = {}, rpPublic, idpPublic }) {
  const codes = new Set(), log = [], note = event => log.push({ at: Date.now(), ...event })
  const idp = await startOrigin("oauth-idp", async (req, res, url, reply) => {
    if (url.pathname === "/authorize") {
      note({ kind: "idp_authorize_shown" })
      return reply(200, page("Fake IdP sign-in", `<main><h1 id="idp-title">Sign in to the Fake IdP</h1><p>Continue as test@example.invalid</p><button id="allow">Allow</button></main>
<script>document.getElementById('allow').onclick=async()=>{const r=await fetch('/grant',{method:'POST'});const {code}=await r.json();location.href=${JSON.stringify(rpPublic)}+'/callback?code='+encodeURIComponent(code)+'&state='+encodeURIComponent(${JSON.stringify(url.searchParams.get("state") ?? "")})}</script>`), "text/html")
    }
    if (url.pathname === "/grant" && req.method === "POST") { const code = randomBytes(12).toString("hex"); codes.add(code); note({ kind: "idp_grant" }); return reply(200, { code }) }
    if (url.pathname === "/token" && req.method === "POST") {
      const code = new URLSearchParams(await readBody(req)).get("code")
      if (!codes.delete(code)) return reply(400, { error: "invalid_grant" })
      note({ kind: "idp_token_issued" }); return reply(200, { email: "test@example.invalid" })
    }
    if (url.pathname === "/state") return reply(200, { log })
    return false
  }, idpNet)
  const rp = await startOrigin("oauth-rp", async (req, res, url, reply) => {
    if (url.pathname === "/") {
      return reply(200, page("Relying party", `<main><h1>Example app</h1><p id="status">Signed out</p><button id="google">Continue with Google</button></main>
<script>const state=Math.random().toString(16).slice(2);
document.getElementById('google').onclick=()=>{window.open(${JSON.stringify(idpPublic)}+'/authorize?state='+state,'idp','popup,width=480,height=640')};
addEventListener('message',async e=>{if(e.origin!==location.origin||!e.data||e.data.state!==state)return;
 const r=await fetch('/exchange',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({code:e.data.code})});
 document.getElementById('status').textContent=r.ok?'OAuth login OK':'OAuth login failed'})</script>`), "text/html")
    }
    if (url.pathname === "/callback") {
      return reply(200, page("Returning", `<main><p id="returning">Signing you in...</p></main><script>const p=new URLSearchParams(location.search);
if(window.opener){window.opener.postMessage({code:p.get('code'),state:p.get('state')},location.origin);window.close()}else{document.getElementById('returning').textContent='No opener: the popup lost its opener'}</script>`), "text/html")
    }
    if (url.pathname === "/exchange" && req.method === "POST") {
      const { code } = JSON.parse(await readBody(req))
      const r = await fetch(idpPublic + "/token", { method: "POST", headers: { "content-type": "application/x-www-form-urlencoded" }, body: new URLSearchParams({ code }) })
      note({ kind: r.ok ? "rp_login_ok" : "rp_login_failed", text: r.ok ? "OAuth login OK" : "OAuth login failed" })
      return reply(r.ok ? 200 : 401, { ok: r.ok })
    }
    if (url.pathname === "/state") return reply(200, { log })
    return false
  }, rpNet)
  return { rp, idp, log, close: async () => { await rp.close(); await idp.close() } }
}
