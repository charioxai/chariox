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
      const result = await connection.send("Runtime.evaluate", { expression, returnByValue: true }, sessionId);
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
for (const [name, markup] of [
  ["default payment", `<form>${field}<button type="submit">Pay</button></form>`],
  ["external form owner", `${field.replace('id="field"', 'id="field" form="payment"')}<form id="payment"></form><button form="payment">Pay</button>`],
  ["image submit", `<form>${field}<input type="image" alt="Pay"></form>`],
  ["unlabelled submit", `<form>${field}<button type="submit"></button></form>`],
  ["unclassified buttonless submit", `<form>${field}</form>`],
]) {
  test(`MP-11 P1: retained Enter blocks ${name} before native dispatch`, () => native(markup, async ({ input, status, events }) => {
    const result = await input({ kind: "key", key: "Enter" }, { _retained_agent: true }).then(() => null, error => error);
    assert.match(await status(), /^submitted=0 releases=0/);
    assert.match(result?.message ?? "allowed", /requires focus/);
    assert.deepEqual(events, [], "MP-11: denied activation must dispatch no native input");
  }));
}

test("MP-11: retained native activation matrix fails closed; focused controls remain usable", () => native(`${field}<div id="stage"></div>`, async ({ host, bound, input, events, evaluate }) => {
  const controls = [
    ['button', '<button id="action">LABEL</button>'],
    ...['button', 'submit', 'image', 'reset'].map(type => [`input-${type}`, `<input id="action" type="${type}" value="LABEL" alt="LABEL">`]),
    ['link', '<a id="action" href="#">LABEL</a>'],
    ...['button', 'link', 'menuitem', 'tab', 'switch', 'checkbox', 'option'].map(role => [`role-${role}`, `<div id="action" role="${role}" tabindex="0">LABEL</div>`]),
    ['label', '<label id="action" for="target">LABEL</label><input id="target" type="checkbox" aria-label="LABEL">'],
    ['summary', '<details><summary id="action">LABEL</summary></details>'],
    ['handler-only', '<div id="action" tabindex="0">LABEL</div>'],
    ...['click', 'pointerdown', 'keydown', 'keyup'].map(type => [`handler-${type}`, '<div id="action" tabindex="0">LABEL</div>']),
    ['ancestor-handler', '<div id="listener"><div id="action" tabindex="0">LABEL</div></div>'],
    ['document-handler', '<div id="action" tabindex="0">LABEL</div>'],
    ['window-handler', '<div id="action" tabindex="0">LABEL</div>'],
    ['unnamed-icon-button', '<button id="action" style="width:40px;height:30px;background:black"></button>'],
    ['unclassified-button', '<button id="action">Continue</button>'],
    ['routine-label-sensitive-target', '<label id="action" for="target">Search</label><input id="target" type="checkbox" aria-label="LABEL">'],
    ['implicit-submit', '<form><input id="action"><button>LABEL</button></form>'],
    ['external-submit', '<input id="action" form="payment"><form id="payment"></form><button form="payment">LABEL</button>'],
  ];
  const failures = [];
  let refusals = 0, focused = 0;
  for (const [kind, template] of controls) {
    for (const label of ['Pay', 'Approve']) {
      const configure = async () => evaluate(`(() => {
        window.cleanup?.();
        document.querySelector('#stage').innerHTML = ${JSON.stringify(template.replaceAll('LABEL', label))};
        window.effects = 0;
        const action = document.querySelector('#action');
        const listener = ${JSON.stringify(kind)} === 'document-handler' ? document :
          ${JSON.stringify(kind)} === 'window-handler' ? window : document.querySelector('#listener') || action;
        const effect = event => { event.preventDefault(); window.effects++; };
        const types = ${JSON.stringify(['implicit-submit', 'external-submit'].includes(kind) ? ['submit'] : kind.startsWith('handler-') && kind !== 'handler-only' ? [kind.slice('handler-'.length)] : ['click', 'pointerdown', 'keydown', 'keyup'])};
        const target = types[0] === 'submit' ? action.form : listener;
        for (const type of types) target.addEventListener(type, effect);
        window.cleanup = () => { for (const type of types) target.removeEventListener(type, effect); };
        action.tabIndex = 0; action.focus();
        const r = action.getBoundingClientRect();
        return { x:Math.round(r.x+r.width/2), y:Math.round(r.y+r.height/2) };
      })()`);
      const point = await configure();
      const activations = ['implicit-submit', 'external-submit'].includes(kind) ? [{ kind:'key', key:'Enter' }] :
        [{ kind:'click', ...point }, { kind:'key', key:'Enter' }, { kind:'key', key:'Space' }];
      for (const activation of activations) {
        refusals++;
        await configure(); events.length = 0;
        const result = await input(activation, { _retained_agent: true }).then(() => null, error => error);
        const effects = await evaluate('window.effects');
        if (result?.code !== 'sensitive_requires_focus' || !result.message.includes('requires focus') || effects !== 0 || events.length !== 0) {
          failures.push({ kind, label, activation:activation.key || activation.kind, error:result?.message, effects, dispatched:events.length });
        }
      }
      // A focused native activation must still reach this exact fixture handler.
      await configure(); events.length = 0;
      await input(kind.startsWith('handler-key') ? { kind:'key', key:'Enter' } : activations[0], { _retained_agent: false });
      assert((await evaluate('window.effects')) > 0, `MP-08: focused ${kind} ${label} did not activate`);
      focused++;
      assert.equal((await host.request({ op:'state' })).generation, bound.generation);
    }
  }
  await evaluate("window.cleanup(); document.querySelector('#stage').innerHTML='<button id=search>Search</button><p id=plain>Ordinary page area</p>'; window.effects=0; document.querySelector('#search').onclick=()=>window.effects++");
  for (const id of ['search', 'plain']) {
    const point = await evaluate(`(() => { const r=document.querySelector('#${id}').getBoundingClientRect(); return {x:Math.round(r.x+r.width/2),y:Math.round(r.y+r.height/2)}; })()`);
    await input({ kind:'click', ...point }, { _retained_agent:true });
  }
  assert.equal(await evaluate('window.effects'), 1, 'MP-08: positively routine Search remains usable');
  assert.deepEqual(failures, [], 'MP-11: retained matrix dispatched native input or handler effects');
  console.log(`MP-08/MP-10/MP-11 native matrix: ${refusals} retained refusals, ${focused} focused activations; Search and ordinary page click pass`);
}, false));

