// MP-08/MP-10/MP-11: DOM mirror v2 observer and service on real Chromium
// (credential-free local fixtures, the real host controller and CDP).
import test from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { inflateRawSync, deflateSync, constants as zlib } from "node:zlib";
import { mkdtemp, rm } from "node:fs/promises";
import path from "node:path";
import { KernelBrowserHost } from "./kernel-browser-host.mjs";

assert(process.env.CHARIOX_MDACCESS_DRILL_ROOT, "MP-11: explicit disposable drill root required");

async function mirrored(html, run, routes = {}) {
  const root = await mkdtemp(path.join(process.env.CHARIOX_MDACCESS_DRILL_ROOT, "mirror2-"));
  const server = createServer((request, response) => {
    const port = String(server.address().port), route = routes[request.url];
    if (route) { response.setHeader("Content-Type", route.type); for (const [name, value] of Object.entries(route.headers ?? {})) response.setHeader(name, value); response.end(route.body.replaceAll("PORT", port)); return; }
    response.setHeader("Content-Type", "text/html"); response.end(`<!doctype html>${html.replaceAll("PORT", port)}`);
  });
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const host = new KernelBrowserHost(root);
  try {
    const opened = await host.request({ op: "open", url: `http://127.0.0.1:${server.address().port}/` });
    const evaluate = async expression => {
      const { connection, sessionId } = await host.browser.resolvePageTarget(host.tabs.get(opened.tab_id).target_id);
      const reply = await connection.send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true }, sessionId);
      assert(!reply.exceptionDetails, "MP-10: credential-free fixture evaluation failed");
      return reply.result.value;
    };
    // about:blank is "complete" too: wait for the fixture document itself.
    for (let i = 0; i < 400 && await evaluate("location.protocol === 'http:' && document.readyState === 'complete'").catch(() => false) !== true; i++) await new Promise(resolve => setTimeout(resolve, 25));
    await host.request({ op: "state" });
    const subscribe = async () => {
      const { subscription_id } = await host.request({ op: "mirror_subscribe", tab_id: opened.tab_id, generation: opened.generation, device_scale_factor: 1, wire: 2 });
      // Protocol 489: bodies may be deflated in the subscription's context (in order, fresh per reset).
      let applied = 0, context = Buffer.alloc(0);
      const next = async (wait_ms = 0, reset = false) => {
        let packet = await host.request({ op: "mirror_next", subscription_id, generation: opened.generation, after_sequence: reset ? 0 : applied, drift_nodes: [], wait_ms }); applied = packet.sequence;
        if (packet.reset) context = Buffer.alloc(0);
        if (packet.encoding === "deflate") { context = Buffer.concat([context, Buffer.from(packet.packet_base64, "base64")]); const out = inflateRawSync(context, { finishFlush: zlib.Z_SYNC_FLUSH }); packet = { ...JSON.parse(out.subarray(out.length - packet.packet_bytes)), resources: packet.resources ?? [], tiles: packet.tiles ?? [] }; }
        return { ops: [], resources: [], tiles: [], ...packet };
      };
      const input = async (sequence, action) => host.request({ op: "input", tab_id: opened.tab_id, generation: opened.generation, document_id: host.tabs.get(opened.tab_id).document_id, input: { kind: "mirror", subscription_id, sequence, action } });
      return { next, input };
    };
    const { next, input } = await subscribe();
    await run({ next, evaluate, host, input, subscribe });
  } finally {
    await host.stop();
    await new Promise(resolve => server.close(resolve));
    await rm(root, { recursive: true, force: true });
  }
}

test("MP-10: a focused unchanged field does not answer every credit (no form-op busy loop)", () => mirrored(
  '<input id="q" autofocus value="query"><p>page</p>', async ({ next, evaluate }) => {
    await evaluate("document.querySelector('#q').focus()");
    assert.equal((await next()).reset, true);
    const started = Date.now(), packet = await next(600);
    assert.deepEqual(packet.ops.map(op => op.op), [], "MP-10: nothing changed, the credit long-polls");
    assert(Date.now() - started >= 500, "MP-10: the credit waited for a change");
    // A real change still travels at once, exactly once.
    await evaluate("document.querySelector('#q').value='query2';document.querySelector('#q').dispatchEvent(new Event('input',{bubbles:true}))");
    const changed = await next(600);
    assert.deepEqual(changed.ops.filter(op => op.op === "form").map(op => op.form.value), ["query2"]);
  }));

// Typeahead shape (Wikipedia portal): the page rebuilds its suggestion list with innerHTML on every keystroke.
const typeahead = '<input id="q"><div id="s"></div><script>window.render=items=>{document.querySelector("#s").innerHTML=items.map(t=>`<a class="link" href="/wiki/${t}" style="display:block;height:20px"><h3 class="title"><em>${t[0]}</em>${t.slice(1)}</h3><p class="desc">about ${t}</p></a>`).join("")}</script>';
test("MP-10: a rebuilt subtree of the same shape travels as text/attribute diffs, not re-serialized nodes", () => mirrored(typeahead, async ({ next, evaluate, input }) => {
  assert.equal((await next()).reset, true);
  await evaluate("render(['Ada','Adams'])");
  const first = await next(600);
  assert.equal(first.ops.filter(op => op.op === "children").reduce((n, op) => n + op.nodes.length, 0), 2 * 7, "MP-10: the first list is serialized once");
  const linkId = first.ops.find(op => op.op === "children").children[0];
  await evaluate("render(['Ada Lovelace','Adam Smith'])");
  const second = await next(600);
  assert.deepEqual([...new Set(second.ops.map(op => op.op))].sort(), ["text"], JSON.stringify(second.ops));
  assert.deepEqual(second.ops.map(op => op.text).sort(), ["about Ada Lovelace", "about Adam Smith", "da Lovelace", "dam Smith"]);
  assert(JSON.stringify(second).length < 1500, "MP-10: one keystroke-sized delta");
  // One more item: only that item's records travel.
  await evaluate("render(['Ada','Adams','Adobe'])");
  const third = await next(600), added = third.ops.filter(op => op.op === "children");
  assert.equal(added.length, 1); assert.equal(added[0].nodes.length, 7);
  // A viewer that clicked the old node (before the morph reached it) is refused.
  await assert.rejects(input(first.sequence, { kind: "click", node_id: linkId, x: 5, y: 5 }), /changed mirror input target/);
}));

