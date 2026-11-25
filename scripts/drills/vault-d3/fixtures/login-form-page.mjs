// F3: login form (text, password, remember-me) -> /welcome, a per-event log with the first-click flag, and a 300x75 box for the fit cell.
// The log holds counts and sha256 prefixes of what was typed, never the characters.
import { documentId, page, readBody, sha8, startOrigin } from "./common.mjs"

const eventScript = `
const events=[];let first=true;
const send=e=>{navigator.sendBeacon('/event',JSON.stringify({...e,t:Date.now(),doc:document.documentElement.dataset.doc}))};
for(const type of ['focusin','focusout','click']) document.addEventListener(type,e=>{const id=e.target&&e.target.id||e.target.tagName;send({type,id,first_click:type==='click'&&first});if(type==='click')first=false},true);
const counts={};document.addEventListener('keydown',e=>{const id=e.target&&e.target.id||'body';counts[id]=(counts[id]||0)+1},true);
addEventListener('pagehide',()=>send({type:'keycounts',counts}));setInterval(()=>send({type:'keycounts',counts}),1000);`

export async function startLoginForm({ net = {}, welcomeOrigin = "" } = {}) {
  const events = [], submissions = []
  const origin = await startOrigin("login-form", async (req, res, url, reply) => {
    const doc = documentId()
    const shell = (title, body) => page(title, body).replace("<html ", `<html data-doc="${doc}" `)
    if (url.pathname === "/" && req.method === "GET") return reply(200, shell("Sign in", `<main><h1>Sign in</h1><form id="login" method="post" action="/login">
<label for="user">Username</label><input id="user" name="user" type="text" autocomplete="username">
<label for="pass">Password</label><input id="pass" name="pass" type="password" autocomplete="current-password">
<label><input id="remember" name="remember" type="checkbox" style="width:auto"> Remember me</label>
<button id="go" type="submit">Sign in</button></form></main><script>${eventScript}</script>`), "text/html")
    if (url.pathname === "/login" && req.method === "POST") {
      const form = new URLSearchParams(await readBody(req))
      submissions.push({ at: Date.now(), user_sha8: sha8(form.get("user") ?? ""), pass_sha8: sha8(form.get("pass") ?? ""), pass_len: (form.get("pass") ?? "").length, remember: form.get("remember") === "on" })
      res.writeHead(303, { location: welcomeOrigin + "/welcome" }); return res.end()
    }
    if (url.pathname === "/welcome") return reply(200, shell("Welcome", `<main><h1 id="welcome">Welcome</h1><p>You are signed in. Type below to prove the page still takes input.</p><label for="after">Notes</label><input id="after" type="text"></main><script>${eventScript}</script>`), "text/html")
    if (url.pathname === "/small") return reply(200, shell("Small box", `<div id="small" style="width:300px;height:75px;background:#def;border:2px solid #36c;margin:40px auto;display:flex;align-items:center;justify-content:center">300 x 75 box</div>`), "text/html")
    if (url.pathname === "/event" && req.method === "POST") { try { events.push(JSON.parse(await readBody(req))) } catch {} ; res.writeHead(204); return res.end() }
    if (url.pathname === "/state") return reply(200, { submissions, events })
    return false
  }, net)
  return { ...origin, events, submissions }
}
