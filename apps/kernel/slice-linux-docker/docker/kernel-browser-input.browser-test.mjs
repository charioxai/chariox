// MP-08/MP-10/MP-11: opt-in native Chromium; no mocked DOM or Input dispatch.
import test from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { mkdtemp, rm, mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { KernelBrowserHost } from "./kernel-browser-host.mjs";
import { inputHostTab } from "./kernel-browser-input.mjs";

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
    const { connection, sessionId } = await host.browser.resolvePageTarget(host.tabs.get(tab.tab_id).target_id);
    let ready=false;
    for(let tries=0;tries<200&&!ready;tries++) {
      try {ready=(await connection.send('Runtime.evaluate',{
        expression:"document.readyState==='complete'&&Boolean(document.querySelector('#field'))",returnByValue:true,
      },sessionId)).result?.value===true;}
      catch(error){if(error.code!=='browser_cdp_command_failed'||!error.message.includes('context was destroyed'))throw error;}
      if(!ready)await new Promise(resolve=>setTimeout(resolve,25));
    }
    assert(ready,'MP-11: native fixture must finish loading before input');
    const state=await host.request({op:'state'});
    bound.document_id=state.tabs.find(t=>t.tab_id===tab.tab_id).document_id;
    await input({ kind: "click", x: 30, y: 25 });
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
    await operation({ host, bound, input, status, events, evaluate, connection });
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
      await assert.rejects(input(event,{_retained_agent:retained}),{code:'user_domain_sensitive_requires_focus'},`${target}/${retained}/${event.key??event.kind}`);
      assert.equal(events.length,0); assert.equal(await evaluate('window.effects'),0);
      assert.equal((await host.request({op:'state'})).generation,bound.generation);
    }
  }
},false));

// MP-11: use actual controller, isolated observer and physical Chromium dispatch.
// This is supplementary security regression evidence, not hosted-client acceptance.
for (const dpr of [1, 2]) test(`MP-11: plain mirror keys require live painted focus at DPR ${dpr}`, () => native(
  `${field}<div id="ancestor"><input id="other" value="ordinary"></div><button id="pay" onclick="window.effects++">Public action</button>`,
  async ({host,bound,input,events,evaluate,connection}) => {
    const sub=await host.request({op:'mirror_subscribe',...bound,device_scale_factor:dpr});
    const rows=[];let sequence=0;
    for(const scenario of ['observed-focus-move','unobserved-focus-move','protected-field','password-field','otp-field','protected-ancestor','protected-shadow-host','changed-geometry','focus-during-preflight']) {
      for(const key of (scenario==='focus-during-preflight'?['Enter']:['Enter','Backspace','Delete'])) {
        await evaluate(`(() => {
          document.querySelector('#late')?.remove(); document.querySelector('#shadow-host')?.remove();
          const field=document.querySelector('#field'),other=document.querySelector('#other'),ancestor=document.querySelector('#ancestor');
          field.type='text';field.removeAttribute('autocomplete');field.style.marginLeft='';field.removeAttribute('data-observation-protected');ancestor.removeAttribute('data-observation-protected');
          field.value='ordinary';other.value='ordinary';window.effects=0;field.focus();
        })()`);
        const packet=await host.request({op:'mirror_next',subscription_id:sub.subscription_id,generation:bound.generation,after_sequence:sequence,drift_nodes:[]});sequence=packet.sequence;
        assert(packet.focused,'MP-11: fixture focus must be painted');
        await evaluate(`(() => {
          if(${JSON.stringify(scenario)}==='observed-focus-move')document.querySelector(${JSON.stringify(key==='Enter'?'#pay':'#other')}).focus();
          if(${JSON.stringify(scenario)}==='unobserved-focus-move'){const e=document.createElement('input');e.id='late';e.value='ordinary';document.body.append(e);e.focus();}
          if(${JSON.stringify(scenario)}==='protected-field')document.querySelector('#field').setAttribute('data-observation-protected','');
          if(${JSON.stringify(scenario)}==='password-field')document.querySelector('#field').type='password';
          if(${JSON.stringify(scenario)}==='otp-field')document.querySelector('#field').setAttribute('autocomplete','one-time-code');
          if(${JSON.stringify(scenario)}==='protected-ancestor'){document.querySelector('#ancestor').setAttribute('data-observation-protected','');document.querySelector('#other').focus();}
          if(${JSON.stringify(scenario)}==='protected-shadow-host'){const e=document.createElement('div');e.id='shadow-host';e.setAttribute('data-observation-protected','');document.body.append(e);const field=document.createElement('input');field.value='ordinary';e.attachShadow({mode:'open'}).append(field);field.focus();}
          if(${JSON.stringify(scenario)}==='changed-geometry')document.querySelector('#field').style.marginLeft='50px';
        })()`);
        // Deliberately do not ask for a new mirror packet between focus change/key.
        events.length=0;let refused=false;
        const send=connection.send.bind(connection);
        if(scenario==='focus-during-preflight') connection.send=async(method,params,session)=>{
          const reply=await send(method,params,session);
          if(method==='Runtime.evaluate'&&params.expression.includes('let e = document.activeElement'))
            await send('Runtime.evaluate',{expression:"document.querySelector('#pay').focus()",returnByValue:true},session);
          return reply;
        };
        try {await input({kind:'mirror',subscription_id:sub.subscription_id,sequence,action:{kind:'key',key}});} catch {refused=true;} finally {connection.send=send;}
        const effects=await evaluate('window.effects');
        rows.push({scenario,key,dpr,sequence,refused,dispatches:events.length,effects,pass:refused&&events.length===0&&effects===0});
      }
    }
    // Tab moves native focus during keyDown: the release must remain paired.
    await evaluate("document.querySelector('#field').style.marginLeft='';document.querySelector('#field').removeAttribute('data-observation-protected');document.querySelector('#field').focus()");
    const packet=await host.request({op:'mirror_next',subscription_id:sub.subscription_id,generation:bound.generation,after_sequence:sequence,drift_nodes:[]});
    events.length=0;
    await input({kind:'mirror',subscription_id:sub.subscription_id,sequence:packet.sequence,action:{kind:'key',key:'Tab'}});
    assert.deepEqual(events.map(event=>event.type),['keyDown','keyUp']);
    if(process.env.CHARIOX_MIRROR_KEY_EVIDENCE) {
      const evidence=process.env.CHARIOX_MIRROR_KEY_EVIDENCE;await mkdir(evidence,{recursive:true});
      const screenshot=await host.request({op:'screenshot',...bound});
      await writeFile(path.join(evidence,`plain-key-dpr${dpr}.png`),Buffer.from(screenshot.data_base64,'base64'));
      await writeFile(path.join(evidence,`plain-key-dpr${dpr}.json`),JSON.stringify({mp:['MP-08','MP-10','MP-11'],scope:'Real sandboxed host Chromium/controller fixture; no hosted client/provider acceptance',rows,pairedRelease:events.map(event=>event.type)},null,2)+'\n');
    }
    assert(rows.every(row=>row.pass),JSON.stringify(rows.filter(row=>!row.pass)));
  }
));