test("MP-10: a node added and removed before the drain sends no children op", () => mirrored("<p>page</p>", async ({ next, evaluate }) => {
  assert.equal((await next()).reset, true);
  await evaluate("const t=document.createElement('div');document.body.append(t);t.remove();true");
  const started = Date.now(), packet = await next(600);
  assert.deepEqual(packet.ops, []); assert(Date.now() - started >= 500);
}));

// Compact snapshot rows: [idDelta, parentBack, tag | kindCode, ...]; kind code 0 = text (4th item).
const texts = packet => { const out = new Map(); let id = 0; for (const row of packet.nodes) { id += row[0]; if (row[2] === 0) out.set(row[3], `n${id}`); } return out; };
test("MP-10/MP-11: two viewers of one tab keep independent snapshots and deltas (one resets)", () => mirrored('<p id="x">one</p><p id="y">two</p>', async ({ evaluate, subscribe }) => {
  const a = await subscribe(), b = await subscribe();
  const idsA = texts(await a.next()), idsB = texts(await b.next());
  await evaluate("document.querySelector('#x').firstChild.data='uno';true");
  const textOps = packet => packet.ops.filter(op => op.op === "text").map(op => [op.id, op.text]);
  assert.deepEqual(textOps(await a.next(600)), [[idsA.get("one"), "uno"]], "MP-10: viewer A gets the change under its own ids");
  assert.deepEqual(textOps(await b.next(600)), [[idsB.get("one"), "uno"]], "MP-10: viewer B gets the same change under its own ids");
  // Only B resets; A's next delta still names A's nodes.
  const idsB2 = texts(await b.next(0, true));
  await evaluate("document.querySelector('#y').firstChild.data='dos';true");
  assert.deepEqual(textOps(await a.next(600)), [[idsA.get("two"), "dos"]]);
  assert.deepEqual(textOps(await b.next(600)), [[idsB2.get("two"), "dos"]]);
}));

// A cross-origin child frame (localhost vs 127.0.0.1: an isolated frame) whose own
// stylesheet is cross-origin to it: its CSSOM cannot read the rules.
test("MP-10: a child frame's unreadable cross-origin stylesheet reaches the viewer", () => mirrored(
  '<p>top</p><iframe src="http://localhost:PORT/frame" style="width:300px;height:100px"></iframe>', async ({ next }) => {
    await new Promise(resolve => setTimeout(resolve, 500));
    const packet = await next();
    assert.equal(packet.reset, true);
    const css = JSON.stringify([...packet.ops, ...packet.nodes]);
    assert.match(css, /rgb\(1, 2, 3\)/, "MP-10: the frame's sheet text is in the snapshot packet");
  }, {
    "/frame": { type: "text/html", body: '<!doctype html><link rel="stylesheet" href="http://127.0.0.1:PORT/frame.css"><p class="mark">framed</p>' },
    "/frame.css": { type: "text/css", body: ".mark { color: rgb(1, 2, 3); }" },
  }));

test("MP-10: adopted stylesheet edits with an unchanged rule count reach the viewer (document and shadow root)", () => mirrored(
  '<p>doc</p><div id="h"></div><script>window.sheet=new CSSStyleSheet();sheet.replaceSync("p{color:rgb(255,0,0)}");document.adoptedStyleSheets=[sheet];const r=document.querySelector("#h").attachShadow({mode:"open"});r.innerHTML="<p>shadow</p>";window.inner=new CSSStyleSheet();inner.replaceSync("p{color:rgb(0,128,0)}");r.adoptedStyleSheets=[inner]</script>', async ({ next, evaluate }) => {
    assert.equal((await next()).reset, true);
    await evaluate("sheet.replaceSync('p{color:rgb(0,0,255)}');inner.cssRules[0].style.color='rgb(7,7,7)';true");
    let ops = [];
    for (let i = 0; i < 3 && ops.length < 2; i++) ops.push(...(await next(2000)).ops.filter(op => op.op === "adopted"));
    const text = JSON.stringify(ops);
    assert.match(text, /rgb\(0, 0, 255\)/, "MP-10: document sheet replaced");
    assert.match(text, /rgb\(7, 7, 7\)/, "MP-10: shadow sheet rule edited");
  }));

// Compact rows -> [id, tag] for element rows (kind codes are numbers).
const elements = packet => { const out = []; let id = 0; for (const row of packet.nodes) { id += row[0]; if (typeof row[2] === "string") out.push([`n${id}`, row[2], row[3] || {}]); } return out; };
test("MP-08/MP-11: typing reaches a field inside a mirrored cross-origin frame; its password field stays refused", () => mirrored(
  '<p>top</p><iframe src="http://localhost:PORT/form" style="width:400px;height:120px"></iframe>', async ({ next, input }) => {
    await new Promise(resolve => setTimeout(resolve, 500));
    const snapshot = await next();
    const fields = elements(snapshot).filter(([id, tag]) => tag === "input" && Number(id.slice(1)) >= 1e9);
    const [field] = fields.find(([, , attrs]) => attrs.id === "f") ?? [], [secret] = fields.find(([, , attrs]) => attrs.id === "p") ?? [];
    assert(field && secret, "MP-10: the frame's inputs are mirrored");
    await input(snapshot.sequence, { kind: "text", node_id: field, text: "hi" });
    const changed = await next(1000);
    assert.deepEqual(changed.ops.filter(op => op.op === "form").map(op => [op.id, op.form.value]), [[field, "hi"]]);
    await assert.rejects(input(changed.sequence, { kind: "text", node_id: secret, text: "x" }), /sensitive|protected|refus/i);
  }, {
    "/form": { type: "text/html", body: '<!doctype html><input id="f" type="text"><input id="p" type="password">' },
  }));

