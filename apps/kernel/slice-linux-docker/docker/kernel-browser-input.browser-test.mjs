// MP-08/MP-10/MP-11: opt-in native Chromium; no mocked DOM or Input dispatch.
import test from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { mkdtemp, rm } from "node:fs/promises";
import path from "node:path";
import { KernelBrowserHost } from "./kernel-browser-host.mjs";

assert(process.env.CHARIOX_MDACCESS_DRILL_ROOT, "MP-10: explicit disposable drill root required");

async function native(markup, operation, instrument = true) {
  const root = await mkdtemp(path.join(process.env.CHARIOX_MDACCESS_DRILL_ROOT, "native-input-"));
  const server = createServer((_request, response) => {
    response.setHeader("Content-Type", "text/html");
    response.end(`<!doctype html>${markup}<p id="result">submitted=0 releases=0</p>
      <script>
      let submitted=0, releases=0;
      const status=()=>document.querySelector('#result').textContent=
        'submitted='+submitted+' releases='+releases+' focus='+document.activeElement.id;
      if (${instrument}) {
        document.addEventListener('submit',e=>{e.preventDefault();submitted++;status()});
        document.addEventListener('keyup',e=>{if(e.key==='Tab')releases++;status()});
      }
      document.querySelector('#field').focus();
      </script>`);
  });
  const host = new KernelBrowserHost(root);
  try {
    await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
    const opened = await host.request({ op: "open", url: `http://127.0.0.1:${server.address().port}/` });
    const tab = opened.tabs[0];
    const bound = { tab_id: tab.tab_id, generation: opened.generation, document_id: tab.document_id };
    const input = (event, authority = {}) => host.request({ op: "input", ...bound, input: event, ...authority });
    await input({ kind: "click", x: 30, y: 25 });
    const { connection, sessionId } = await host.browser.resolvePageTarget(host.tabs.get(tab.tab_id).target_id);
    const events = [];
    const send = connection.send.bind(connection);
    connection.send = async (method, params, session) => {
      if (method.startsWith("Input.")) events.push({ method, type: params.type });
      return send(method, params, session);
    };
    // Read only this credential-free fixture's event counter, never a broad DOM
    // snapshot that could match the counter's initial text in a script node.
    const status = async () => (await connection.send("Runtime.evaluate", {
      expression: "document.querySelector('#result').textContent", returnByValue: true,
    }, sessionId)).result.value;
    const evaluate = async expression => {
      const result = await connection.send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true }, sessionId);
      assert(!result.exceptionDetails, 'MP-10: credential-free fixture setup/read failed');
      return result.result.value;
    };
    await operation({ host, bound, input, status, events, evaluate });
  } finally {
    await host.stop();
    await new Promise(resolve => server.close(resolve));
    await rm(root, { recursive: true, force: true });
  }
}

const field = '<input id="field" type="text" style="height:30px">';
test("MP-08/MP-10/MP-11: retained native keys, editing and activations match focused dispatch on every handler path", () => native(`<div id="root">${field}<textarea id="area"></textarea><div id="edit" contenteditable>text</div><button style="width:140px;height:30px" id="pay" onclick="window.effects++">Approve payment</button><input id="input-button" type="button" value="Pay" onclick="window.effects++"><button style="width:60px;height:30px" id="icon" onclick="window.effects++"></button><div id="handler" style="width:50px;height:30px" onclick="window.effects++">Pay</div></div><div style="height:3000px"></div>`, async ({host,bound,input,events,evaluate}) => {
  const keys=['Tab','Shift+Tab','Enter','Space','Delete','Backspace','ArrowLeft','ArrowRight','ArrowUp','ArrowDown','Home','End','Escape','a'];
  let pairs=0;
  for(const id of ['field','area','edit','pay']) for(const location of ['element','document','window','react-root']) {
    await evaluate(`(() => {
      window.cleanup?.(); const action=document.querySelector('#${id}');
      const target=${JSON.stringify(location)}==='element'?action:${JSON.stringify(location)}==='document'?document:${JSON.stringify(location)}==='window'?window:document.querySelector('#root');
      const effect=()=>window.effects++; const types=['keydown','keyup','beforeinput','input'];
      for(const type of types) target.addEventListener(type,effect);
      window.cleanup=()=>{for(const type of types) target.removeEventListener(type,effect)};
    })()`);
    for(const event of [...keys.map(key=>({kind:'key',key})),{kind:'text',text:'ordinary'}]) {
      const results=[];
      for(const retained of [false,true]) {
        await evaluate(`(() => { window.effects=0; const e=document.querySelector('#${id}'); if('value' in e)e.value='text'; e.focus(); })()`);
        events.length=0;
        await input(event,{_retained_agent:retained});
        results.push({events:[...events],effects:await evaluate('window.effects')});
      }
      assert.deepEqual(results[1],results[0],`${id}/${location}/${event.key??event.kind}`);
      assert(results[1].events.length>0); pairs++;
    }
  }
  await evaluate('window.cleanup()');
  for(const id of ['field','area','edit']) {
    await evaluate(`document.querySelector('#${id}').focus()`);
    await input({kind:'text',text:'retained'}, {_retained_agent:true});
    assert((await evaluate(`document.querySelector('#${id}').value ?? document.querySelector('#${id}').textContent`)).includes('retained'));
  }
  events.length=0;
  await input({kind:'scroll',x:1000,y:500,delta_x:0,delta_y:600},{_retained_agent:true});
  await evaluate('new Promise(resolve=>setTimeout(resolve,500))');
  assert((await evaluate('window.scrollY'))>0); assert.deepEqual(events.map(e=>e.type),['mouseWheel']);
  await host.request({op:'snapshot',...bound});
  console.log(`MP-08/MP-10/MP-11: ${pairs} retained/focused native keyboard pairs plus activation and text parity`);
},false));

