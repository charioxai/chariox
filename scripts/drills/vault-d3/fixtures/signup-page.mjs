// F1: sign-up form (email, password, confirm). Server-side validation; the log holds sha256 prefixes only.
// ?eye=1 adds a show-password toggle that flips the field to type=text (cell D1-2). ?big=1 draws large text for OCR.
import { readBody, esc, page, sha8, startOrigin, writeState } from "./common.mjs"

// stateDir (optional): a validator-owned dir; the password the page RECEIVES is kept there (mode 600) as the needle of the never-seen scan, so the
// validator knows the canary without reading the Vault. The scan roots must not include that dir.
export async function startSignup({ net = {}, stateDir = null } = {}) {
  const accounts = [], log = []
  const origin = await startOrigin("signup", async (req, res, url, reply) => {
    if (url.pathname === "/" && req.method === "GET") {
      const eye = url.searchParams.get("eye") === "1", big = url.searchParams.get("big") === "1"
      return reply(200, page("Sign up", `${big ? `<style>main{max-width:1100px}input{font:28px/1.3 monospace}</style>` : ""}<main><h1>Create account</h1><form id="signup" method="post" action="/signup">
<label for="email">Email</label><input id="email" name="email" type="email" autocomplete="email">
<label for="password">Password</label><input id="password" name="password" type="password" autocomplete="new-password">${eye ? `<button type="button" id="eye" aria-label="Show password">Show password</button>` : ""}
<label for="confirm">Confirm password</label><input id="confirm" name="confirm" type="password" autocomplete="new-password">
<button type="submit">Create account</button></form></main>
<script>${eye ? `document.getElementById('eye').onclick=()=>{const p=document.getElementById('password');p.type=p.type==='password'?'text':'password'}` : ""}</script>`), "text/html")
    }
    if (url.pathname === "/signup" && req.method === "POST") {
      const form = new URLSearchParams(await readBody(req))
      const ok = form.get("password")?.length >= 12 && form.get("password") === form.get("confirm") && /@/.test(form.get("email") ?? "")
      log.push({ at: Date.now(), kind: "signup", ok, password_sha8: sha8(form.get("password") ?? ""), confirm_equal: form.get("password") === form.get("confirm") })
      if (ok && stateDir) writeState(stateDir, "f1-received-password.needle", form.get("password"))
      if (ok) accounts.push({ email_sha8: sha8(form.get("email")), password_sha8: sha8(form.get("password")) })
      return reply(ok ? 200 : 422, page("Sign up", `<main><h1 id="result">${ok ? "Account created" : "Sign-up refused"}</h1><p>${esc(ok ? "created" : "Password must be 12+ characters and match the confirmation.")}</p></main>`), "text/html")
    }
    if (url.pathname === "/state") return reply(200, { accounts, log })
    return false
  }, net)
  return { ...origin, accounts, log }
}