// A page may shrink its Resource Timing buffer (Chrome keeps 250 entries by default): bytes the
// page loads afterwards (here a CSS image applied after load) must still reach the viewer.
const svg = color => `<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40"><rect width="40" height="40" fill="${color}"/></svg>`;
test("MP-10: a CSS image the page loads after its Resource Timing buffer is full still reaches the viewer", () => mirrored(
  '<script>performance.setResourceTimingBufferSize(1)</script><style>.late{width:40px;height:40px;background-image:url(/late.svg)}</style><img src="/early.svg"><div id="box"></div>', async ({ next, evaluate }) => {
    await new Promise(resolve => setTimeout(resolve, 500));
    assert.equal((await next()).reset, true);
    // The page loads the image itself (a preload) once its buffer is full, then uses it.
    await evaluate("new Promise(resolve=>{const image=new Image();image.onload=()=>resolve(true);image.src='/late.svg'}).then(()=>{document.getElementById('box').className='late';return true})");
    const delivered = [];
    for (let i = 0; i < 8 && !delivered.some(r => Buffer.from(r.data_base64, "base64").toString().includes("#0a0b0c")); i++) delivered.push(...(await next(1000)).resources);
    assert(delivered.some(r => r.mime_type === "image/svg+xml" && Buffer.from(r.data_base64, "base64").toString().includes("#0a0b0c")), `MP-10: the late CSS image arrived (${delivered.map(r => r.key)})`);
  }, {
    "/early.svg": { type: "image/svg+xml", body: svg("#010203") },
    "/late.svg": { type: "image/svg+xml", body: svg("#0a0b0c") },
  }));

test("MP-10: review #941-3 an imported sheet's url() resolves against the imported sheet, not the importer", () => mirrored(
  '<link rel="stylesheet" href="/css/main.css"><div class="x" style="width:20px;height:20px">a</div>', async ({ next }) => {
    await new Promise(resolve => setTimeout(resolve, 500));
    const delivered = [...(await next()).resources];
    const image = () => delivered.some(r => Buffer.from(r.data_base64, "base64").toString().includes("#0d0e0f")), font = () => delivered.some(r => r.mime_type === "font/woff2");
    for (let i = 0; i < 6 && !(image() && font()); i++) delivered.push(...(await next(1000)).resources);
    assert(image(), `MP-10: the imported sheet's image arrived (${delivered.map(r => r.key)})`);
    assert(font(), `MP-10: the imported sheet's font arrived (${delivered.map(r => r.mime_type)})`);
  }, {
    "/css/main.css": { type: "text/css", body: '@import url("/themes/dark/theme.css");' },
    "/themes/dark/theme.css": { type: "text/css", body: '@font-face{font-family:T;src:url(t.woff2) format("woff2")}.x{background-image:url(icon.svg);font-family:T}' },
    "/themes/dark/t.woff2": { type: "font/woff2", body: "wOF2" + "\u0001".repeat(60) },
    "/themes/dark/icon.svg": { type: "image/svg+xml", body: svg("#0d0e0f") },
  }));

test("MP-10: review #941-6 CSSOM sheet.disabled toggles reach the viewer (disable and enable)", () => mirrored(
  '<style id="a">p{color:rgb(1,2,3)}</style><style id="b">p{background:rgb(4,5,6)}</style><p>t</p><script>document.getElementById("b").sheet.disabled=true</script>', async ({ next, evaluate }) => {
    const snapshot = await next(); let id = 0; const styles = [];
    for (const row of snapshot.nodes) { id += row[0]; if (row[2] === "style") styles.push([`n${id}`, row[3]?.media ?? null]); }
    assert.deepEqual(styles.map(([, media]) => media), [null, "not all"], "MP-10: the snapshot carries the disabled sheet as media=not all");
    await evaluate("document.getElementById('a').sheet.disabled=true;document.getElementById('b').sheet.disabled=false;true");
    const ops = [];
    for (let i = 0; i < 4 && ops.length < 2; i++) ops.push(...(await next(1500)).ops.filter(op => op.op === "attr" && op.name === "media"));
    assert.deepEqual(ops.map(op => [op.id, op.value]).sort(), [[styles[0][0], "not all"], [styles[1][0], null]].sort());
  }));

test("MP-10: review #941-1 attributes referenced only by a CDP-read cross-origin sheet survive the snapshot (top level and child frame)", () => mirrored(
  '<link rel="stylesheet" href="http://localhost:PORT/cdn.css"><div id="top" data-state="open">top</div><iframe src="http://localhost:PORT/child" style="width:300px;height:100px"></iframe>', async ({ next, evaluate }) => {
    await new Promise(resolve => setTimeout(resolve, 800));
    const snapshot = await next();
    const divs = elements(snapshot).filter(([, tag]) => tag === "div");
    const top = divs.find(([id]) => Number(id.slice(1)) < 1e9), child = divs.find(([id]) => Number(id.slice(1)) >= 1e9);
    assert.equal(top?.[2]["data-state"], "open", `MP-10: top-level attribute kept (${JSON.stringify(divs)})`);
    assert.equal(child?.[2]["data-state"], "open", `MP-10: child-frame attribute kept (${JSON.stringify(divs)})`);
    await evaluate("document.getElementById('top').dataset.state='closed';true");
    const ops = []; for (let i = 0; i < 3 && !ops.length; i++) ops.push(...(await next(1000)).ops.filter(op => op.op === "attr" && op.name === "data-state"));
    assert.deepEqual(ops.map(op => [op.id, op.value]), [[top[0], "closed"]], "MP-10: a later change of that attribute travels");
  }, {
    "/cdn.css": { type: "text/css", body: '[data-state="open"]{outline:1px solid red}' },
    "/child": { type: "text/html", body: '<!doctype html><link rel="stylesheet" href="http://127.0.0.1:PORT/cdn.css"><div data-state="open">child</div>' },
  }));

