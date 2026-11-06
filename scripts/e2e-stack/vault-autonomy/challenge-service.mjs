// MP-08 / MP-09 / MP-10 / MP-11: owned rehearsal only; no mail or public-service requests.
import {createServer} from 'node:http'
import {randomUUID, randomBytes, timingSafeEqual} from 'node:crypto'

export async function startChallengeService() {
  const challenges = new Map()
  const record = (challenge, event) => challenge.events.push({at: Date.now(), ...event})
  const server = createServer(async (req, res) => {
    const url = new URL(req.url, 'http://127.0.0.1')
    const reply = (status, value) => { res.writeHead(status, {'content-type':'application/json','cache-control':'no-store'}); res.end(JSON.stringify(value)) }
    const match = /^\/challenge\/([a-f0-9-]+)(?:\/(issue|verify|status|me))?$/.exec(url.pathname)
    const challenge = match && challenges.get(match[1])
    if (!challenge) return reply(404, {error:'Unknown challenge'})
    const action = match[2]
    if (req.method === 'GET' && action === 'status') return reply(200, {id:challenge.id, issued:challenge.issued, accepted:challenge.accepted, authenticated_calls:challenge.authenticatedCalls, events:challenge.events})
    if (req.method === 'POST' && action === 'issue') {
      if (challenge.issued) return reply(409, {error:'A challenge was already requested'})
      challenge.issued = true; record(challenge, {kind:'challenge_issued'})
      return reply(200, {issued:true})
    }
    if (req.method === 'POST' && action === 'verify') {
      let body = ''; for await (const chunk of req) {body += chunk; if (body.length > 1024) {req.destroy(); return}}
      let code; try {code = req.headers['content-type']?.split(';')[0] === 'application/x-www-form-urlencoded' ? new URLSearchParams(body).get('code') : JSON.parse(body).code} catch {return reply(400, {error:'Invalid request'})}
      record(challenge, {kind:'input_delivered', nonempty:typeof code === 'string' && code.length > 0})
      if (!challenge.issued || code !== '246810') {
        record(challenge, {kind:'verification', status:422, result:'rejected'})
        return reply(422, {accepted:false, error:'That code was not accepted. Try again.'})
      }
      challenge.accepted = true
      record(challenge, {kind:'verification', status:200, result:'accepted'})
      // The value is delivered only to the owned page, then captured through Vault.
      return reply(200, {accepted:true, token:challenge.token})
    }
    if (req.method === 'GET' && action === 'me') {
      const actual = Buffer.from(req.headers.authorization ?? ''), expected = Buffer.from('Bearer '+challenge.token)
      if (!challenge.accepted || actual.length !== expected.length || !timingSafeEqual(actual, expected)) return reply(401, {authenticated:false})
      challenge.authenticatedCalls++; record(challenge,{kind:'authenticated_call',status:200})
      return reply(200, {authenticated:true, scope:'challenge:'+challenge.id})
    }
    if (req.method !== 'GET' || action) return reply(405, {error:'Method not allowed'})
    res.writeHead(200, {'content-type':'text/html; charset=utf-8','cache-control':'no-store'})
    res.end(`<!doctype html><meta name="viewport" content="width=device-width"><title>Owned verification rehearsal</title>
<style>body{font:18px system-ui;margin:0;background:#f5f3f0}main{box-sizing:border-box;max-width:520px;padding:24px;margin:16px auto;background:white;border:1px solid #ccc;border-radius:8px}input,button{font:inherit;box-sizing:border-box;padding:10px;width:100%;margin:8px 0}p{line-height:1.4}[role=alert]{color:#a10}svg{display:none}</style>
<svg aria-hidden="true">${'<path d="M0 0"/>'.repeat(6000)}</svg>
<main id="challenge"><h1>Verification rehearsal</h1><p>This page emails nothing. Request one test code, then enter it here.</p><button id="issue">Send Code</button><form hidden id="verify"><label for="code">Test code</label>${challenge.segmented ? Array.from({length:6},(_,i)=>'<input id="code'+i+'" aria-label="Code digit '+(i+1)+'" maxlength="1" inputmode="numeric" style="width:60px">').join('') : '<input id="code" autocomplete="one-time-code" inputmode="numeric">'}<button>Verify</button><p role="alert"></p></form><section hidden id="accepted"><p>Verification accepted.</p><label for="token">Scoped API token</label><input id="token" type="password" readonly></section></main>
<script>
const base=${JSON.stringify(url.pathname)}, template=document.querySelector('#challenge').outerHTML;
function bind(issued=false,error='') {
 const form=document.querySelector('#verify'),issue=document.querySelector('#issue');
 issue.hidden=issued;form.hidden=!issued;form.querySelector('[role=alert]').textContent=error;
 issue.onclick=async()=>{const r=await fetch(base+'/issue',{method:'POST'});if(!r.ok)return;issue.hidden=true;form.hidden=false;form.querySelector('input').focus()};
 form.onsubmit=async e=>{e.preventDefault();const r=await fetch(base+'/verify',{method:'POST',headers:{'content-type':${JSON.stringify(challenge.requestEncoding === 'urlencoded' ? 'application/x-www-form-urlencoded;charset=UTF-8' : 'application/json')}},body:${challenge.requestEncoding === 'urlencoded' ? "new URLSearchParams({code:[...form.querySelectorAll('input')].map(input=>input.value).join('')})" : "JSON.stringify({code:[...form.querySelectorAll('input')].map(input=>input.value).join('')})"}});const result=await r.json();if(!result.accepted){document.querySelector('#challenge').outerHTML=template;document.querySelector('#challenge').style.marginTop='80px';bind(true,result.error);return}form.hidden=true;document.querySelector('#accepted').hidden=false;document.querySelector('#token').value=result.token};
}
bind();
</script>`)
  })
  await new Promise(resolve => server.listen(0,'127.0.0.1',resolve))
  const origin = 'http://127.0.0.1:'+server.address().port
  return {
    create({requestEncoding='json',segmented=false} = {}) {const id=randomUUID();challenges.set(id,{id,requestEncoding,segmented,issued:false,accepted:false,authenticatedCalls:0,token:randomBytes(32).toString('hex'),events:[]});return {id,url:origin+'/challenge/'+id}},
    async close() {server.closeAllConnections();await new Promise(resolve=>server.close(resolve))},
  }
}