test("MP-08/MP-10/MP-11: retained native clicks activate the same controls as focus", () => native(`${field}<button style="width:140px;height:30px" id="pay" onclick="window.effects++">Approve payment</button><input id="input-button" type="button" value="Pay" onclick="window.effects++"><button style="width:60px;height:30px" id="icon" onclick="window.effects++"></button><div id="handler" style="width:50px;height:30px" onclick="window.effects++">Pay</div>`, async ({input,events,evaluate}) => {
  for(const id of ['pay','input-button','icon','handler']) {
    const point=await evaluate(`(() => { const r=document.querySelector('#${id}').getBoundingClientRect(); return {x:Math.round(r.x+r.width/2),y:Math.round(r.y+r.height/2)} })()`);
    for(const retained of [false,true]) {
      await evaluate('window.effects=0'); events.length=0;
      await input({kind:'click',...point},{_retained_agent:retained});
      await evaluate('new Promise(resolve=>setTimeout(resolve,30))');
      assert.equal(await evaluate('window.effects'),1,id);
      assert.deepEqual(events.map(e=>e.type),['mousePressed','mouseReleased']);
    }
  }
},false));

test("MP-08: retained Tab releases on an approval button without activating or stopping Chromium", () => native(`${field}<button id="pay">Approve payment</button>`, async ({host,bound,input,status,events}) => {
  const stream=await host.request({op:'subscribe',...bound});
  await input({kind:'key',key:'Tab'},{_retained_agent:true});
  assert.equal(await status(),'submitted=0 releases=1 focus=pay');
  assert.deepEqual(events.map(e=>e.type),['keyDown','keyUp']);
  assert.equal((await host.request({op:'state'})).generation,bound.generation);
  await host.request({op:'poll',...stream});
}));

test("MP-11: retained/focused state reads cannot restart native Chromium after human stop; explicit start can", () => native(field, async ({host,bound}) => {
  await host.request({op:'stop'});
  for(const retained of [true,false]) {
    await assert.rejects(host.request({op:'state',_retained_agent:retained}),{code:'browser_unavailable'});
    assert.equal(host.chromium.child,null); assert.equal(host.browser,null);
  }
  const restarted=await host.request({op:'start',_retained_agent:true});
  assert.equal(restarted.generation,bound.generation+1); assert.equal(restarted.tabs[0].tab_id,bound.tab_id);
},false));

test("MP-11: retained input still checks revocation between paired native events", () => native(field, async ({input,events,host,bound}) => {
  const cancellation=new AbortController();
  const {connection}=await host.browser.resolvePageTarget(host.tabs.get(bound.tab_id).target_id);
  const send=connection.send.bind(connection);
  connection.send=async(method,params,session)=>{const result=await send(method,params,session); if(method==='Input.dispatchKeyEvent'&&params.type==='keyDown')cancellation.abort(); return result;};
  await assert.rejects(host.request({op:'input',...bound,input:{kind:'key',key:'Tab'},_retained_agent:true},{signal:cancellation.signal}),{code:'browser_action_cancelled'});
  assert.deepEqual(events.map(e=>e.type),['keyDown']);
}));


for (const target of ['password','otp','shadow','frame']) for (const retained of [false,true])
test(`MP-08/MP-10/MP-11: protected ${target} rejects ${retained ? "retained" : "focused"} text-producing keys`, () => native(`${field}<input id="password" type="password"><input id="otp" autocomplete="one-time-code"><div id="shadow"></div><iframe id="frame" srcdoc='<input id="inner">'></iframe>`, async ({host,bound,input,events,evaluate}) => {
  await evaluate(`(() => {
    const shadow=document.querySelector('#shadow').attachShadow({mode:'open'});
    shadow.innerHTML='<input id="secret" type="password">';
    window.effects=0;
    for(const type of ['keydown','beforeinput','input']) document.addEventListener(type,()=>window.effects++);
  })()`);
  {
    await evaluate(target==='shadow' ? "document.querySelector('#shadow').shadowRoot.querySelector('input').focus()" :
      target==='frame' ? "document.querySelector('#frame').contentDocument.querySelector('#inner').focus()" : `document.querySelector('#${target}').focus()`);
    for(const event of [
      {kind:'text',text:'fixture'}, ...['a','é','😀',' ', 'Enter','Space'].map(key=>({kind:'key',key})),
    ]) {
      events.length=0;
      await assert.rejects(input(event,{_retained_agent:retained}),/secret field input requires the Vault path/,`${target}/${retained}/${event.key??event.kind}`);
      assert.equal(events.length,0); assert.equal(await evaluate('window.effects'),0);
      assert.equal((await host.request({op:'state'})).generation,bound.generation);
    }
  }
},false));