test("MP-10: review #941-2 a cross-origin frame's window scroll reaches every viewer (another viewer's scroll_to, page script)", () => mirrored(
  '<p>top</p><iframe src="http://localhost:PORT/long" style="width:300px;height:100px"></iframe>', async ({ next, subscribe }) => {
    await new Promise(resolve => setTimeout(resolve, 800));
    const a = { next }, b = await subscribe();
    const snapA = await a.next(), snapB = await b.next();
    const documentOf = snapshot => { let id = 0; for (const row of snapshot.nodes) { id += row[0]; if (row[2] === 1 && id >= 1e9) return `n${id}`; } return null; };
    const docA = documentOf(snapA), docB = documentOf(snapB);
    assert(docA && docB, "MP-10: both viewers mirror the frame document");
    // Viewer B scrolls the frame; viewer A must follow.
    await b.input(snapB.sequence, { kind: "scroll_to", node_id: docB, x: 0, y: 300 });
    const scrolls = []; for (let i = 0; i < 4 && !scrolls.length; i++) scrolls.push(...(await a.next(1000)).ops.filter(op => op.op === "scroll" && op.id === docA));
    assert.deepEqual(scrolls.map(op => op.scroll), [[0, 300]], "MP-10: viewer A follows the frame's new position");
    // The frame's own script scrolls it: both viewers follow.
    for (const [viewer, doc] of [[a, docA], [b, docB]]) {
      const seen = []; for (let i = 0; i < 8 && !seen.some(op => op.scroll[1] === 600); i++) seen.push(...(await viewer.next(1000)).ops.filter(op => op.op === "scroll" && op.id === doc));
      assert(seen.some(op => op.scroll[1] === 600), `MP-10: page-script frame scroll reaches the viewer (${JSON.stringify(seen)})`);
    }
  }, {
    "/long": { type: "text/html", body: '<!doctype html><div style="height:2000px">long</div><script>setTimeout(()=>scrollTo(0,600),3500)</script>' },
  }));

// Wikipedia "Web browser" shape: a video poster (opaque region, lossless still) beside the controls the viewer clicks.
const opaque = '<canvas id="c" width="320" height="180"></canvas><p id="x">day</p><script>const g=document.querySelector("#c").getContext("2d");window.paint=c=>{g.fillStyle=c;g.fillRect(0,0,320,180)};paint("#3a6")</script>';
test("MP-10: opaque-region stills never ride on a DOM-change packet and an unchanged still is not sent again", () => mirrored(opaque, async ({ next, evaluate }) => {
  const first = await next();
  assert.equal(first.reset, true); assert.equal(first.tiles.length, 1, "MP-10: the snapshot carries the still");
  await new Promise(resolve => setTimeout(resolve, 1100));
  await evaluate("document.querySelector('#x').className='night'");
  const change = await next(600);
  assert.deepEqual(change.ops.map(op => op.op), ["attr"]);
  assert.equal(change.tiles.length, 0, "MP-10: the echo does not queue behind a still refresh");
  // Unchanged pixels: refresh packets carry no still; changed pixels travel once.
  for (let i = 0; i < 2; i++) { await new Promise(resolve => setTimeout(resolve, 1100)); assert.equal((await next(600)).tiles.length, 0, "MP-10: unchanged still not re-sent"); }
  await evaluate("paint('#a36')");
  await new Promise(resolve => setTimeout(resolve, 1100));
  let changed = []; for (let i = 0; i < 4 && !changed.length; i++) changed = (await next(600)).tiles;
  assert.equal(changed.length, 1, "MP-10: a changed still reaches the viewer");
}));

test("MP-11: review #941-1(P1) text/keys are refused for a focused field the viewer has not applied (delayed packet, programmatic focus)", () => mirrored(
  '<input id="a"><p id="x">page</p>', async ({ next, evaluate, input }) => {
    const snapshot = await next();
    await input(snapshot.sequence, { kind: "focus", node_id: elements(snapshot).find(([, tag, attrs]) => tag === "input" && attrs.id === "a")[0] });
    // The page inserts and focuses another input; credit 2 serializes it, but that reply never reached the viewer.
    await evaluate("const b=document.createElement('input');b.id='b';document.body.append(b);b.focus()");
    const unseen = await next(600);
    assert(unseen.ops.some(op => op.op === "children"), "MP-10: the new input is serialized in a later packet");
    await assert.rejects(input(snapshot.sequence, { kind: "text", text: "secret" }), /changed mirror input target/);
    await assert.rejects(input(snapshot.sequence, { kind: "key", key: "Enter" }), /changed mirror input target/);
    assert.equal(await evaluate("document.querySelector('#b').value"), "", "MP-11: nothing reached the unseen field");
    // Once applied, the same focus is admitted.
    await input(unseen.sequence, { kind: "text", text: "ok" });
    assert.equal(await evaluate("document.querySelector('#b').value"), "ok");
    // An identity change of the focused field (text -> password) after the viewer's epoch is refused too.
    await evaluate("document.querySelector('#b').type='search'");
    const swapped = await next(600);
    await assert.rejects(input(unseen.sequence, { kind: "text", text: "x" }), /changed mirror input target/);
    await input(swapped.sequence, { kind: "text", text: "!" });
    assert.equal(await evaluate("document.querySelector('#b').value"), "ok!");
  }));