test("MP-08 P2: paired Tab releases on an approval button without activating or stopping Chromium", () => native(`${field}<button id="pay">Approve payment</button>`, async ({ host, bound, input, status, events }) => {
  const stream = await host.request({ op: "subscribe", ...bound });
  // Even a retained navigation may release on a sensitive target. The real
  // kernel focused-input case is also covered by the Rust native drill.
  await input({ kind: "key", key: "Tab" }, { _retained_agent: true });
  assert.equal(await status(), "submitted=0 releases=1 focus=pay");
  assert.deepEqual(events.map(event => event.type), ["keyDown", "keyUp"]);
  assert.equal((await host.request({ op: "state" })).generation, bound.generation);
  await host.request({ op: "poll", ...stream });
}));

// MP-08/MP-10/MP-11: generic retained keyboard allow-list, including delegated shortcuts.
test("MP-11: retained keyboard matrix refuses shortcuts before dispatch; editing/navigation remain usable", () => native(`${field}<div id="root"><button id="action">Search</button></div><textarea id="area"></textarea><div id="edit" contenteditable>text</div>`, async ({ host, bound, input, events, evaluate }) => {
  let refusals = 0;
  const failures=[];
  for (const location of ['element', 'document', 'window', 'react-root']) {
    await evaluate(`(() => {
      window.cleanup?.(); window.effects=0;
      const action=document.querySelector('#action'); action.focus();
      const target=${JSON.stringify(location)} === 'element' ? action :
        ${JSON.stringify(location)} === 'document' ? document :
        ${JSON.stringify(location)} === 'window' ? window : document.querySelector('#root');
      const effect=()=>window.effects++;
      target.addEventListener('keydown',effect); target.addEventListener('keyup',effect);
      window.cleanup=()=>{target.removeEventListener('keydown',effect);target.removeEventListener('keyup',effect)};
    })()`);
    for (const key of ['Delete', 'Backspace', 'ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End', 'Escape', 'a', 'F1', 'Control+a', 'Shift+Delete']) {
      events.length=0; await evaluate('window.effects=0');
      const result=await input({kind:'key',key}, {_retained_agent:true}).then(()=>null,error=>error);
      const effects=await evaluate('window.effects');
      if(result?.code!=='sensitive_requires_focus' || events.length!==0 || effects!==0)
        failures.push({location,key,code:result?.code,dispatched:events.length,effects});
      refusals++;
    }
    // Live focus allows the identical destructive shortcut.
    await evaluate('window.effects=0');
    await input({kind:'key',key:'Delete'}, {_retained_agent:false});
    assert.equal(await evaluate('window.effects'),2);
    assert.equal((await host.request({op:'state'})).generation,bound.generation);
  }
  await evaluate('window.cleanup()');
  assert.deepEqual(failures,[], 'MP-11: retained keyboard shortcuts dispatched input or effects');
  for (const id of ['field','area','edit']) {
    await evaluate(`(() => { const e=document.querySelector('#${id}'); e.focus();
      if(e.isContentEditable) { const r=document.createRange();r.selectNodeContents(e);r.collapse(false);const s=getSelection();s.removeAllRanges();s.addRange(r); }
      else e.setSelectionRange(e.value.length,e.value.length); })()`);
    events.length=0;
    await input({kind:'text',text:'routin'}, {_retained_agent:true});
    await input({kind:'key',key:'e'}, {_retained_agent:true});
    await input({kind:'key',key:'Space'}, {_retained_agent:true});
    await input({kind:'key',key:'Backspace'}, {_retained_agent:true});
    assert.equal(await evaluate(`document.querySelector('#${id}').value ?? document.querySelector('#${id}').textContent`),id==='edit' ? 'textroutine' : 'routine');
    for (const key of ['Delete','ArrowLeft','ArrowRight','ArrowUp','ArrowDown','Home','End']) {
      await input({kind:'key',key}, {_retained_agent:true});
    }
    assert.equal(events.filter(e=>e.method==='Input.insertText').length,1);
    // Escape is outside the allow-list even in an editable field.
    events.length=0;
    await assert.rejects(input({kind:'key',key:'Escape'}, {_retained_agent:true}), {code:'sensitive_requires_focus'});
    assert.deepEqual(events,[]);
  }
  await evaluate("document.querySelector('#field').focus()");
  events.length=0;
  await input({kind:'key',key:'Tab'}, {_retained_agent:true});
  await input({kind:'key',key:'Shift+Tab'}, {_retained_agent:true});
  assert.equal(await evaluate('document.activeElement.id'),'field');
  await input({kind:'scroll',x:10,y:10,delta_x:0,delta_y:100}, {_retained_agent:true});
  assert.deepEqual(events.map(e=>e.type),['keyDown','keyUp','keyDown','keyUp','mouseWheel']);
  // Text insertion into a non-editable control is also outside the allow-list.
  await evaluate("document.querySelector('#action').focus()"); events.length=0;
  await assert.rejects(input({kind:'text',text:'a'}, {_retained_agent:true}), {code:'sensitive_requires_focus'});
  assert.deepEqual(events,[]);
  console.log(`MP-08/MP-10/MP-11 keyboard matrix: ${refusals} zero-dispatch refusals; 4 focused shortcuts; editable text/navigation, Tab/Shift+Tab/wheel pass`);
}, false));

test("MP-11: retained observation cannot restart native Chromium after human stop", () => native(field, async ({ host, bound }) => {
  await host.request({op:'stop'});
  assert.equal(host.chromium.child,null);
  await assert.rejects(host.request({op:'state',_retained_agent:true}), {code:'not_focused_agent'});
  assert.equal(host.chromium.child,null);
  assert.equal(host.browser,null);
  const focused=await host.request({op:'state',_retained_agent:false});
  assert.equal(focused.generation,bound.generation+1);
  assert.equal(focused.tabs[0].tab_id,bound.tab_id);
}, false));
