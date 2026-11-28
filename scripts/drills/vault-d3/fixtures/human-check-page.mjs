// F4: the human-check cases. (c) is the one that matters for row 547: our own CROSS-ORIGIN iframe with a canvas "pick the tiles" grid,
// because the vendor test sitekeys do not render the hCaptcha image grid. (a) hCaptcha / (b) Turnstile test widgets need the vendors' scripts
// (outside network) and are served only with ?vendor=1; verify the test sitekeys against the vendors' current docs before relying on them.
import { page, readBody, startOrigin } from "./common.mjs"

const HCAPTCHA_TEST_SITEKEY = "10000000-ffff-ffff-ffff-000000000001"
const TURNSTILE_TEST_SITEKEYS = { passes: "1x00000000000000000000AA", interactive: "3x00000000000000000000FF" }

/** parentNet / gridNet fix the two origins (they must differ: the iframe is cross-origin). */
export async function startHumanCheck({ parentNet = {}, gridNet = {}, gridPublic }) {
  const circles = new Set(); while (circles.size < 3) circles.add(Math.floor(Math.random() * 9))
  const log = []
  const grid = await startOrigin("grid", async (req, res, url, reply) => {
    if (url.pathname === "/grid.html") {
      return reply(200, page("Pick the tiles", `<main style="max-width:360px"><p id="prompt">Click all the tiles with a circle</p><canvas id="grid" width="300" height="300" style="width:300px;height:300px;border:1px solid #666"></canvas><button id="verify">Verify</button><p id="result"></p></main>
<script>const circles=${JSON.stringify([...circles])},picked=new Set(),c=document.getElementById('grid'),g=c.getContext('2d');
function draw(){g.fillStyle='#fff';g.fillRect(0,0,300,300);for(let i=0;i<9;i++){const x=(i%3)*100,y=Math.floor(i/3)*100;g.fillStyle=['#fde','#def','#efd'][i%3];g.fillRect(x+2,y+2,96,96);g.fillStyle=i%2?'#c33':'#33c';g.beginPath();
 if(circles.includes(i))g.arc(x+50,y+50,30,0,7);else g.rect(x+22,y+22,56,56);g.fill();if(picked.has(i)){g.strokeStyle='#000';g.lineWidth=5;g.strokeRect(x+4,y+4,92,92)}}}
c.onclick=e=>{const r=c.getBoundingClientRect(),i=Math.floor((e.clientY-r.top)/100)*3+Math.floor((e.clientX-r.left)/100);picked.has(i)?picked.delete(i):picked.add(i);draw()};
document.getElementById('verify').onclick=async()=>{const r=await fetch('/verify',{method:'POST',body:JSON.stringify({picked:[...picked]})});const j=await r.json();document.getElementById('result').textContent=j.ok?'Verified':'Try again';parent.postMessage({verified:j.ok},'*')};
draw();</script>`), "text/html")
    }
    if (url.pathname === "/verify" && req.method === "POST") {
      const { picked = [] } = JSON.parse(await readBody(req))
      const ok = picked.length === circles.size && picked.every(i => circles.has(i))
      log.push({ at: Date.now(), kind: "grid_verify", ok, picked: picked.length }); return reply(200, { ok })
    }
    if (url.pathname === "/state") return reply(200, { log })
    return false
  }, gridNet)
  const parent = await startOrigin("human-check", async (req, res, url, reply) => {
    if (url.pathname === "/") {
      const vendor = url.searchParams.get("vendor") === "1"
      return reply(200, page("Human check", `<main><h1>Are you human?</h1><iframe id="grid-frame" src="${gridPublic}/grid.html" style="width:340px;height:430px;border:0" title="tile challenge"></iframe><p id="outcome">Waiting</p>
${vendor ? `<div class="h-captcha" data-sitekey="${HCAPTCHA_TEST_SITEKEY}"></div><div class="cf-turnstile" data-sitekey="${TURNSTILE_TEST_SITEKEYS.interactive}"></div>` : ""}</main>
<script>addEventListener('message',e=>{if(e.origin===${JSON.stringify(gridPublic)}&&e.data&&e.data.verified)document.getElementById('outcome').textContent='Human check passed'})</script>
${vendor ? `<script src="https://js.hcaptcha.com/1/api.js" async defer></script><script src="https://challenges.cloudflare.com/turnstile/v0/api.js" async defer></script>` : ""}`), "text/html")
    }
    if (url.pathname === "/state") return reply(200, { circles: circles.size, log })
    return false
  }, parentNet)
  return { parent, grid, log, close: async () => { await parent.close(); await grid.close() } }
}