test("MP-10: review #941-2 a resource whose final packet the viewer never applied is sent again after its reset (applied ones are not)", () => mirrored(
  '<img src="/one.svg">', async ({ next }) => {
    await new Promise(resolve => setTimeout(resolve, 500));
    assert.equal((await next()).reset, true);
    const wanted = r => r.mime_type === "image/svg+xml" && Buffer.from(r.data_base64, "base64").toString().includes("#0d0e0f");
    const until = async () => { for (let i = 0; i < 8; i++) { const packet = await next(1000); if (packet.resources.some(wanted)) return packet; } return null; };
    assert(await until(), "MP-10: the image is delivered once");
    // That packet was lost (or a gap blocked it): the viewer resets having applied only the snapshot.
    assert.equal((await next(0, true)).reset, true);
    assert(await until(), "MP-10: the unapplied image is sent again");
    await next(600); // the viewer acknowledges it (its next credit names that sequence)
    assert.equal((await next(0, true)).reset, true);
    for (let i = 0; i < 3; i++) assert(!(await next(600)).resources.some(wanted), "MP-10: an applied image is not re-sent");
  }, { "/one.svg": { type: "image/svg+xml", body: svg("#0d0e0f") } }));

const shadowForm = '<x-box></x-box><script>customElements.define("x-box",class extends HTMLElement{constructor(){super();this.attachShadow({mode:"open"}).innerHTML="<input id=s><input id=c type=checkbox>"}})</script>';
test("MP-10: review #941-3 typing and checkbox changes inside an open shadow root echo as form ops", () => mirrored(shadowForm, async ({ next, input }) => {
  const snapshot = await next();
  const [field] = elements(snapshot).find(([, tag, attrs]) => tag === "input" && attrs.id === "s") ?? [], [box] = elements(snapshot).find(([, tag, attrs]) => tag === "input" && attrs.id === "c") ?? [];
  assert(field && box, "MP-10: the shadow controls are mirrored");
  await input(snapshot.sequence, { kind: "text", node_id: field, text: "hi" });
  await input(snapshot.sequence, { kind: "click", node_id: box, x: 5, y: 5 });
  const forms = new Map();
  for (let i = 0; i < 4 && forms.size < 2; i++) for (const op of (await next(1000)).ops) if (op.op === "form") forms.set(op.id, { ...forms.get(op.id), ...op.form });
  assert.equal(forms.get(field)?.value, "hi"); assert.equal(forms.get(box)?.checked, true);
}));

test("MP-10: review #941-4 property-only changes of unfocused controls reach the viewer (value, checked, radio group, select, form reset)", () => mirrored(
  '<form id="f"><input id="q" value="a"><input id="c" type="checkbox"><input type="radio" name="t" id="r1" checked><input type="radio" name="t" id="r2"><select id="s"><option>x</option><option>y</option></select></form><button id="b">b</button>', async ({ next, evaluate }) => {
    const snapshot = await next();
    const id = name => elements(snapshot).find(([, , attrs]) => attrs.id === name)[0];
    await evaluate("document.querySelector('#b').focus();q.value='updated';c.checked=true;r2.checked=true;s.selectedIndex=1;true");
    const collect = async want => { const forms = new Map(); for (let i = 0; i < 6 && !want(forms); i++) for (const op of (await next(1000)).ops) if (op.op === "form") forms.set(op.id, { ...forms.get(op.id), ...op.form }); return forms; };
    let forms = await collect(f => f.size >= 5);
    assert.equal(forms.get(id("q"))?.value, "updated"); assert.equal(forms.get(id("c"))?.checked, true);
    assert.equal(forms.get(id("r2"))?.checked, true); assert.equal(forms.get(id("r1"))?.checked, false, "MP-10: the unchecked radio of the group");
    assert.equal(forms.get(id("s"))?.selected_index, 1);
    await evaluate("f.reset();true");
    forms = await collect(f => f.get(id("q"))?.value === "a" && f.get(id("s"))?.selected_index === 0);
    assert.equal(forms.get(id("q"))?.value, "a"); assert.equal(forms.get(id("s"))?.selected_index, 0); assert.equal(forms.get(id("c"))?.checked, false);
    // Unchanged controls stay silent.
    for (let i = 0; i < 3; i++) assert.deepEqual((await next(1900)).ops.filter(op => op.op === "form"), []);
  }));

test("MP-10: review #941-5 CSSOM edits through a readable linked stylesheet with an unchanged rule count reach the viewer", () => mirrored(
  '<link rel="stylesheet" href="/a.css"><p>text</p>', async ({ next, evaluate }) => {
    await new Promise(resolve => setTimeout(resolve, 300));
    assert.equal((await next()).reset, true);
    const css = async needle => { for (let i = 0; i < 5; i++) for (const op of (await next(1000)).ops) if (op.op === "css" && op.css.includes(needle)) return true; return false; };
    await evaluate("document.styleSheets[0].cssRules[0].style.color='rgb(1, 2, 3)';true");
    assert(await css("rgb(1, 2, 3)"), "MP-10: a declaration edit reaches the viewer");
    await evaluate("const s=document.styleSheets[0];s.deleteRule(0);s.insertRule('p{color:rgb(4, 5, 6)}',0);true");
    assert(await css("rgb(4, 5, 6)"), "MP-10: a replaced rule reaches the viewer");
  }, { "/a.css": { type: "text/css", body: "p{color:rgb(9, 9, 9)}" } }));