// MP-11: mutate the real target during awaited input preparation, without
// another mirror packet. These are security regressions, not hosted acceptance.
for (const dpr of [1, 2]) test(`MP-11: addressed mirror input fences live dispatch at DPR ${dpr}`, () => native(
  `<div id="ancestor">${field}</div><input id="other"><div id="click-ancestor"><button id="target" title="click-target" style="width:180px;height:40px" onclick="window.effects++"><span>Public action</span></button></div>`,
  async ({host,bound,input,events,evaluate,connection}) => {
    const sub=await host.request({op:'mirror_subscribe',...bound,device_scale_factor:dpr});
    const rows=[];let sequence=0;
    // Focus itself marks a host mutation; a later refusal may stop the host.
    // Exercise the exact native dispatch path while retaining this browser to
    // measure zero physical events and inspect every race in the same document.
    const dispatch=event=>inputHostTab(host.browser,host.tabs.get(bound.tab_id),event,{
      resolveMirror:inner=>host.mirror.resolveInput(host.tabs.get(bound.tab_id),inner,'adapter'),
    });
    const reset=async()=>evaluate(`(() => {
      document.querySelector('#overlay')?.remove();
      document.querySelector('#ancestor').removeAttribute('data-observation-protected');
      document.querySelector('#ancestor').innerHTML=${JSON.stringify(field)};
      document.querySelector('#click-ancestor').removeAttribute('data-observation-protected');
      document.querySelector('#click-ancestor').innerHTML='<button id="target" title="click-target" style="width:180px;height:40px" onclick="window.effects++"><span>Public action</span></button>';
      window.effects=0;window.inputEffects=0;
      document.querySelector('#field').focus();
      document.oninput=()=>window.inputEffects++;
    })()`);
    const packet=async()=>{const p=await host.request({op:'mirror_next',subscription_id:sub.subscription_id,generation:bound.generation,after_sequence:sequence,drift_nodes:[]});sequence=p.sequence;return p;};
    for(const kind of ['text','composition','click']) for(const scenario of kind==='click'
      ? ['replacement','move','overlay','fractional-overlay','protected-target','protected-ancestor','protected-hit']
      : ['focus-swap','replacement','protected-target','protected-ancestor','password','final-preflight-protection']) {
      console.log(`MP-11: addressed DPR${dpr} ${kind}/${scenario}`);
      await reset();const p=await packet();
      const node_id=kind==='click'?p.nodes.find(n=>n.attributes?.title==='click-target').id:p.focused;
      const mutation=`(() => {
        const e=document.querySelector(${JSON.stringify(kind==='click'?'#target':'#field')});
        const scenario=${JSON.stringify(scenario)};
        if(scenario==='focus-swap')document.querySelector('#other').focus();
        if(scenario==='replacement'){const clone=e.cloneNode(true);e.replaceWith(clone);if(${kind!=='click'})clone.focus();}
        if(scenario==='move')e.style.marginLeft='200px';
        if(scenario==='overlay'){const r=e.getBoundingClientRect(),overlay=document.createElement('button');overlay.id='overlay';overlay.style.cssText='position:fixed;z-index:9999;left:'+r.x+'px;top:'+r.y+'px;width:'+r.width+'px;height:'+r.height+'px';overlay.onclick=()=>window.effects++;document.body.append(overlay);}
        if(scenario==='fractional-overlay'){const r=e.getBoundingClientRect(),overlay=document.createElement('button');overlay.id='overlay';overlay.style.cssText='position:fixed;padding:0;border:0;z-index:9999;left:'+Math.floor(r.x+r.width/2)+'px;top:'+Math.floor(r.y+r.height/2)+'px;width:0.25px;height:0.25px';overlay.onclick=()=>window.effects++;document.body.append(overlay);}
        if(scenario==='protected-target'||scenario==='final-preflight-protection')e.setAttribute('data-observation-protected','');
        if(scenario==='protected-ancestor')e.parentElement.setAttribute('data-observation-protected','');
        if(scenario==='protected-hit')e.firstElementChild.setAttribute('data-observation-protected','');
        if(scenario==='password')e.type='password';
      })()`;
      const send=connection.send.bind(connection),run=host.browser.inputCapture.run.bind(host.browser.inputCapture);
      let armed=false,preflight=false,mutated=false,refused=false,error;
      host.browser.inputCapture.run=async(...args)=>{if(kind==='click')armed=true;return run(...args);};
      connection.send=async(method,params,session)=>{
        const reply=await send(method,params,session);
        if(method==='Runtime.evaluate'&&params.expression.includes('.__charioxMirror.focus('))armed=true;
        if(armed&&method==='Runtime.evaluate'&&params.expression.includes('let e = document.activeElement'))preflight=true;
        if(armed&&!mutated&&method==='Page.getFrameTree'&&(scenario!=='final-preflight-protection'||preflight)) {
          const result=await send('Runtime.evaluate',{expression:mutation,returnByValue:true},session);
          assert(!result.exceptionDetails);mutated=true;
        }
        return reply;
      };
      events.length=0;
      const action={kind,node_id,...(kind==='click'?{}:{text:'文',...(kind==='composition'?{selection_start:1,selection_end:1}:{})})};
      try{await dispatch({kind:'mirror',subscription_id:sub.subscription_id,sequence,action});}
      catch(e){refused=true;error=e.message;}
      finally{connection.send=send;host.browser.inputCapture.run=run;}
      const effects=await evaluate('window.effects+window.inputEffects');
      rows.push({kind,scenario,dpr,sequence,mutated,preflight,refused,error,dispatches:events.length,effects,pass:mutated&&refused&&events.length===0&&effects===0});
      if(process.env.CHARIOX_MIRROR_DISPATCH_EVIDENCE) {
        const screenshot=await host.request({op:'screenshot',...bound});
        await writeFile(path.join(process.env.CHARIOX_MIRROR_DISPATCH_EVIDENCE,`addressed-dpr${dpr}-${kind}-${scenario}.png`),Buffer.from(screenshot.data_base64,'base64'));
      }
    }
    const positive=[];
    for(const kind of ['text','composition','click']) {
      await reset();const p=await packet();events.length=0;
      const node_id=kind==='click'?p.nodes.find(n=>n.attributes?.title==='click-target').id:p.focused;
      // A press can move/remove its target; mouseReleased must remain paired.
      if(kind==='click')await evaluate("document.querySelector('#target').onmousedown=()=>{window.effects++;document.querySelector('#target').remove()}");
      await dispatch({kind:'mirror',subscription_id:sub.subscription_id,sequence,action:{kind,node_id,...(kind==='click'?{}:{text:'文',...(kind==='composition'?{selection_start:1,selection_end:1}:{})})}});
      const value=await evaluate("document.querySelector('#field').value");
      const physical=events.map(e=>e.type??e.method);
      positive.push({kind,physical,value,pass:kind==='click'?physical.join(',')==='mousePressed,mouseReleased':value.includes('文')});
    }
    if(process.env.CHARIOX_MIRROR_DISPATCH_EVIDENCE) {
      const evidence=process.env.CHARIOX_MIRROR_DISPATCH_EVIDENCE;await mkdir(evidence,{recursive:true});
      await reset();await packet();
      const screenshot=await host.request({op:'screenshot',...bound});
      await writeFile(path.join(evidence,`addressed-dpr${dpr}.png`),Buffer.from(screenshot.data_base64,'base64'));
      await writeFile(path.join(evidence,`addressed-dpr${dpr}.json`),JSON.stringify({mp:['MP-08','MP-10','MP-11'],scope:'Real sandboxed Chromium/controller security regressions; not hosted client acceptance',rows,positive},null,2)+'\n');
    }
    assert(rows.every(row=>row.pass),JSON.stringify(rows.filter(row=>!row.pass)));
    assert(positive.every(row=>row.pass),JSON.stringify(positive));
  },false
));