// A noisy PNG (previews of it cannot compress below the input-time packet budget).
const noisePng = (w, h, seed) => {
  const crc = buf => { let c = ~0; for (const b of buf) { c ^= b; for (let k = 0; k < 8; k++) c = c >>> 1 ^ 0xedb88320 & -(c & 1); } return ~c >>> 0; };
  const chunk = (type, data) => { const out = Buffer.alloc(12 + data.length); out.writeUInt32BE(data.length); out.write(type, 4); data.copy(out, 8); out.writeUInt32BE(crc(out.subarray(4, 8 + data.length)), 8 + data.length); return out; };
  const raw = Buffer.alloc((w * 3 + 1) * h); let x = seed; for (let i = 0; i < raw.length; i++) raw[i] = i % (w * 3 + 1) ? (x = x * 1103515245 + 12345 >>> 0) >>> 24 : 0;
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(w); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = 2;
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", ihdr), chunk("IDAT", deflateSync(raw)), chunk("IEND", Buffer.alloc(0))]).toString("base64");
};
test("MP-10: review #941-6 previews respect the packet budget while the viewer moves (oversized preview, preview after another resource)", () => mirrored(
  `<img src="data:image/png;base64,${noisePng(320, 320, 1)}" style="display:block;width:1900px;height:1900px"><img src="data:image/png;base64,${noisePng(320, 320, 2)}" style="display:block;width:1900px;height:1900px">`, async ({ next, input }) => {
    const snapshot = await next();
    const sizes = [];
    for (let i = 0; i < 6; i++) {
      await input((await next(0)).sequence, { kind: "scroll_to", node_id: null, x: 0, y: 100 + i * 50 });
      const packet = await next(200);
      sizes.push(packet.resources.reduce((n, r) => n + r.data_base64.length, 0));
    }
    assert(sizes.every(n => n <= 64 * 1024), `MP-10: every packet within 1 s of input stays within 64 KB of media (${sizes})`);
  }));

// Typeahead inside a cross-origin frame: the top page asks it (postMessage) to rebuild its list with the same shape.
const frameTypeahead = '<!doctype html><div id="s"></div><script>const render=items=>{document.querySelector("#s").innerHTML=items.map(t=>`<a href="#x" style="display:block;height:20px"><b>${t}</b></a>`).join("")};render(["Ada","Adams"]);onmessage=e=>{render(e.data);document.querySelector("a").focus()}</script>';
test("MP-11: review #941-1(P1) a morphed child-frame node refuses clicks and keys from before the morph reached the viewer", () => mirrored(
  '<p>top</p><iframe src="http://localhost:PORT/list" style="width:300px;height:100px"></iframe>', async ({ next, evaluate, input }) => {
    await new Promise(resolve => setTimeout(resolve, 800));
    const snapshot = await next();
    const [link] = elements(snapshot).find(([id, tag]) => tag === "a" && Number(id.slice(1)) >= 1e9) ?? [];
    assert(link, "MP-10: the frame's links are mirrored");
    await input(snapshot.sequence, { kind: "focus", node_id: link });
    await evaluate("frames[0].postMessage(['Bob','Bobby'],'*');true");
    let morphed = null; for (let i = 0; i < 4 && !morphed; i++) { const packet = await next(1000); if (packet.ops.some(op => op.op === "text" && op.text === "Bob")) morphed = packet; }
    assert(morphed && !morphed.ops.some(op => op.op === "children" && op.nodes.length), "MP-10: the rebuilt list travels as a morph (same ids, new text)");
    await assert.rejects(input(snapshot.sequence, { kind: "click", node_id: link, x: 5, y: 5 }), /changed mirror input target/);
    await assert.rejects(input(snapshot.sequence, { kind: "key", key: "Enter" }), /changed mirror input target/);
    // Once the viewer applied the morph, the same target is admitted.
    await input(morphed.sequence, { kind: "key", key: "Enter" });
  }, { "/list": { type: "text/html", body: frameTypeahead } }));

const shadowScroller = '<x-s></x-s><script>customElements.define("x-s",class extends HTMLElement{constructor(){super();this.attachShadow({mode:"open"}).innerHTML=\'<div id="sc" style="height:100px;overflow:auto"><div style="height:2000px">long</div></div>\'}})</script>';
test("MP-10: review #941-2 scrolling inside an open shadow root reaches every viewer (page script, another viewer's scroll_to)", () => mirrored(shadowScroller, async ({ next, evaluate, subscribe }) => {
  const a = { next }, b = await subscribe();
  const snapA = await a.next(), snapB = await b.next();
  const scroller = snapshot => elements(snapshot).find(([, , attrs]) => attrs.id === "sc")?.[0];
  const [idA, idB] = [scroller(snapA), scroller(snapB)];
  assert(idA && idB, "MP-10: the shadow scroller is mirrored");
  const scrolls = async (viewer, id, y) => { const seen = []; for (let i = 0; i < 4 && !seen.some(s => s[1] === y); i++) seen.push(...(await viewer.next(1000)).ops.filter(op => op.op === "scroll" && op.id === id).map(op => op.scroll)); return seen; };
  await evaluate("document.querySelector('x-s').shadowRoot.getElementById('sc').scrollTop=300;true");
  assert.deepEqual(await scrolls(a, idA, 300), [[0, 300]], "MP-10: viewer A follows the page's shadow scroll");
  assert.deepEqual(await scrolls(b, idB, 300), [[0, 300]], "MP-10: viewer B follows the page's shadow scroll");
  await b.input((await b.next(0)).sequence, { kind: "scroll_to", node_id: idB, x: 0, y: 600 });
  assert.deepEqual(await scrolls(a, idA, 600), [[0, 600]], "MP-10: viewer A follows viewer B's shadow scroll");
}));

test("MP-08: review #941-3 focus inside an attached cross-origin frame names the frame's leaf (focus, native Tab, programmatic, leaving)", () => mirrored(
  '<input id="top"><iframe src="http://localhost:PORT/two" style="width:300px;height:100px"></iframe>', async ({ next, evaluate, input }) => {
    await new Promise(resolve => setTimeout(resolve, 800));
    const snapshot = await next();
    const id = name => elements(snapshot).find(([, , attrs]) => attrs.id === name)?.[0];
    const [a, b, top] = [id("a"), id("b"), id("top")];
    assert(a && b && Number(a.slice(1)) >= 1e9, "MP-10: the frame's inputs are mirrored");
    let focused = snapshot.focused, sequence = snapshot.sequence;
    const focusOf = async want => { for (let i = 0; i < 4 && focused !== want; i++) { const packet = await next(1000); sequence = packet.sequence; if (Object.hasOwn(packet, "focused")) focused = packet.focused; } return focused; };
    await input(sequence, { kind: "focus", node_id: a });
    assert.equal(await focusOf(a), a, "MP-08: the viewer is told the child field has focus, not the iframe");
    await input(sequence, { kind: "key", key: "Tab" });
    assert.equal(await focusOf(b), b, "MP-08: native Tab inside the frame moves the viewer's focus");
    await evaluate("frames[0].postMessage('a','*');true");
    assert.equal(await focusOf(a), a, "MP-08: programmatic focus inside the frame reaches the viewer");
    await evaluate("document.querySelector('#top').focus();true");
    assert.equal(await focusOf(top), top, "MP-08: focus leaving the frame reaches the viewer");
  }, { "/two": { type: "text/html", body: '<!doctype html><input id="a"><input id="b"><script>onmessage=e=>document.getElementById(e.data).focus()</script>' } }));

test("MP-10: review #941-4 a CDP-read cross-origin sheet keeps its already-loaded imports (own base, media condition, images and fonts)", () => mirrored(
  '<link rel="stylesheet" href="http://localhost:PORT/cdn/main.css"><div class="x" style="width:20px;height:20px">a</div>', async ({ next }) => {
    await new Promise(resolve => setTimeout(resolve, 800));
    const snapshot = await next(), delivered = [...snapshot.resources];
    const css = JSON.stringify([...snapshot.ops, ...snapshot.nodes]);
    assert.match(css, /rgb\(1, 1, 1\)/, "MP-10: the importing sheet's own rules");
    assert.match(css, /@media screen ?\{[^{}]*(?:\{[^{}]*\}[^{}]*)*\.x ?\{[^}]*rgb\(3, 1, 4\)/, `MP-10: the imported rules under their media condition (${css.slice(0, 2000)})`);
    const image = () => delivered.some(r => Buffer.from(r.data_base64, "base64").toString().includes("#030104")), font = () => delivered.some(r => r.mime_type === "font/woff2");
    for (let i = 0; i < 6 && !(image() && font()); i++) delivered.push(...(await next(1000)).resources);
    assert(image(), `MP-10: the imported sheet's image (own base) arrived (${delivered.map(r => r.key)})`);
    assert(font(), `MP-10: the imported sheet's font (own base) arrived (${delivered.map(r => r.mime_type)})`);
  }, {
    "/cdn/main.css": { type: "text/css", body: '@import url("../themes/theme.css") screen;\n.y{color:rgb(1,1,1)}' },
    "/themes/theme.css": { type: "text/css", body: '@font-face{font-family:T;src:url(t.woff2) format("woff2")}.x{background-image:url(icon.svg);font-family:T;color:rgb(3,1,4)}' },
    "/themes/t.woff2": { type: "font/woff2", headers: { "Access-Control-Allow-Origin": "*" }, body: "wOF2" + "\u0001".repeat(60) },
    "/themes/icon.svg": { type: "image/svg+xml", body: svg("#030104") },
  }));

test("MP-10: review #941-5 property-only changes of controls beyond the first reconciliation batch reach the viewer; unchanged ones stay silent", () => mirrored(
  '<button id="f">f</button>' + Array.from({ length: 2100 }, (_, i) => `<input type="checkbox" id="c${i}">`).join(""), async ({ next, evaluate }) => {
    const snapshot = await next();
    const late = elements(snapshot).find(([, , attrs]) => attrs.id === "c2050")[0];
    await evaluate("document.querySelector('#f').focus();document.querySelector('#c2050').checked=true;true");
    const forms = []; for (let i = 0; i < 4 && !forms.length; i++) forms.push(...(await next(1900)).ops.filter(op => op.op === "form"));
    assert.deepEqual(forms.map(op => [op.id, op.form]), [[late, { checked: true }]]);
    for (let i = 0; i < 2; i++) assert.deepEqual((await next(1900)).ops.filter(op => op.op === "form"), [], "MP-10: unchanged controls stay silent");
  }));

test("MP-08: review #941-2 Shift+Arrow/Home/End extend the kernel selection, so typing replaces the range (input and textarea)", () => mirrored(
  '<input id="i" value="hello world"><textarea id="t">one two\nthree</textarea>', async ({ next, evaluate, input }) => {
    await next();
    for (const [selector, keys, typed, expected] of [["#i", ["Shift+ArrowLeft", "Shift+ArrowLeft", "Shift+ArrowLeft", "Shift+ArrowLeft", "Shift+ArrowLeft"], "there", "hello there"], ["#i", ["Home", "Shift+End"], "new", "new"], ["#t", ["Shift+ArrowUp", "Shift+Home"], "1", "1"], ["#t", ["Shift+ArrowLeft", "Shift+Home", "Shift+ArrowRight"], "ab", "1ab"]]) {
      await evaluate(`(()=>{const e=document.querySelector(${JSON.stringify(selector)});e.focus();e.setSelectionRange(e.value.length,e.value.length);return true})()`);
      const focused = await next(600);
      for (const key of keys) await input(focused.sequence, { kind: "key", key });
      await input(focused.sequence, { kind: "text", text: typed });
      assert.equal(await evaluate(`document.querySelector(${JSON.stringify(selector)}).value`), expected, `MP-08: ${keys.join(" ")} then "${typed}"`);
    }
  }));

// The mirrored sheets in document order (snapshot style records, then css ops replacing their text),
// and the colors they give `ids` in a fresh document of the source browser, beside the source's own.
const mirroredCss = packets => { const css = new Map(); for (const packet of packets) { let id = 0; for (const row of packet.nodes ?? []) { id += row[0]; if (row[2] === "style") css.set(`n${id}`, row[4]?.css ?? ""); } for (const op of packet.ops) if (op.op === "css") css.set(op.id, op.css); } return [...css.values()].join("\n"); };
const colors = async (evaluate, ids, css) => {
  const read = `${JSON.stringify(ids)}.map(id=>getComputedStyle(d.getElementById(id)).color)`;
  const source = await evaluate(`(()=>{const d=document;return ${read}})()`);
  const mirror = await evaluate(`new Promise(resolve=>{const f=document.createElement("iframe");f.srcdoc="<!doctype html>"+${JSON.stringify(ids.map(id => `<p id="${id}">${id}</p>`).join(""))};f.onload=()=>{const d=f.contentDocument,s=d.createElement("style");s.textContent=${JSON.stringify(css)};d.head.append(s);resolve(${read});f.remove()};document.body.append(f)})`);
  return { source, mirror };
};
const layered = '<link rel="stylesheet" href="/css/main.css"><p id="q">q</p><p id="r">r</p><p id="s">s</p><p id="u">u</p>';
const layeredSheets = (prefix = "") => ({
  [`${prefix}/css/main.css`]: { type: "text/css", headers: { "Access-Control-Allow-Origin": "*" }, body: '@import url("t/theme.css") layer(theme);\n@import url("t/anon.css") layer;\n@import url("t/sup.css") layer(x.y) supports(display: grid) screen;\np{color:rgb(0, 0, 255)}' },
  [`${prefix}/css/t/theme.css`]: { type: "text/css", headers: { "Access-Control-Allow-Origin": "*" }, body: "#q{color:rgb(255, 0, 0)}" },
  [`${prefix}/css/t/anon.css`]: { type: "text/css", headers: { "Access-Control-Allow-Origin": "*" }, body: "#r{color:rgb(255, 0, 0)}" },
  [`${prefix}/css/t/sup.css`]: { type: "text/css", headers: { "Access-Control-Allow-Origin": "*" }, body: "#s{color:rgb(255, 0, 0)}#u{color:rgb(0, 128, 0) !important}" },
});
test("MP-10: review #941-3 readable @import keeps its cascade layer (named, anonymous) and supports/media conditions: viewer colors match the source", () => mirrored(
  layered, async ({ next, evaluate }) => {
    await new Promise(resolve => setTimeout(resolve, 300));
    const { source, mirror } = await colors(evaluate, ["q", "r", "s", "u"], mirroredCss([await next()]));
    assert.deepEqual(source, ["rgb(0, 0, 255)", "rgb(0, 0, 255)", "rgb(0, 0, 255)", "rgb(0, 128, 0)"], "MP-10: the fixture's layered rules lose to the unlayered one");
    assert.deepEqual(mirror, source, "MP-10: the mirrored CSS gives the same colors");
  }, layeredSheets()));

test("MP-10: review #941-4 a CDP-read cross-origin sheet keeps its declared layer order ahead of its imports (and their layers): viewer colors match the source", () => mirrored(
  '<link rel="stylesheet" href="http://localhost:PORT/cdn/main.css"><p id="q">q</p><p id="r">r</p>', async ({ next, evaluate }) => {
    await new Promise(resolve => setTimeout(resolve, 800));
    const packets = [await next()]; for (let i = 0; i < 4 && !mirroredCss(packets).includes("#r"); i++) packets.push(await next(1000));
    const { source, mirror } = await colors(evaluate, ["q", "r"], mirroredCss(packets));
    assert.deepEqual(source, ["rgb(0, 0, 255)", "rgb(0, 128, 0)"], "MP-10: the declared order a, b, c decides (not import order)");
    assert.deepEqual(mirror, source, "MP-10: the mirrored CSS gives the same colors");
  }, {
    "/cdn/main.css": { type: "text/css", body: '@layer a, b;\n@import url("b.css") layer(b);\n@import url("a.css") layer(a);\n@import url("c.css") layer(c);\n@layer d;\n@import url("d.css");' }, // Chrome ignores an @import after a later @layer statement
    "/cdn/d.css": { type: "text/css", body: "#q{color:rgb(255, 0, 0) !important}" },
    "/cdn/a.css": { type: "text/css", body: "#q{color:rgb(255, 0, 0)}#r{color:rgb(255, 0, 0)}" },
    "/cdn/b.css": { type: "text/css", body: "#q{color:rgb(0, 0, 255)}" },
    "/cdn/c.css": { type: "text/css", body: "#r{color:rgb(0, 128, 0)}" },
  }));

test("MP-10: review #941-5 CSSOM sheet.disabled toggles of unreadable cross-origin sheets reach the viewer (disable and enable)", () => mirrored(
  '<link id="a" rel="stylesheet" href="http://localhost:PORT/a.css"><link id="b" rel="stylesheet" href="http://localhost:PORT/b.css"><p>t</p>', async ({ next, evaluate }) => {
    for (let i = 0; i < 40 && !await evaluate("[...document.styleSheets].length === 2"); i++) await new Promise(resolve => setTimeout(resolve, 50));
    await evaluate("document.getElementById('b').sheet.disabled=true;true");
    await new Promise(resolve => setTimeout(resolve, 300));
    const snapshot = await next(); let id = 0; const styles = [];
    for (const row of snapshot.nodes) { id += row[0]; if (row[2] === "style") styles.push([`n${id}`, row[3]?.media ?? null]); }
    assert.deepEqual(styles.map(([, media]) => media), [null, "not all"], "MP-10: the snapshot carries the disabled sheet as media=not all");
    await evaluate("document.getElementById('a').sheet.disabled=true;document.getElementById('b').sheet.disabled=false;true");
    const ops = [];
    for (let i = 0; i < 4 && ops.length < 2; i++) ops.push(...(await next(1500)).ops.filter(op => op.op === "attr" && op.name === "media"));
    assert.deepEqual(ops.map(op => [op.id, op.value]).sort(), [[styles[0][0], "not all"], [styles[1][0], null]].sort());
  }, { "/a.css": { type: "text/css", body: "p{color:rgb(1,2,3)}" }, "/b.css": { type: "text/css", body: "p{background:rgb(4,5,6)}" } }));
