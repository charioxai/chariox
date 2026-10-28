// MP-08/MP-10/MP-11: DOM mirror v2 observer and service on real Chromium
// (credential-free local fixtures, the real host controller and CDP).
import test from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { inflateRawSync, deflateSync, constants as zlib } from "node:zlib";
import { createHash } from "node:crypto";
import { mkdtemp, rm, readFile } from "node:fs/promises";
import path from "node:path";
import { KernelBrowserHost } from "./kernel-browser-host.mjs";

assert(process.env.CHARIOX_MDACCESS_DRILL_ROOT, "MP-11: explicit disposable drill root required");


// mirrorregion: the lease a viewer would open for the page's first opaque (tile) region at this packet.
async function regionLease({ host, subscription_id, generation }, sequence) {
  const stream = host.mirror.streams.get(subscription_id);
  const region_id = [...stream.regionRecords].find(([, r]) => r.kind === "tile")[0];
  return host.mirror.v2.regions.resolve({ mirror_subscription_id: subscription_id, generation, document_id: stream.document_id, sequence, region_id }, stream.scope);
}
const settle = ms => new Promise(resolve => setTimeout(resolve, ms));
async function stayDom(next, ms, label) { const until = Date.now() + ms; let packet, tiles = 0; while (Date.now() < until) { packet = await next(300); tiles += packet.tiles.length; assert.equal(packet.fallback, undefined, label); } return { ...packet, tiles: packet.tiles, seenTiles: tiles }; }

async function mirrored(html, run, routes = {}) {
  const root = await mkdtemp(path.join(process.env.CHARIOX_MDACCESS_DRILL_ROOT, "mirror2-"));
  const server = createServer((request, response) => {
    const port = String(server.address().port), route = routes[request.url];
    if (route) { response.setHeader("Content-Type", route.type); for (const [name, value] of Object.entries(route.headers ?? {})) response.setHeader(name, value); const send = () => response.end(route.body.replaceAll("PORT", port)); if (route.delay_ms) setTimeout(send, route.delay_ms); else send(); return; }
    response.setHeader("Content-Type", "text/html"); response.end(`<!doctype html>${html.replaceAll("PORT", port)}`);
  });
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const host = new KernelBrowserHost(root);
  try {
    const opened = await host.request({ op: "open", url: `http://127.0.0.1:${server.address().port}/` });
    const evaluate = async expression => {
      const { connection, sessionId } = await host.browser.resolvePageTarget(host.tabs.get(opened.tab_id).target_id);
      const reply = await connection.send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true }, sessionId);
      assert(!reply.exceptionDetails, `MP-10: credential-free fixture evaluation failed: ${reply.exceptionDetails?.exception?.description}`);
      return reply.result.value;
    };
    // about:blank is "complete" too: wait for the fixture document itself.
    for (let i = 0; i < 400 && await evaluate("location.protocol === 'http:' && document.readyState === 'complete'").catch(() => false) !== true; i++) await new Promise(resolve => setTimeout(resolve, 25));
    await host.request({ op: "state" });
    const subscribe = async (extra = {}) => {
      const { subscription_id } = await host.request({ op: "mirror_subscribe", tab_id: opened.tab_id, generation: opened.generation, device_scale_factor: 1, wire: 2, ...extra });
      // Protocol 489: bodies may be deflated in the subscription's context (in order, fresh per reset).
      let applied = 0, context = Buffer.alloc(0);
      const next = async (wait_ms = 0, reset = false, extra = {}) => {
        let packet = await host.request({ op: "mirror_next", subscription_id, generation: opened.generation, after_sequence: reset ? 0 : applied, drift_nodes: [], wait_ms, ...extra }); applied = packet.sequence;
        if (packet.reset) context = Buffer.alloc(0);
        if (packet.encoding === "deflate") { context = Buffer.concat([context, Buffer.from(packet.packet_base64, "base64")]); const out = inflateRawSync(context, { finishFlush: zlib.Z_SYNC_FLUSH }); packet = { ...JSON.parse(out.subarray(out.length - packet.packet_bytes)), resources: packet.resources ?? [], tiles: packet.tiles ?? [] }; }
        return { ops: [], resources: [], tiles: [], ...packet };
      };
      const input = async (sequence, action) => host.request({ op: "input", tab_id: opened.tab_id, generation: opened.generation, document_id: host.tabs.get(opened.tab_id).document_id, input: { kind: "mirror", subscription_id, sequence, action } });
      return { next, input, subscription_id, generation: opened.generation };
    };
    const { next, input, subscription_id, generation } = await subscribe();
    await run({ next, evaluate, host, input, subscribe, subscription_id, generation, tab_id: opened.tab_id });
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

// github.com re-sets its "Go to file" input's type (and name) to the same value on every render.
test("MP-10: an input whose type is re-set to the same value keeps its node; a new type re-serializes", () => mirrored(
  '<input id="q" type="text" name="q"><button id="b">b</button>', async ({ next, evaluate }) => {
    const snapshot = await next(); assert.equal(snapshot.reset, true);
    const children = packet => packet.ops.filter(op => op.op === "children").reduce((n, op) => n + op.nodes.length, 0);
    for (let i = 0; i < 3; i++) {
      await evaluate("q.setAttribute('type','text');q.setAttribute('name','q');q.setAttribute('aria-expanded',String(" + (i % 2 === 0) + "))");
      const packet = await next(600);
      assert.equal(children(packet), 0, "MP-10: the same input stays the viewer's node: " + JSON.stringify(packet.ops));
    }
    await evaluate("q.setAttribute('type','search')");
    assert(children(await next(600)) > 0, "MP-10: a new input type still re-serializes the control");
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
test("MP-08/MP-11: typing reaches a field inside a mirrored cross-origin frame, password field included", () => mirrored(
  '<p>top</p><iframe src="http://localhost:PORT/form" style="width:400px;height:120px"></iframe>', async ({ next, input }) => {
    await new Promise(resolve => setTimeout(resolve, 500));
    const snapshot = await next();
    const fields = elements(snapshot).filter(([id, tag]) => tag === "input" && Number(id.slice(1)) >= 1e9);
    const [field] = fields.find(([, , attrs]) => attrs.id === "f") ?? [], [secret] = fields.find(([, , attrs]) => attrs.id === "p") ?? [];
    assert(field && secret, "MP-10: the frame's inputs are mirrored");
    await input(snapshot.sequence, { kind: "text", node_id: field, text: "hi" });
    const changed = await next(1000);
    assert.deepEqual(changed.ops.filter(op => op.op === "form").map(op => [op.id, op.form.value]), [[field, "hi"]]);
    // UX-first: what a person types is delivered to every field, a password field too.
    await input(changed.sequence, { kind: "text", node_id: secret, text: "x" });
    const typed = await next(1000);
    // The mirror shows the typed character masked ("•"), as it does for every password field.
    assert.deepEqual(typed.ops.filter(op => op.op === "form").map(op => [op.id, op.form.value]), [[secret, "\u2022"]]);
  }, {
    "/form": { type: "text/html", body: '<!doctype html><input id="f" type="text"><input id="p" type="password">' },
  }));

// Stripe Elements styles its card frame `transform: translate(0px)`: a translation moves nothing, so the owner's typing is
// admitted (a scaled or rotated frame would put the mirrored coordinates in the wrong place and stays refused).
for (const transform of ["translate(0px)", "translateZ(0)"]) {
  test(`MP-08/MP-11: typing into a cross-origin frame styled transform:${transform} reaches its field`, () => mirrored(
    `<p>top</p><iframe src="http://localhost:PORT/form" style="width:400px;height:120px;transform:${transform}"></iframe>`, async ({ next, input }) => {
      await new Promise(resolve => setTimeout(resolve, 500));
      const snapshot = await next();
      const [field] = elements(snapshot).filter(([id, tag, attrs]) => tag === "input" && Number(id.slice(1)) >= 1e9 && attrs.id === "f")[0] ?? [];
      assert(field, "MP-10: the frame's input is mirrored");
      await input(snapshot.sequence, { kind: "text", node_id: field, text: "hi" });
      const changed = await next(1000);
      assert.deepEqual(changed.ops.filter(op => op.op === "form").map(op => [op.id, op.form.value]), [[field, "hi"]]);
    }, { "/form": { type: "text/html", body: '<!doctype html><input id="f" type="text">' } }));
}

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

test("MP-08/MP-10/MP-11: user keys follow a page-created editor before its mirror packet arrives", () => mirrored(
  '<input id="a"><p id="x">page</p>', async ({ next, evaluate, input }) => {
    const snapshot = await next();
    await input(snapshot.sequence, { kind: "focus", node_id: elements(snapshot).find(([, tag, attrs]) => tag === "input" && attrs.id === "a")[0] });
    await evaluate("const b=document.createElement('input');b.id='b';document.body.append(b);b.focus()");
    const unseen = await next(600);
    assert(unseen.ops.some(op => op.op === "children"));
    // Input is the person's choice, not model-visible observation. A page-created
    // editor must accept keys while its DOM packet is still crossing the relay.
    await input(snapshot.sequence, { kind: "text", text: "label" });
    await input(snapshot.sequence, { kind: "key", key: "Enter" });
    assert.equal(await evaluate("document.querySelector('#b').value"), "label");
    await evaluate("document.querySelector('#b').type='search'");
    await next(600);
    await input(unseen.sequence, { kind: "text", text: "!" });
    assert.equal(await evaluate("document.querySelector('#b').value"), "label!");
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

// UX-first (r72): a person's click is delivered where they clicked; the native page decides who receives it (as in Chrome).
test("MP-11: a click on a node the native page covers with an overlay lands on the overlay, not a refusal", () => mirrored(
  '<button id="b" style="position:absolute;left:10px;top:10px;width:80px;height:30px">b</button><div id="o" style="position:fixed;inset:0"></div><script>window.hits=[];addEventListener("click",e=>hits.push(e.target.id),true)</script>', async ({ next, evaluate, input }) => {
    const snapshot = await next(), [button] = elements(snapshot).find(([, , attrs]) => attrs.id === "b");
    await input(snapshot.sequence, { kind: "click", node_id: button, x: 5, y: 5 });
    assert.deepEqual(await evaluate("hits"), ["o"]);
  }));

test("MP-11: a click inside a transformed same-origin frame is delivered at the frame's bounding box", () => mirrored(
  '<iframe id="f" style="position:absolute;left:20px;top:20px;width:200px;height:100px;border:0;transform:translate(7px,3px)" srcdoc="<button id=b style=\'width:80px;height:30px\' onclick=hit=(window.hit||0)+1>b</button>"></iframe>', async ({ next, evaluate, input }) => {
    await evaluate("new Promise(resolve => f.contentDocument?.getElementById('b') ? resolve(true) : f.addEventListener('load', () => resolve(true)))");
    const snapshot = await next(600), [button] = elements(snapshot).find(([, tag, attrs]) => tag === "button" && attrs.id === "b") ?? [];
    assert(button, "the frame's button is mirrored");
    await input(snapshot.sequence, { kind: "click", node_id: button, x: 5, y: 5 });
    assert.equal(await evaluate("f.contentWindow.hit"), 1);
  }));

test("MP-11 review r72-1: a click inside a scaled same-origin frame is mapped through the frame's scale", () => mirrored(
  '<iframe id="f" style="position:absolute;left:20px;top:20px;width:200px;height:100px;border:0;transform-origin:0 0;transform:scale(2)" srcdoc="<button id=b style=\'width:80px;height:30px\' onclick=hit=(window.hit||0)+1>b</button>"></iframe>', async ({ next, evaluate, input }) => {
    await evaluate("new Promise(resolve => f.contentDocument?.getElementById('b') ? resolve(true) : f.addEventListener('load', () => resolve(true)))");
    const snapshot = await next(600), [button] = elements(snapshot).find(([, tag, attrs]) => tag === "button" && attrs.id === "b") ?? [];
    assert(button, "the frame's button is mirrored");
    await input(snapshot.sequence, { kind: "click", node_id: button, x: 5, y: 5 });
    assert.equal(await evaluate("f.contentWindow.hit"), 1);
  }));

test("MP-11 review r72-2: a click on a node clipped by a native scroll pane (viewer scrolled ahead) reveals it in the pane and lands", () => mirrored(
  '<div id="pane" style="position:absolute;left:10px;top:50px;width:300px;height:100px;overflow:auto"><div style="height:300px"></div><button id="b" style="width:80px;height:30px">b</button></div><div id="under" style="position:absolute;left:10px;top:340px;width:300px;height:60px">under</div><script>window.hits=[];addEventListener("click",e=>hits.push(e.target.id),true)</script>', async ({ next, evaluate, input }) => {
    const snapshot = await next(), [button] = elements(snapshot).find(([, , attrs]) => attrs.id === "b");
    await input(snapshot.sequence, { kind: "click", node_id: button, x: 5, y: 5 });
    assert.deepEqual(await evaluate("hits"), ["b"]);
    assert(await evaluate("pane.scrollTop") > 0, "the pane scrolled the button into view");
  }));

test("MP-11 review r72-2: a click on a node below its own frame's viewport reveals it in the frame and lands", () => mirrored(
  '<iframe id="f" style="position:absolute;left:20px;top:20px;width:200px;height:100px;border:0" srcdoc="<div style=\'height:500px\'></div><button id=b style=\'width:80px;height:30px\' onclick=hit=(window.hit||0)+1>b</button>"></iframe><div id="under" style="position:absolute;left:20px;top:300px;width:300px;height:60px">under</div>', async ({ next, evaluate, input }) => {
    await evaluate("new Promise(resolve => f.contentDocument?.getElementById('b') ? resolve(true) : f.addEventListener('load', () => resolve(true)))");
    const snapshot = await next(600), [button] = elements(snapshot).find(([, tag, attrs]) => tag === "button" && attrs.id === "b") ?? [];
    assert(button, "the frame's button is mirrored");
    await input(snapshot.sequence, { kind: "click", node_id: button, x: 5, y: 5 });
    assert.equal(await evaluate("f.contentWindow.hit"), 1);
  }));

test("MP-11 review r74-1: a button already visible in a scaled, bordered scroll pane lands without scrolling the pane", () => mirrored(
  '<div id="pane" style="position:absolute;left:10px;top:50px;width:300px;height:100px;border:4px solid #888;overflow:auto;transform-origin:0 0;transform:scale(2)"><button id="b" style="position:absolute;left:200px;top:10px;width:60px;height:30px">b</button></div><script>window.hits=[];addEventListener("click",e=>hits.push(e.target.id),true)</script>', async ({ next, evaluate, input }) => {
    const snapshot = await next(), [button] = elements(snapshot).find(([, , attrs]) => attrs.id === "b");
    await input(snapshot.sequence, { kind: "click", node_id: button, x: 5, y: 5 });
    assert.deepEqual(await evaluate("hits"), ["b"]);
    assert.equal(await evaluate("pane.scrollTop + pane.scrollLeft"), 0, "an already visible control is not scrolled");
  }));

test("MP-11 review r74-1: a click on a frame's button while the frame sits below its scroll pane's visible part reveals the frame and lands in it", () => mirrored(
  '<div id="pane" style="position:absolute;left:20px;top:50px;width:300px;height:100px;overflow:auto"><div style="height:300px"></div><iframe id="f" style="width:200px;height:100px;border:0" srcdoc="<button id=b style=\'width:80px;height:30px\' onclick=hit=(window.hit||0)+1>b</button>"></iframe></div><div style="position:absolute;left:20px;top:340px;width:300px;height:80px" id="under">under</div><script>window.hits=[];addEventListener("click",e=>hits.push(e.target.id),true)</script>', async ({ next, evaluate, input }) => {
    await evaluate("new Promise(resolve => f.contentDocument?.getElementById('b') ? resolve(true) : f.addEventListener('load', () => resolve(true)))");
    const snapshot = await next(600), [button] = elements(snapshot).find(([, tag, attrs]) => tag === "button" && attrs.id === "b") ?? [];
    assert(button, "the frame's button is mirrored");
    await input(snapshot.sequence, { kind: "click", node_id: button, x: 5, y: 5 });
    assert.deepEqual(await evaluate("hits"), [], "nothing under the clipped frame is clicked");
    assert.equal(await evaluate("f.contentWindow.hit"), 1);
    assert(await evaluate("pane.scrollTop") > 0, "the pane scrolled the frame into view");
  }));

test("MP-11 review r74-1: a click inside a separately mirrored frame clipped by its scroll pane reveals the frame, never lands on content under it", () => mirrored(
  '<div id="pane" style="position:absolute;left:20px;top:50px;width:300px;height:100px;overflow:auto"><div style="height:300px"></div><iframe id="f" src="http://localhost:PORT/btn" style="width:200px;height:100px;border:0"></iframe></div><div style="position:absolute;left:20px;top:340px;width:300px;height:80px" id="under">under</div><script>window.hits=[];addEventListener("click",e=>hits.push(e.target.id),true)</script>', async ({ next, evaluate, input }) => {
    await new Promise(resolve => setTimeout(resolve, 500));
    const snapshot = await next(600), [button] = elements(snapshot).find(([id, tag, attrs]) => tag === "button" && attrs.id === "b" && Number(id.slice(1)) >= 1e9) ?? [];
    assert(button, "the frame's button is mirrored");
    await input(snapshot.sequence, { kind: "click", node_id: button, x: 5, y: 5 });
    assert.deepEqual(await evaluate("hits"), [], "nothing under the clipped frame is clicked");
    assert(await evaluate("pane.scrollTop") > 0, "the pane scrolled the frame into view");
  }, {
    "/btn": { type: "text/html", body: '<!doctype html><button id="b" style="width:80px;height:30px">b</button>' },
  }));

test("MP-11: a click on a node that lost its box since the viewer's frame lands on its nearest boxed ancestor", () => mirrored(
  '<div id="p" style="position:absolute;left:200px;top:100px;width:100px;height:40px;background:#ccc">box<span id="z" style="display:contents"></span></div><script>window.hits=[];addEventListener("click",e=>hits.push(e.target.id),true)</script>', async ({ next, evaluate, input }) => {
    const snapshot = await next(), [collapsed] = elements(snapshot).find(([, , attrs]) => attrs.id === "z");
    await input(snapshot.sequence, { kind: "click", node_id: collapsed, x: 1, y: 1 });
    assert.deepEqual(await evaluate("hits"), ["p"]);
  }));

test("MP-10: a click on a node below the native viewport (the viewer scrolled ahead) reveals it natively and lands, not a refusal", () => mirrored(
  '<div style="height:3000px"></div><button id="b" style="width:80px;height:30px">b</button><script>window.hits=[];addEventListener("click",e=>hits.push(e.target.id),true)</script>', async ({ next, evaluate, input }) => {
    const snapshot = await next(), [button] = elements(snapshot).find(([, , attrs]) => attrs.id === "b");
    assert.equal(await evaluate("scrollY"), 0);
    await input(snapshot.sequence, { kind: "click", node_id: button, x: 5, y: 5 });
    assert.deepEqual(await evaluate("hits"), ["b"]);
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
test("MP-11: review #941-1(P1) a morphed child-frame node keeps click identity while user keys follow live focus", () => mirrored(
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
    await input(snapshot.sequence, { kind: "key", key: "Enter" });
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

// Adopted sheets per root (document, shadow) from snapshot rows and adopted ops; their colors in a fresh document of the source browser.
const adoptedRoots = packet => { const out = new Map(); let id = 0; for (const row of packet.nodes) { id += row[0]; if (row[2] === 1 || row[2] === 2) out.set(`n${id}`, row[4]?.adopted ?? []); } return out; };
const adoptedColors = (evaluate, [documentSheets, shadowSheets]) => evaluate(`new Promise(resolve=>{const f=document.createElement("iframe");f.srcdoc='<!doctype html><p id="d">d</p><div id="h"></div>';f.onload=()=>{const d=f.contentDocument,w=f.contentWindow,sheets=texts=>texts.map(t=>{const s=new w.CSSStyleSheet();s.replaceSync(t);return s}),r=d.getElementById("h").attachShadow({mode:"open"});r.innerHTML="<p>s</p>";d.adoptedStyleSheets=sheets(${JSON.stringify(documentSheets)});r.adoptedStyleSheets=sheets(${JSON.stringify(shadowSheets)});resolve([getComputedStyle(d.getElementById("d")).color,getComputedStyle(r.firstChild).color]);f.remove()};document.body.append(f)})`);
test("MP-10: review #941-3 a disabled adopted stylesheet stays inactive in the viewer (snapshot, disable, enable; document and shadow root): viewer colors match the source", () => mirrored(
  '<p id="d">doc</p><div id="h"></div><script>const mk=t=>{const s=new CSSStyleSheet();s.replaceSync(t);return s};window.base=mk("p{color:rgb(0, 0, 255)}");window.theme=mk("p{color:rgb(255, 0, 0)}");theme.disabled=true;document.adoptedStyleSheets=[base,theme];const r=document.getElementById("h").attachShadow({mode:"open"});r.innerHTML="<p>s</p>";window.ibase=mk("p{color:rgb(0, 128, 0)}");window.itheme=mk("p{color:rgb(255, 0, 0)}");itheme.disabled=true;r.adoptedStyleSheets=[ibase,itheme]</script>', async ({ next, evaluate }) => {
    const roots = adoptedRoots(await next());
    const source = () => evaluate('[getComputedStyle(document.getElementById("d")).color,getComputedStyle(document.getElementById("h").shadowRoot.firstChild).color]');
    assert.deepEqual(await source(), ["rgb(0, 0, 255)", "rgb(0, 128, 0)"]);
    assert.deepEqual(await adoptedColors(evaluate, [...roots.values()]), await source(), "MP-10: sheets disabled before the snapshot are inactive in the viewer");
    // The page enables both themes and disables the shadow root's base sheet.
    await evaluate("theme.disabled=false;ibase.disabled=true;itheme.disabled=false;true");
    const ops = [];
    for (let i = 0; i < 4 && ops.length < 2; i++) ops.push(...(await next(1500)).ops.filter(op => op.op === "adopted"));
    for (const op of ops) roots.set(op.id, op.sheets);
    assert.deepEqual(await source(), ["rgb(255, 0, 0)", "rgb(255, 0, 0)"]);
    assert.deepEqual(await adoptedColors(evaluate, [...roots.values()]), await source(), "MP-10: disable and enable toggles reach the viewer");
  }));

// Compact element rows -> { id, tag, attrs, ...extra } (snapshot nodes or a children op's nodes).
const rows = nodes => { const out = []; let id = 0; for (const row of nodes) { id += row[0]; if (typeof row[2] === "string") out.push({ id: `n${id}`, tag: row[2], attrs: row[3] || {}, ...(row[4] || {}) }); } return out; };
test("MP-10: review #941-2 every selected option of a multiple select reaches the viewer (snapshot, a secondary option changing, replaced options)", () => mirrored(
  '<select id="m" multiple><option>a</option><option selected>b</option><option selected>c</option></select><select id="one"><option>x</option><option>y</option></select><button id="b">b</button>', async ({ next, evaluate }) => {
    const all = rows((await next()).nodes), options = all.filter(r => r.tag === "option");
    assert.deepEqual(options.map(r => r.form?.checked), [false, true, true, undefined, undefined], "MP-10: each option of the multiple select carries its selectedness (single selects need none)");
    // A secondary option is deselected by script while the first selected one stays (value/selectedIndex unchanged).
    await evaluate("document.querySelector('#b').focus();m.options[2].selected=false;true");
    const forms = [];
    for (let i = 0; i < 3 && !forms.length; i++) forms.push(...(await next(1500)).ops.filter(op => op.op === "form"));
    assert.deepEqual(forms.map(op => [op.id, op.form.checked]), [[options[2].id, false]]);
    await evaluate("m.innerHTML='<option>x</option><option selected>y</option><option selected>z</option>';true");
    let children = null;
    for (let i = 0; i < 3 && !children; i++) children = (await next(1500)).ops.find(op => op.op === "children");
    assert.deepEqual(rows(children.nodes).map(r => r.form?.checked), [false, true, true], "MP-10: replaced options carry their selectedness");
  }));

test("MP-08: review #941-1 a text control's own range (select-all, a drag) set through mirror input is replaced by the next text (input and textarea, backward); out-of-range refused", () => mirrored(
  '<input id="i" value="hello world"><textarea id="t">one two\nthree</textarea>', async ({ next, evaluate, input }) => {
    const snapshot = await next(), id = name => elements(snapshot).find(([, , attrs]) => attrs.id === name)[0];
    for (const [name, anchor, focus, direction, typed, expected] of [["i", 0, 11, "forward", "x", "x"], ["t", 13, 4, "backward", "2", "one 2"]]) {
      await evaluate(`(()=>{const e=document.getElementById("${name}");e.focus();e.setSelectionRange(e.value.length,e.value.length);return true})()`);
      const focused = await next(600);
      await input(focused.sequence, { kind: "selection", anchor_id: id(name), anchor_offset: anchor, focus_id: id(name), focus_offset: focus });
      assert.deepEqual(await evaluate(`(e=>[e.selectionStart,e.selectionEnd,e.selectionDirection])(document.getElementById("${name}"))`), [Math.min(anchor, focus), Math.max(anchor, focus), direction]);
      await input(focused.sequence, { kind: "text", text: typed });
      assert.equal(await evaluate(`document.getElementById("${name}").value`), expected, `MP-08: the range of #${name} is replaced`);
    }
    await assert.rejects(input(snapshot.sequence, { kind: "selection", anchor_id: id("t"), anchor_offset: 0, focus_id: id("t"), focus_offset: 99 }), "MP-11: an offset beyond the value is refused");
  }));

test("MP-08: review #320-2 modified deletion keys edit the focused control (Ctrl+Backspace, Ctrl+Delete, line deletion; input and textarea)", () => mirrored(
  '<input id="i" value="hello big world"><textarea id="t">one two\nthree</textarea>', async ({ next, evaluate, input }) => {
    await next();
    for (const [name, at, keys, expected] of [["i", 15, ["Ctrl+Backspace"], "hello big "], ["t", 4, ["Ctrl+Delete"], "one \nthree"], ["t", 10, ["Shift+Home", "Backspace"], "one \n"]]) {
      await evaluate(`(()=>{const e=document.getElementById("${name}");e.focus();e.setSelectionRange(${at},${at});return true})()`);
      const focused = await next(600);
      for (const key of keys) await input(focused.sequence, { kind: "key", key });
      assert.equal(await evaluate(`document.getElementById("${name}").value`), expected, `MP-08: ${keys.join(" ")} in #${name}`);
    }
  }));

test("MP-10: review #941-2 a select that becomes multiple (and back) keeps every selected option in the viewer after property-only selections", () => mirrored(
  '<select id="s"><option>a</option><option>b</option><option>c</option></select><button id="b">b</button>', async ({ next, evaluate }) => {
    // What the viewer records for #s: the newest select record, its options' selectedness (multiple) or its index.
    const forms = new Map(); let select = null, options = [];
    const absorb = nodes => { let current = null; for (const r of rows(nodes)) { if (r.form) forms.set(r.id, { ...r.form }); if (r.tag === "select" && r.attrs.id === "s") { select = r; options = []; current = r; } else if (r.tag === "option" && current) options.push(r.id); else current = null; } };
    const apply = packet => { if (packet.nodes) absorb(packet.nodes); for (const op of packet.ops) { if (op.op === "children") absorb(op.nodes); if (op.op === "form") forms.set(op.id, { ...forms.get(op.id), ...op.form }); if (op.op === "attr" && select && op.id === select.id) { if (op.value === null) delete select.attrs[op.name]; else select.attrs[op.name] = op.value; } } };
    const viewer = () => Object.hasOwn(select.attrs, "multiple") ? options.map(id => forms.get(id)?.checked ?? null) : forms.get(select.id)?.selected_index;
    const source = () => evaluate("s.multiple ? [...s.options].map(o => o.selected) : s.selectedIndex");
    apply(await next());
    assert.equal(viewer(), 0);
    // Property-only writes after each switch: no input or change event.
    for (const script of ["s.multiple=true;s.options[2].selected=true", "s.multiple=false;s.selectedIndex=1", "s.multiple=true;s.options[0].selected=true"]) {
      await evaluate(`document.querySelector('#b').focus();${script};true`);
      const expected = await source();
      for (let i = 0; i < 4 && JSON.stringify(viewer()) !== JSON.stringify(expected); i++) apply(await next(1500));
      assert.deepEqual(viewer(), expected, `MP-10: the viewer matches the source after ${script}`);
    }
  }));

test("MP-10: review #941-3 an adopted stylesheet keeps its media condition (snapshot nonmatching, later matching and nonmatching; document and shadow root): viewer colors match the source", () => mirrored(
  '<p id="d">doc</p><div id="h"></div><script>const mk=(t,media)=>{const s=new CSSStyleSheet({media});s.replaceSync(t);return s};window.theme=mk("p{color:rgb(255, 0, 0)}","print");document.adoptedStyleSheets=[mk("p{color:rgb(0, 0, 255)}"),theme];const r=document.getElementById("h").attachShadow({mode:"open"});r.innerHTML="<p>s</p>";window.itheme=mk("p{color:rgb(255, 0, 0)}","print");r.adoptedStyleSheets=[mk("p{color:rgb(0, 128, 0)}"),itheme]</script>', async ({ next, evaluate }) => {
    const roots = adoptedRoots(await next());
    const source = () => evaluate('[getComputedStyle(document.getElementById("d")).color,getComputedStyle(document.getElementById("h").shadowRoot.firstChild).color]');
    assert.deepEqual(await source(), ["rgb(0, 0, 255)", "rgb(0, 128, 0)"]);
    assert.deepEqual(await adoptedColors(evaluate, [...roots.values()]), await source(), "MP-10: print-only sheets are inactive in the viewer");
    for (const [script, expected] of [["theme.media.mediaText='screen';itheme.media.appendMedium('all')", ["rgb(255, 0, 0)", "rgb(255, 0, 0)"]], ["theme.media.mediaText='print';itheme.media.mediaText='(max-width: 10px)'", ["rgb(0, 0, 255)", "rgb(0, 128, 0)"]]]) {
      await evaluate(`${script};true`);
      assert.deepEqual(await source(), expected);
      const ops = [];
      for (let i = 0; i < 4 && ops.length < 2; i++) ops.push(...(await next(1500)).ops.filter(op => op.op === "adopted"));
      for (const op of ops) roots.set(op.id, op.sheets);
      assert.deepEqual(await adoptedColors(evaluate, [...roots.values()]), expected, `MP-10: the viewer follows ${script}`);
    }
  }));

test("MP-10: review #941-4 a text selection inside a mirrored same-origin child frame reaches every viewer (another viewer's selection, page script, clearing)", () => mirrored(
  '<p id="top">top words</p><iframe id="f" src="/child"></iframe>', async ({ evaluate, subscribe }) => {
    for (let i = 0; i < 80 && !await evaluate("!!document.getElementById('f').contentDocument?.getElementById('c')"); i++) await new Promise(resolve => setTimeout(resolve, 25));
    const a = await subscribe(), b = await subscribe();
    const snapshotA = await a.next(), idsA = texts(snapshotA), child = texts(await b.next()).get("child words here");
    // Viewer B's view of the kernel selection (headers travel only when they change).
    let seen = null;
    const follow = async expected => { for (let i = 0; i < 4 && JSON.stringify(seen) !== JSON.stringify(expected); i++) { const packet = await b.next(1000); if (Object.hasOwn(packet, "selection")) seen = packet.selection; } return seen; };
    await a.input(snapshotA.sequence, { kind: "selection", anchor_id: idsA.get("child words here"), anchor_offset: 0, focus_id: idsA.get("child words here"), focus_offset: 5 });
    assert.equal(await evaluate("String(document.getElementById('f').contentDocument.getSelection())"), "child");
    const selected = { anchor_id: child, anchor_offset: 0, focus_id: child, focus_offset: 5 };
    assert.deepEqual(await follow(selected), selected, "MP-10: viewer A's selection in the child reaches viewer B");
    await evaluate("document.getElementById('f').contentDocument.getSelection().removeAllRanges();true");
    assert.equal(await follow(null), null, "MP-10: its clearing reaches viewer B");
    await evaluate("(()=>{const d=document.getElementById('f').contentDocument,t=d.getElementById('c').firstChild;d.getSelection().setBaseAndExtent(t,11,t,6);return true})()");
    const scripted = { anchor_id: child, anchor_offset: 11, focus_id: child, focus_offset: 6 };
    assert.deepEqual(await follow(scripted), scripted, "MP-10: a page script's selection in the child reaches viewer B");
  }, { "/child": { type: "text/html", body: '<p id="c">child words here</p>' } }));

test("MP-08: review #320-1 a range sent for an unfocused text control focuses it, so the next text and keys edit that control, not the one focused before", () => mirrored(
  '<input id="a" value="first field"><input id="b" value="second field"><textarea id="t">third field</textarea>', async ({ next, evaluate, input }) => {
    const snapshot = await next(), id = name => elements(snapshot).find(([, , attrs]) => attrs.id === name)[0];
    const values = () => evaluate("['a','b','t'].map(id => document.getElementById(id).value)");
    for (const [name, anchor, focus, edit, expected] of [["b", 7, 12, { kind: "text", text: "box" }, ["first field", "second box", "third field"]], ["t", 11, 5, { kind: "key", key: "Backspace" }, ["first field", "second box", "third"]]]) {
      await evaluate("document.getElementById('a').focus();document.getElementById('a').setSelectionRange(11,11);true");
      const focused = await next(600);
      await input(focused.sequence, { kind: "selection", anchor_id: id(name), anchor_offset: anchor, focus_id: id(name), focus_offset: focus });
      assert.equal(await evaluate("document.activeElement.id"), name, `MP-08: #${name} takes the kernel's focus with its range`);
      await input(focused.sequence, edit);
      assert.deepEqual(await values(), expected, `MP-08: the edit replaced the range of #${name}; #a is unchanged`);
    }
  }));

test("MP-08: review #320-2 a printable key on the focused select runs the kernel's typeahead and echoes as a form op", () => mirrored(
  '<select id="s"><option>Apple</option><option>Banana</option><option>Cherry</option></select>', async ({ next, evaluate, input }) => {
    const snapshot = await next(), id = elements(snapshot).find(([, , attrs]) => attrs.id === "s")[0];
    await evaluate("document.getElementById('s').focus();true");
    const focused = await next(600);
    await input(focused.sequence, { kind: "key", key: "b" });
    assert.equal(await evaluate("document.getElementById('s').value"), "Banana");
    const forms = [];
    for (let i = 0; i < 3 && !forms.length; i++) forms.push(...(await next(1000)).ops.filter(op => op.op === "form" && op.id === id));
    assert.equal(forms[0]?.form.selected_index, 1, "MP-08: the viewer receives the kernel's typeahead result");
  }));

// Protocol 492 (audit 2026-10-10 §4.2): committed control state and focus transitions from the viewer.
const controls = '<select id="s"><option>Apple</option><option>Banana</option></select><input id="d" type="date"><input id="c" type="color"><input id="r" type="range"><select id="m" multiple><option>a</option><option>b</option></select><input id="t" value="text"><input id="cc" type="date" autocomplete="cc-exp"><select id="cm" autocomplete="cc-exp-month"><option>01</option><option>02</option></select><select id="off" disabled><option>x</option><option>y</option></select><script>window.events=[];for(const e of document.querySelectorAll("select,input"))for(const t of ["input","change"])e.addEventListener(t,ev=>events.push(ev.target.id+":"+t))</script>';
test("MP-08: protocol 492 `set` commits a viewer-picked control state in the kernel page (select, date, color, range, multiple option) with input+change; text, disabled and out-of-range refused; payment fields are the owner's to pick", () => mirrored(controls, async ({ next, evaluate, input }) => {
  const snapshot = await next(), id = name => elements(snapshot).find(([, , attrs]) => attrs.id === name)[0];
  const option = elements(snapshot).filter(([, tag]) => tag === "option")[3][0];
  for (const action of [{ node_id: id("s"), selected_index: 1 }, { node_id: id("d"), value: "2026-10-10" }, { node_id: id("c"), value: "#ff8800" }, { node_id: id("r"), value: "70" }, { node_id: option, checked: true }]) await input(snapshot.sequence, { kind: "set", ...action });
  assert.deepEqual(await evaluate("[s.value,d.value,c.value,r.value,[...m.selectedOptions].map(o=>o.text).join()]"), ["Banana", "2026-10-10", "#ff8800", "70", "b"]);
  assert.deepEqual(await evaluate("events"), ["s:input", "s:change", "d:input", "d:change", "c:input", "c:change", "r:input", "r:change", "m:input", "m:change"], "MP-08: the page's own handlers see the commit");
  for (const action of [{ node_id: id("t"), value: "typed" }, { node_id: id("s"), selected_index: 2 }, { node_id: id("s"), value: "Apple" }, { node_id: id("off"), selected_index: 1 }, { node_id: id("d"), value: "2026-10-11", checked: true }])
    await assert.rejects(input(snapshot.sequence, { kind: "set", ...action }), /refused|unavailable|protected|failed/, JSON.stringify(action));
  assert.deepEqual(await evaluate("[t.value,s.value,off.value]"), ["text", "Banana", "x"]);
  // r67 (review #1016 @5947a7dad P2): the owner's pick in a payment picker reaches the native control; nothing Vault-filled is involved.
  for (const action of [{ node_id: id("cc"), value: "2027-01" + "-01" }, { node_id: id("cm"), selected_index: 1 }]) await input(snapshot.sequence, { kind: "set", ...action });
  assert.deepEqual(await evaluate("[cc.value,cm.value]"), ["2027-01-01", "02"]);
  // The next packet carries the kernel's committed state (the viewer stays in sync even if it differed).
  const forms = []; for (let i = 0; i < 3 && forms.length < 2; i++) forms.push(...(await next(1000)).ops.filter(op => op.op === "form" && [id("s"), id("d")].includes(op.id)));
  assert.deepEqual(forms.map(op => op.form.selected_index ?? op.form.value).sort(), [1, "2026-10-10"].sort());
}));

test("MP-08: protocol 492 `blur` leaves the kernel's focus only while the viewer's control is still the live focus", () => mirrored('<input id="a" value="one"><input id="b" value="two">', async ({ next, evaluate, input }) => {
  await evaluate("a.focus();true");
  const snapshot = await next(), id = name => elements(snapshot).find(([, , attrs]) => attrs.id === name)[0];
  assert.equal(snapshot.focused, id("a"));
  await input(snapshot.sequence, { kind: "blur", node_id: id("a") });
  assert.equal(await evaluate("document.activeElement.localName"), "body", "MP-08: the viewer's blur reached the kernel");
  const blurred = await next(600);
  assert.equal(blurred.focused, null, "MP-08: no focused node travels back");
  await evaluate("b.focus();true");
  await input(blurred.sequence, { kind: "blur", node_id: id("a") });
  assert.equal(await evaluate("document.activeElement.id"), "b", "MP-08: a stale blur never takes focus from another control");
}));

// Audit 2026-10-10 §4.5: CDP CSS events replace stylesheet polling and its caps (GitHub's main sheet
// exceeds 4000 rules; insertRule into it never mirrored).
const bigSheets = '<style id="big"></style><p class="hit">text</p><div id="h"></div><script>document.getElementById("big").textContent=Array.from({length:5000},(_,i)=>".c"+i+"{color:rgb(1,1,1)}").join("");window.k=new CSSStyleSheet();k.replaceSync(Array.from({length:5000},(_,i)=>".d"+i+"{color:rgb(2,2,2)}").join(""));document.adoptedStyleSheets=[k];const r=document.getElementById("h").attachShadow({mode:"open"});r.innerHTML="<b>shadow</b>";r.adoptedStyleSheets=[k]</script>';
test("MP-10: CSSOM edits of sheets over 4000 rules (style element, adopted in document and shadow root) reach the viewer within one credit", () => mirrored(bigSheets, async ({ next, evaluate }) => {
  assert.equal((await next()).reset, true);
  await evaluate("document.getElementById('big').sheet.insertRule('.hit{color:rgb(9,8,7)}', 5000);k.insertRule('.hit{background:rgb(6,5,4)}', 5000);true");
  const started = Date.now(), packet = await next(2000), text = JSON.stringify(packet.ops) + JSON.stringify(packet.sheets ?? {});
  assert.match(text, /rgb\(9, 8, 7\)/, "MP-10: the style element's new rule travels");
  assert.match(text, /rgb\(6, 5, 4\)/, "MP-10: the adopted sheet's new rule travels");
  assert.equal(packet.ops.filter(op => op.op === "adopted").length, 2, "MP-10: both adopting roots are updated");
  assert(Date.now() - started < 1500, "MP-10: reported by the CSS agent, not a periodic check");
  // A rule's declaration edit (no rule count change) travels too.
  await evaluate("document.getElementById('big').sheet.cssRules[5000].style.color='rgb(3,4,5)';true");
  const edited = await next(2000);
  assert.match(JSON.stringify(edited.ops) + JSON.stringify(edited.sheets ?? {}), /rgb\(3, 4, 5\)/);
}));

// Audit 2026-10-10 §4.3 (protocol 492): UA state with no attribute footprint, the fragment, custom element definitions.
const uaState = '<style>:target{color:rgb(1,2,3)} x-late:not(:defined){display:none}</style><button popovertarget="p">open</button><div popover id="p">pop</div><dialog id="d">dialog</dialog><input id="i" type="checkbox"><x-late>late</x-late><x-never>never</x-never><p id="sec">section</p>';
test("MP-10: protocol 492 popover/modal/indeterminate/definition state and the fragment reach the viewer (state records and ops, fragment header, :target marker)", () => mirrored(uaState, async ({ next, evaluate }) => {
  const snapshot = await next(), records = []; let id = 0;
  for (const row of snapshot.nodes) { id += row[0]; records.push({ id: `n${id}`, tag: row[2], attrs: row[3] || {}, extra: row[4] || {} }); }
  const of = predicate => records.find(predicate)?.id;
  assert.deepEqual(records.find(r => r.tag === "x-never").extra.state, { defined: false }, "MP-10: an undefined custom element is marked");
  assert.equal(records.find(r => r.attrs.id === "p").extra.state, undefined, "MP-10: default state travels as nothing");
  assert.match(JSON.stringify(snapshot.nodes), /\[chariox-target\] \{ color: rgb\(1, 2, 3\); \}/, "MP-10: :target follows the fragment marker");
  assert.match(JSON.stringify(snapshot.nodes), /x-late:not\(:not\(\[chariox-undefined\]\)\) \{ display: none; \}/, "MP-10: :defined follows the viewer's undefined marker (review #320@90c1195fc-2)");
  await evaluate("document.getElementById('d').showModal();document.getElementById('p').showPopover();document.getElementById('i').indeterminate=true;customElements.define('x-late',class extends HTMLElement{});location.hash='sec';true");
  const states = new Map(); let fragment;
  for (let i = 0; i < 4 && (states.size < 4 || fragment === undefined); i++) { const packet = await next(2000); for (const op of packet.ops.filter(op => op.op === "state")) states.set(op.id, { ...states.get(op.id), ...op.state }); if (packet.fragment !== undefined) fragment = packet.fragment; }
  assert.deepEqual(Object.fromEntries(states), { [of(r => r.attrs.id === "p")]: { popover_open: true }, [of(r => r.attrs.id === "d")]: { modal: true }, [of(r => r.attrs.id === "i")]: { indeterminate: true }, [of(r => r.tag === "x-late")]: { defined: true } });
  assert.equal(fragment, "sec", "MP-10: the fragment travels in the header");
  await evaluate("document.getElementById('d').close();document.getElementById('p').hidePopover();true");
  const closed = new Map(); for (let i = 0; i < 3 && closed.size < 2; i++) for (const op of (await next(2000)).ops.filter(op => op.op === "state")) closed.set(op.id, op.state);
  assert.deepEqual(Object.fromEntries(closed), { [of(r => r.attrs.id === "p")]: { popover_open: false }, [of(r => r.attrs.id === "d")]: { modal: false } });
}));

// Audit 2026-10-10 §4.4: attribute denylist instead of the rendered/CSS-referenced allowlist.
const attributesPage = '<x-card id="card"><h2 slot="title">Title</h2><p>body</p></x-card><form><input id="v" required pattern="[a-z]+" minlength="2" maxlength="5" step="2" list="l" data-small="ok" data-big="' + "x".repeat(2000) + '" data-csrf-token="t0k3n" csrf="t0k3n" onclick="x()"><datalist id="l"><option value="ab"></option></datalist><input id="img" type="image" alt="Go"><input id="mo" type="month"><input id="fi" type="file"></form><script>document.getElementById("card").attachShadow({mode:"open"}).innerHTML="<slot name=title></slot><slot></slot>"</script>';
test("MP-10: every attribute travels except the denied ones (named slots, constraint validation, data-*, input types); long values, credential-like names and handlers do not", () => mirrored(attributesPage, async ({ next, evaluate }) => {
  const snapshot = await next(), rows = elements(snapshot);
  const attrs = name => rows.find(([, , a]) => a.id === name)?.[2];
  assert.deepEqual(rows.find(([, tag, a]) => tag === "slot" && a.name)?.[2], { name: "title" }, "MP-10: a named slot keeps its name");
  assert.deepEqual(attrs("v"), { id: "v", required: "", pattern: "[a-z]+", minlength: "2", maxlength: "5", step: "2", list: "l", "data-small": "ok" });
  assert.deepEqual([attrs("img").type, attrs("mo").type, attrs("fi").type], ["image", "month", "file"], "MP-10: input types are kept");
  await evaluate("document.getElementById('v').setAttribute('data-state','open');document.getElementById('v').setAttribute('data-other-token','x');true");
  const ops = (await next(1000)).ops.filter(op => op.op === "attr");
  assert.deepEqual(ops.map(op => [op.name, op.value]), [["data-state", "open"]], "MP-10: attribute changes travel; credential-like names stay");
}));

// GATE-bytes r49: GitHub directory rows carry 64 KiB commit-message titles; eleven of them tripled its first paint.
test("MP-08: a long title travels as a 1 KB tooltip, in the snapshot and in later changes", () => mirrored('<a id="a" title="' + "t".repeat(65536) + '">a</a><a id="b" title="short">b</a>', async ({ next, evaluate }) => {
  const rows = elements(await next()), title = id => rows.find(([, , a]) => a.id === id)?.[2].title;
  assert.deepEqual([title("a"), title("b")], ["t".repeat(1024), "short"]);
  await evaluate("document.getElementById('b').title='u'.repeat(65536);true");
  const ops = (await next(1000)).ops.filter(op => op.op === "attr");
  assert.deepEqual(ops.map(op => [op.name, op.value]), [["title", "u".repeat(1024)]]);
}));

// Audit 2026-10-10 §4.8: packets whose replies were lost (a dropped socket) are sent again instead of a gap and a reset.
test("MP-10: a resend credit that still names an older base after a packet's reply was lost gets that packet again (original bytes, in order)", () => mirrored('<p id="x">one</p>', async ({ next, evaluate, host }) => {
  const first = await next();
  assert.equal(first.reset, true);
  const subscription_id = [...host.mirror.streams.keys()][0], generation = host.generation;
  const credit = (after, resend) => host.request({ op: "mirror_next", subscription_id, generation, after_sequence: after, drift_nodes: [], wait_ms: 1000, ...(resend ? { resend: true } : {}) });
  // Two packets are issued but their replies never reach the viewer (it still names sequence 1).
  await evaluate("x.firstChild.data='two';true"); const lost2 = await credit(first.sequence);
  await evaluate("x.firstChild.data='three';true"); const lost3 = await credit(first.sequence);
  assert.deepEqual([lost2.sequence, lost3.sequence], [2, 3]);
  await new Promise(resolve => setTimeout(resolve, 1700));
  await evaluate("x.firstChild.data='four';true");
  // Packet age never establishes loss; the viewer's explicit resend credits (protocol 501) ask for each packet after its base.
  const again = [await credit(first.sequence, true), await credit(lost2.sequence, true)];
  assert.deepEqual(again, [lost2, lost3], "MP-10: the lost packets come back byte for byte, in order");
  assert.equal((await credit(3)).sequence, 4, "MP-10: then the stream continues");
}));

// Audit 2026-10-10 §4.6: observer instance eviction and resource-key budget never leave a viewer silently stale.
test("MP-10: a subscription whose observer instance was evicted resyncs (reset with the change) instead of receiving empty deltas", () => mirrored('<p id="x">one</p>', async ({ next, evaluate, host }) => {
  assert.equal((await next()).reset, true);
  const stream = [...host.mirror.streams.values()][0];
  await host.mirror.v2.evaluate(stream.world, "(()=>{for(let i=0;i<20;i++)globalThis.__charioxMirror2.get('leaked-'+i);return true})()");
  await evaluate("x.firstChild.data='changed';true");
  let packet = await next(1000);
  for (let i = 0; i < 3 && !JSON.stringify(packet).includes("changed"); i++) packet = await next(1000);
  assert.equal(packet.reset, true, "MP-10: the evicted subscription takes a fresh snapshot");
  assert.match(JSON.stringify(packet.nodes), /changed/);
}));

// MP-08/MP-10: a lazy image far below the view has no bytes yet, but it is an image: Chromium lays it out by its
// width/height, never as its alt text. Its source travels as a key, so the viewer keeps it an image.
test("MP-10: a lazy image that has not loaded yet still travels with its source key", () => mirrored(
  `<p>top</p><div style="height:6000px"></div><img id="lazy" loading="lazy" src="/lazy.png" width="30" height="40" alt="A long alternative text">`, async ({ next, evaluate }) => {
    assert.equal(await evaluate("document.getElementById('lazy').currentSrc"), "", "MP-10: the fixture image has not started loading");
    const image = (await next()).nodes.find(row => row[2] === "img");
    assert.match(image?.[4]?.res ?? "", /^r[0-9]+$/, "MP-10: the pending image has a source key");
  }));

// MP-08/MP-10: currentSrc changes on load without a src/srcset mutation.
for (const picture of [false, true]) test(`MP-10: a lazy ${picture ? 'picture' : 'srcset image'} updates its key to the selected resource`, () => mirrored(
  `<p>top</p><div style="height:6000px"></div>${picture ? '<picture><source srcset="/candidate.svg">' : ''}<img id="lazy" loading="lazy" src="/fallback.svg" ${picture ? '' : 'srcset="/candidate.svg 1x, /retina.svg 2x"'} width="30" height="40" alt="lazy">${picture ? '</picture>' : ''}`, async ({ next, evaluate }) => {
    assert.equal(await evaluate("document.getElementById('lazy').currentSrc"), "");
    const snapshot = await next(), image = rows(snapshot.nodes).find(row => row.attrs?.id === 'lazy');
    let key = image.res, delivered = [...snapshot.resources];
    await evaluate("document.getElementById('lazy').scrollIntoView(); true");
    for (let i = 0; i < 20; i++) {
      const packet = await next(500);
      for (const op of packet.ops) if (op.op === 'res' && op.id === image.id) key = op.res;
      delivered.push(...packet.resources);
      if (delivered.some(resource => resource.key === key && Buffer.from(resource.data_base64, 'base64').toString().includes('#c0ffee'))) break;
    }
    assert.match(await evaluate("document.getElementById('lazy').currentSrc"), /candidate\.svg$/);
    assert(delivered.some(resource => resource.key === key && Buffer.from(resource.data_base64, 'base64').toString().includes('#c0ffee')), 'MP-10: the mirrored image key has the selected candidate bytes');
  }, { '/candidate.svg': { type: 'image/svg+xml', body: '<svg xmlns="http://www.w3.org/2000/svg" width="30" height="40"><path fill="#c0ffee" d="M0 0h30v40H0z"/></svg>' } }));

test("MP-10: past the resource-key budget the least recently used URL mapping goes; new images keep getting keys", () => mirrored(
  `<div id="g"></div><script>const g=document.getElementById("g");for(let i=0;i<4200;i++){const img=document.createElement("img");img.width=1;img.height=1;img.src="data:image/gif;base64,R0lGODlhAQABAAAAACw=#"+i;g.append(img)}</script>`, async ({ next }) => {
    const snapshot = await next(), images = snapshot.nodes.filter(row => row[2] === "img");
    assert.equal(images.length, 4200);
    assert.equal(images.filter(row => typeof row[4]?.res === "string").length, 4200, "MP-10: every image has a key");
    assert.equal(new Set(images.map(row => row[4].res)).size, 4200, "MP-10: keys are never reused");
  }));

// browserperf r10: every record of a nested-generator walk passed through each enclosing level, so a deep page cost
// time in proportion to records times depth (the walk is now one explicit stack).
test("MP-08/MP-10: a snapshot of a deeply nested page costs about what a flat one does", () => mirrored(
  `<div id="deep"></div><div id="flat"></div><script>let d=document.getElementById("deep");for(let i=0;i<400;i++){const c=document.createElement("div");d.append(c);d=c}for(let i=0;i<6000;i++)d.append(document.createElement("b"));const f=document.getElementById("flat");for(let i=0;i<6000;i++)f.append(document.createElement("i"))</script>`, async ({ next, host }) => {
    assert.equal((await next()).reset, true);
    const stream = [...host.mirror.streams.values()][0], time = away => host.mirror.v2.evaluate(stream.world, `(() => { const away = document.getElementById(${JSON.stringify(away)}); away.remove(); const t = performance.now(); ${stream.world.ref}.snapshot(); const ms = performance.now() - t; document.body.append(away); return ms; })()`);
    const deep = [], flat = [];
    for (let i = 0; i < 3; i++) { flat.push(await time("deep")); deep.push(await time("flat")); }
    const median = list => list.sort((a, b) => a - b)[1];
    assert(median(deep) < 3 * median(flat) + 20, `deep ${median(deep).toFixed(1)} ms vs flat ${median(flat).toFixed(1)} ms`);
  }));

// #1073 review: a URL evicted from the key budget keeps its key reserved, so a later URL whose hash lands on it gets another.
const urlKeyNumber = url => { let n = 2166136261; for (let i = 0; i < url.length; i++) n = Math.imul(n ^ url.charCodeAt(i), 16777619); return (n >>> 0) % 999999 + 1; };
const GIF = "data:image/gif;base64,R0lGODlhAQABAAAAACw=#k";
const [evicted, colliding] = (() => { const seen = new Map(); for (let i = 0; ; i++) { const n = urlKeyNumber(GIF + i); if (seen.has(n)) return [seen.get(n), i]; seen.set(n, i); } })();
test("MP-10: a URL whose key hash matches an evicted URL's key gets its own key while the evicted image is still mirrored", () => mirrored(
  `<div id="g"></div><script>const g=document.getElementById("g"),add=k=>{const img=document.createElement("img");img.width=1;img.height=1;img.src="${GIF}"+k;g.append(img)};add(${evicted});for(let i=0;i<4200;i++)add("f"+i);add(${colliding})</script>`, async ({ next }) => {
    const snapshot = await next(), images = snapshot.nodes.filter(row => row[2] === "img");
    assert.equal(images.length, 4202);
    const [first, last] = [images[0][4].res, images.at(-1)[4].res];
    assert.notEqual(first, last, "MP-10: the evicted image's key is never handed to another URL in the same document");
  }));

// Audit 2026-10-10 §4.7: opaque regions as element pixels, not viewport screenshots (R8-3, item 15).
const { decodePng } = await import("./kernel-browser-pixels.mjs");
const tilePixels = tile => { const png = Buffer.from(tile.data_base64, "base64"), width = png.readUInt32BE(16), height = png.readUInt32BE(20), { pixels } = decodePng(tile.data_base64, 1, { width, height }); let red = 0, blue = 0; for (let i = 0; i < pixels.length; i += 4) { if (pixels[i] > 200 && pixels[i + 2] < 60) red++; if (pixels[i + 2] > 200 && pixels[i] < 60) blue++; } return { width, height, red, blue }; };
const canvasPage = '<div style="position:relative"><canvas id="c" width="300" height="150"></canvas><div style="position:absolute;left:0;top:0;width:300px;height:40px;background:rgb(0,0,255)">controls</div></div>' +
  '<div style="transform:scale(0.5);transform-origin:0 0;width:400px;height:200px"><canvas id="t" width="200" height="100" style="width:200px;height:100px"></canvas></div>' +
  '<script>for(const id of ["c","t"]){const g=document.getElementById(id).getContext("2d");g.fillStyle="rgb(255,0,0)";g.fillRect(0,0,400,400)}</script>';
test("MP-10: a canvas region is its own pixels: an overlay above it is not baked in, and under a scale transform it keeps its layout size", () => mirrored(canvasPage, async ({ next }) => {
  const snapshot = await next(), tiles = new Map(snapshot.tiles.map(tile => [tile.node_id, tile]));
  const records = []; let id = 0; for (const row of snapshot.nodes) { id += row[0]; if (row[2] === 5) records.push({ id: `n${id}`, ...(row[4] ?? {}), attrs: row[3] }); }
  const [overlaid, scaled] = ["c", "t"].map(name => records.find(r => r.attrs?.id === name));
  const a = tilePixels(tiles.get(overlaid.id)), b = tilePixels(tiles.get(scaled.id));
  assert.equal(a.blue, 0, "MP-10: the overlay is mirrored DOM, not pixels of the canvas");
  assert(a.red > 0.9 * a.width * a.height, "MP-10: the canvas bitmap travels");
  assert.deepEqual(scaled.size, [200, 100], "MP-10: the region keeps its layout size (the viewer applies the transform)");
  assert.deepEqual([tiles.get(scaled.id).width, tiles.get(scaled.id).height, b.red === b.width * b.height], [200, 100, true]);
}));

test("MP-10: a tainted canvas falls back to a screenshot crop; under a filter (doubly applied) the page stays DOM and the region gets a video lease", () => mirrored(
  '<div id="wrap"><canvas id="c" width="120" height="60"></canvas></div><script>const i=new Image();i.onload=()=>{document.getElementById("c").getContext("2d").drawImage(i,0,0);window.drawn=true};i.src="http://localhost:PORT/tile.svg"</script>', async ctx => {
    const { next, evaluate } = ctx;
    for (let i = 0; i < 100 && await evaluate("window.drawn === true") !== true; i++) await new Promise(resolve => setTimeout(resolve, 20));
    const plain = await next();
    assert.equal(plain.fallback, undefined); assert.equal(plain.tiles.length, 1, "MP-10: the tainted canvas still shows (screenshot crop)");
    await evaluate("document.getElementById('wrap').style.filter='grayscale(1)';true");
    await settle(1100);
    let crop = 0;
    const filtered = await next(1000); crop += filtered.tiles.length;
    const packet = await stayDom(next, 3500, "MP-10: a filtered region never hands the page to video");
    assert.equal(crop + packet.seenTiles, 0, "MP-10: a filtered region's screenshot already holds the filter, as a background it would be filtered again: no still");
    const lease = await regionLease(ctx, packet.sequence);
    assert.equal(lease.rect.width, 120); assert.equal(lease.rect.height, 60);
  }, { "/tile.svg": { type: "image/svg+xml", body: '<svg xmlns="http://www.w3.org/2000/svg" width="120" height="60"><rect width="120" height="60" fill="rgb(255,0,0)"/></svg>' } }));

test("MP-11: a Vault-filled plain field that changes its own display is re-serialized as a mask with the new display", () => mirrored('<form><input id="v" value="filled"><p>after</p></form>', async ({ next, evaluate, host }) => {
  const tab = [...host.tabs.values()][0], { connection, sessionId } = await host.browser.resolvePageTarget(tab.target_id);
  const { root } = await connection.send("DOM.getDocument", { depth: 0 }, sessionId), { nodeId } = await connection.send("DOM.querySelector", { nodeId: root.nodeId, selector: "#v" }, sessionId);
  const { node } = await connection.send("DOM.describeNode", { nodeId }, sessionId);
  await host.protect({ values: ["filled"], targets: [{ kind: "browser", target_id: tab.target_id, document_id: tab.document_id, node_ref: `backend:${node.backendNodeId}` }] });
  const masks = packet => { const out = []; let id = 0; for (const row of packet.nodes ?? []) { id += row[0]; if (row[2] === 4) out.push(row[4]); } for (const op of packet.ops ?? []) if (op.op === "children") { let at = 0; for (const row of op.nodes) { at += row[0]; if (row[2] === 4) out.push(row[4]); } } return out; };
  const first = await next(0, true);
  assert.deepEqual(masks(first).map(mask => mask.display), ["inline-block"]);
  await evaluate("document.getElementById('v').style.display='block';true");
  let packet = await next(1000); for (let i = 0; i < 3 && !masks(packet).length; i++) packet = await next(1000);
  assert.deepEqual(masks(packet).map(mask => mask.display), ["block"], "MP-11: the viewer's mask follows the field's display");
  assert.doesNotMatch(JSON.stringify(packet), /filled/, "MP-11: the protected value never travels");
}));

// Round 9 hosted finding: a focus whose page handler changes the field's attributes (aria-expanded) must not
// refuse the range sent with it (same viewer epoch); ordinary attribute churn only refuses clicks.
test("MP-08: focus, range and text sent together for a field whose focus handler changes its attributes all apply", () => mirrored(
  '<input id="b" value="zqxj wvkpq"><script>b.addEventListener("focus",()=>{b.setAttribute("aria-expanded","true");b.className="open"})</script>', async ({ next, evaluate, input }) => {
    const snapshot = await next(), id = elements(snapshot).find(([, , attrs]) => attrs.id === "b")[0];
    for (const action of [{ kind: "focus", node_id: id }, { kind: "selection", anchor_id: id, anchor_offset: 10, focus_id: id, focus_offset: 5 }, { kind: "text", text: "delta" }]) {
      await input(snapshot.sequence, action);
      if (action.kind === "focus") await next(600); // the attribute change reaches the kernel's stream (not the viewer)
    }
    assert.equal(await evaluate("b.value"), "zqxj delta");
  }));

test("MP-10: review #941 R8-2 an SVG image's changed href / xlink:href reaches the viewer as a resource update (external, fragment, removed)", () => mirrored(
  `<svg width="200" height="60"><defs><image id="sym" href="/a.svg" width="10" height="10"/></defs><image id="h" href="/a.svg" width="40" height="40"/><image id="x" xlink:href="/a.svg" x="50" width="40" height="40"/></svg>`, async ({ next, evaluate }) => {
    await new Promise(resolve => setTimeout(resolve, 300));
    const snapshot = await next(), ids = new Map(rows(snapshot.nodes).filter(r => r.tag === "image").map((r, i) => [["sym", "h", "x"][i], r.id]));
    await evaluate(`new Promise(resolve=>{const i=new Image();i.onload=()=>resolve(true);i.src='/b.svg'}).then(()=>{h.setAttribute('href','/b.svg');x.setAttributeNS('http://www.w3.org/1999/xlink','xlink:href','/b.svg');return true})`);
    const packets = []; for (let i = 0; i < 8 && !packets.some(p => p.resources.some(r => Buffer.from(r.data_base64, "base64").toString().includes("#0a0b0c"))); i++) packets.push(await next(1000));
    const ops = packets.flatMap(p => p.ops), bKey = packets.flatMap(p => p.resources).find(r => Buffer.from(r.data_base64, "base64").toString().includes("#0a0b0c"))?.key;
    assert(bKey, "MP-10: the new image's bytes arrived");
    for (const name of ["h", "x"]) {
      const mine = ops.filter(op => op.id === ids.get(name));
      assert.deepEqual(mine.filter(op => op.op === "res").map(op => op.res), [bKey], `MP-10: ${name} gets the new key (${JSON.stringify(mine)})`);
      assert(!mine.some(op => op.op === "attr" && op.name === "href" && op.value === null), `MP-10: ${name} keeps its image (no href removal)`);
    }
    // To a fragment, then removed: the key goes, the fragment travels as an attribute.
    await evaluate("h.setAttribute('href','#sym');x.removeAttributeNS('http://www.w3.org/1999/xlink','href');true");
    const later = (await next(1000)).ops;
    assert.deepEqual(later.filter(op => op.id === ids.get("h")).map(op => [op.op, op.op === "res" ? op.res : op.value]), [["res", null], ["attr", "#sym"]]);
    assert.deepEqual(later.filter(op => op.id === ids.get("x")).map(op => [op.op, op.op === "res" ? op.res : op.value]), [["res", null], ["attr", null]]);
  }, {
    "/a.svg": { type: "image/svg+xml", body: svg("#010203") },
    "/b.svg": { type: "image/svg+xml", body: svg("#0a0b0c") },
  }));

test("MP-10: residue 2.4-13 a collapsed selection (the caret) inside editable content reaches the viewer; outside editable content it stays null", () => mirrored(
  '<div id="ed" contenteditable="true">hello world</div><p id="p">plain text</p>', async ({ next, evaluate }) => {
    const snapshot = await next(), text = texts(snapshot);
    await evaluate("ed.focus();getSelection().collapse(ed.firstChild,5);true");
    let packet = await next(1000);
    assert.deepEqual(packet.selection, { anchor_id: text.get("hello world"), anchor_offset: 5, focus_id: text.get("hello world"), focus_offset: 5 }, "MP-10: the kernel's caret travels");
    await evaluate("getSelection().collapse(ed.firstChild,2);true");
    packet = await next(1000);
    assert.equal(packet.selection?.anchor_offset, 2, "MP-10: a moved caret travels");
    await evaluate("ed.blur();getSelection().collapse(p.firstChild,3);true");
    packet = await next(1000);
    assert.equal(packet.selection ?? null, null, "MP-10: a caret outside editable content is no selection");
  }));

test("MP-10: residue 2.5-16 an open shadow root attached late to an ordinary (non-custom) host reaches the viewer", () => mirrored(
  '<div id="h">light</div><p>after</p>', async ({ next, evaluate }) => {
    const snapshot = await next(), host = rows(snapshot.nodes).find(r => r.tag === "div" && r.attrs.id === "h").id;
    await evaluate("h.attachShadow({mode:'open'}).innerHTML='<b>shadow text</b><slot></slot>';true");
    const ops = []; for (let i = 0; i < 6 && !ops.some(op => op.op === "children" && op.id === host); i++) ops.push(...(await next(1000)).ops);
    const children = ops.find(op => op.op === "children" && op.id === host);
    assert(children, "MP-10: the host's children change");
    const shadow = rows(children.nodes).length ? children.nodes : [];
    assert(JSON.stringify(shadow).includes("shadow text") && children.nodes.some(row => row[2] === 2), `MP-10: the shadow root and its content travel (${JSON.stringify(children)})`);
  }));

test("MP-11: every Vault-filled non-password field is masked like the pixel path (month, date, tel, textarea); a filled password field is not a mask", () => mirrored(
  '<form><input id="m" type="month" value="2031-04"><input id="d" type="date" value="2031-04-30"><input id="t" type="tel" value="5550100"><textarea id="a">note1234</textarea><input id="p" type="password" value="pw998877"><p>after</p></form>', async ({ next, host }) => {
    const tab = [...host.tabs.values()][0], { connection, sessionId } = await host.browser.resolvePageTarget(tab.target_id);
    const { root } = await connection.send("DOM.getDocument", { depth: 0 }, sessionId), targets = [];
    for (const id of ["m", "d", "t", "a", "p"]) { const { nodeId } = await connection.send("DOM.querySelector", { nodeId: root.nodeId, selector: `#${id}` }, sessionId); const { node } = await connection.send("DOM.describeNode", { nodeId }, sessionId); targets.push({ kind: "browser", target_id: tab.target_id, document_id: tab.document_id, node_ref: `backend:${node.backendNodeId}` }); }
    await host.protect({ values: ["2031-04", "2031-04-30", "5550100", "note1234", "pw998877"], targets });
    const packet = await next(0, true), wire = JSON.stringify(packet);
    assert.equal(packet.nodes.filter(row => row[2] === 4).length, 4, "MP-11: month, date, tel and textarea are masks");
    for (const value of ["2031-04", "5550100", "note1234", "pw998877"]) assert(!wire.includes(value), `MP-11: ${value} never travels`);
    assert(rows(packet.nodes).some(r => r.tag === "input" && r.attrs.id === "p" && /^•+$/.test(r.form?.value)), "MP-11: the password field stays a field showing dots");
  }));

// Decision G (GitHub ref picker): the click's focus must not leave in a packet of its own ahead of the overlay the
// click renders a frame later; one packet carries both.
test("MP-10: a click that focuses a button and renders its overlay a frame later reaches the viewer as one packet", () => mirrored(
  '<button id="b">main</button><div id="host"></div><script>b.addEventListener("click",()=>setTimeout(()=>{b.setAttribute("aria-expanded","true");host.innerHTML="<div class=overlay><h2>Switch branches/tags</h2></div>"},17))</script>', async ({ next, input }) => {
    const snapshot = await next(), button = elements(snapshot).find(([, tag, attrs]) => tag === "button" && attrs.id === "b")[0];
    await input(snapshot.sequence, { kind: "click", node_id: button, x: 5, y: 5 });
    const first = await next(1500);
    assert.equal(first.focused, button, "MP-10: the focus travels");
    assert(first.ops.some(op => op.op === "children") && JSON.stringify(first.ops).includes("Switch branches/tags"), `MP-10: with the overlay in the same packet: ${JSON.stringify(first.ops).slice(0, 300)}`);
  }));

// MP-08/MP-10: focus emulation ahead of an input restores focus to the element the viewer already has. That focusin
// must not end the armed long poll (the re-armed poll would wait behind the input's own page task); a move does.
test("MP-08/MP-10: a focus event that leaves the reported focus unchanged keeps the long poll armed; a focus move ends it", () => mirrored(
  '<input id="a"><input id="b"><p id="p">one</p>', async ({ next, evaluate, host }) => {
    await evaluate("document.querySelector('#a').focus();true");
    const snapshot = await next(), ids = Object.fromEntries(elements(snapshot).filter(([, tag]) => tag === "input").map(([id, , attrs]) => [attrs.id, id]));
    assert.equal(snapshot.focused, ids.a);
    const connection = host.browser.connection, send = connection.send;
    let waits = 0;
    connection.send = (method, params, session) => { if (method === "Runtime.evaluate" && params?.expression?.includes(".waitDrain(")) waits++; return send.call(connection, method, params, session); };
    try {
      const pending = next(1500);
      await new Promise(resolve => setTimeout(resolve, 200));
      await evaluate("const a=document.querySelector('#a');a.blur();a.focus();true");
      await new Promise(resolve => setTimeout(resolve, 200));
      await evaluate("document.querySelector('#p').textContent='two';true");
      const packet = await pending;
      assert(JSON.stringify(packet.ops).includes("two"), "MP-10: the change leaves in the armed poll");
      assert.equal(waits, 1, "MP-08: the unchanged focus did not end and re-arm the long poll");
      const started = Date.now(), moving = next(1500);
      await evaluate("document.querySelector('#b').focus();true");
      assert.equal((await moving).focused, ids.b, "MP-10: a focus move still travels");
      assert(Date.now() - started < 1000, "MP-10: at once, not at the poll bound");
    } finally { connection.send = send; }
  }));

// Round 10 drop drill (DPR1, 3.3 s after reconnect): the web transport replays the credit that was in flight when the
// socket dropped; the kernel answers the replay with the original run, which was still long-polling.
test("MP-10: a credit replayed after a reconnect ends the original credit's long poll (answered now, not after its wait)", () => mirrored('<p id="x">one</p>', async ({ next, host, subscription_id, generation }) => {
  const first = await next();
  const credit = () => host.request({ op: "mirror_next", subscription_id, generation, after_sequence: first.sequence, drift_nodes: [], wait_ms: 2000, command_id: "credit-lost-with-socket" });
  const started = Date.now(), original = credit().catch(error => error);
  await new Promise(resolve => setTimeout(resolve, 100));
  const replay = await credit(); const replayMs = Date.now() - started;
  assert.ok(replayMs < 800, `MP-10: the replay answered in ${replayMs} ms`);
  assert.equal((await original).sequence, replay.sequence, "MP-10: the same packet as the original credit");
}));

// Review #941 @36a395b05 finding 3: `@layer theme; @import ...` starts with a layer statement, so the import flag
// (which routes imported-sheet CSS events to their importer) must look past leading layer statements.
test("MP-10: review #941@36a395b05-3 a same-count declaration edit inside a sheet imported after a leading @layer statement reaches the viewer (also nested)", () => mirrored(
  '<link rel="stylesheet" href="/l.css"><p id="q">q</p><p id="r">r</p>', async ({ next, evaluate }) => {
    await new Promise(resolve => setTimeout(resolve, 300));
    assert.equal((await next()).reset, true);
    const css = async needle => { for (let i = 0; i < 5; i++) for (const op of (await next(1000)).ops) if (op.op === "css" && op.css.includes(needle)) return true; return false; };
    await evaluate("document.styleSheets[0].cssRules[1].styleSheet.cssRules[2].style.color='rgb(1, 2, 3)';true");
    assert(await css("rgb(1, 2, 3)"), "MP-10: the imported declaration edit reaches the viewer");
    await evaluate("document.styleSheets[0].cssRules[1].styleSheet.cssRules[1].styleSheet.cssRules[0].style.color='rgb(4, 5, 6)';true");
    assert(await css("rgb(4, 5, 6)"), "MP-10: a nested imported declaration edit reaches the viewer");
  }, {
    "/l.css": { type: "text/css", body: '@layer theme, base;\n@import url("t.css") layer(theme);' },
    "/t.css": { type: "text/css", body: '@layer inner;\n@import url("n.css") layer(inner);\n#q{color:rgb(9, 9, 9)}' },
    "/n.css": { type: "text/css", body: "#r{color:rgb(8, 8, 8)}" },
  }));

// Review #941 @36a395b05 finding 6: `sheet.media.mediaText = ...` / `appendMedium` change a style or link sheet's
// effective condition without a mutation or CSS event; the viewer's copy must follow (readable and no-CORS sheets).
test("MP-10: review #941@36a395b05-6 CSSOM media-list edits of style and no-CORS link sheets reach the viewer (deltas and the next snapshot)", () => mirrored(
  '<style id="s">#q{color:rgb(9, 9, 9)}</style><link id="l" rel="stylesheet" href="http://localhost:PORT/x.css"><p id="q">q</p><p id="r">r</p>', async ({ next, evaluate }) => {
    await new Promise(resolve => setTimeout(resolve, 300));
    const [s, l] = elements(await next()).filter(([, tag]) => tag === "style").map(([id]) => id);
    const media = async (id, value) => { for (let i = 0; i < 4; i++) if ((await next(1000)).ops.some(op => op.op === "attr" && op.id === id && op.name === "media" && op.value === value)) return true; return false; };
    await evaluate("s.sheet.media.mediaText='print';true");
    assert(await media(s, "print"), "MP-10: the style sheet's new media list reaches the viewer");
    await evaluate("l.sheet.media.appendMedium('print');true");
    assert(await media(l, "print"), "MP-10: the no-CORS link sheet's appended medium reaches the viewer");
    const snapshot = await next(0, true);
    assert.deepEqual(elements(snapshot).filter(([, tag]) => tag === "style").map(([, , attrs]) => attrs.media), ["print", "print"], "MP-10: a snapshot serializes the effective media list");
    await evaluate("s.sheet.media.mediaText='';true");
    assert(await media(elements(snapshot).filter(([, tag]) => tag === "style")[0][0], null), "MP-10: clearing the media list reaches the viewer");
  }, { "/x.css": { type: "text/css", body: "#r{color:rgb(8, 8, 8)}" } }));

// r61 GitHub coverage: Chrome serializes `border:var(--w) solid; border-color:var(--c)` as empty longhands
// (`border-top-width: ;`) which a viewer parser drops: the borders of every such rule vanished (C_area 0.61). A linked sheet
// whose CSSOM text came out that way travels as its source text.
test("MP-10: a linked sheet whose CSSOM serialization drops var() shorthand longhands keeps its borders", () => mirrored(
  '<link rel="stylesheet" href="/gh.css"><div class="b" style="width:50px;height:20px">x</div>', async ({ next }) => {
    const packets = [await next()];
    for (let i = 0; i < 5; i++) packets.push(await next(500));
    const css = mirroredCss(packets);
    assert(css.includes("var(--w)"), `MP-10: the var() shorthand reaches the viewer: ${css.slice(0, 300)}`);
    assert(!/[;{]\s*(?!--)[a-z][a-z-]*:\s*;/.test(css), `MP-10: no empty longhand reaches the viewer: ${css.slice(0, 300)}`);
  }, { "/gh.css": { type: "text/css", body: ".b{--w:3px;--c:red;border:var(--w) solid;border-color:var(--c)}" } }));

test("MP-10: linked lossy CSS stops CSSOM serialization before unused suffix rules", () => mirrored(
  '<link rel="stylesheet" href="/gh.css"><p>fixture</p>', async ({ next, evaluate, host, subscription_id }) => {
    const initial=[await next()];for(let i=0;i<5;i++)initial.push(await next(500));
    const initialCss=mirroredCss(initial)+'\n'+initial.flatMap(packet=>Object.values(packet.sheets??{})).join('\n');
    assert(initialCss.includes('var(--w)'),`MP-10: source fallback preserves the lossy shorthand: ${initialCss.slice(0,300)}`);
    assert(initialCss.includes('suffix-marker-199'),`MP-10: source fallback preserves CSSOM suffix rules: ${initialCss.slice(-300)}`);
    const world=host.mirror.streams.get(subscription_id).world;
    await host.mirror.v2.evaluate(world, `(()=>{
      let prototype=CSSStyleRule.prototype,descriptor;
      while(prototype&&!descriptor){descriptor=Object.getOwnPropertyDescriptor(prototype,'cssText');prototype=Object.getPrototypeOf(prototype)}
      if(!descriptor?.get||!descriptor.configurable)throw Error('CSSStyleRule cssText getter unavailable');
      globalThis.__mirrorLossySuffixReads=0;
      Object.defineProperty(CSSStyleRule.prototype,'cssText',{configurable:true,enumerable:descriptor.enumerable,get(){if(this.selectorText==='.tail')globalThis.__mirrorLossySuffixReads++;return descriptor.get.call(this)}});
      return true;
    })()`);
    const reset=await next(0,true);
    assert.equal(reset.reset,true,'MP-10: the measured serialization is a real source reset');
    const suffixReads=await host.mirror.v2.evaluate(world,'globalThis.__mirrorLossySuffixReads');
    assert.equal(suffixReads,0,`MP-10: serialization stops at the first lossy rule; read ${suffixReads} suffix rules`);
  }, { "/gh.css": { type: "text/css", body: "p{border:var(--w) solid;border-color:red}" + Array.from({length:200},(_,i)=>`.tail{--suffix-marker-${i}:${i}}`).join("") + ".tail{--suffix-marker-199:199}" } }));

test("MP-08/MP-10: the next document reads a linked sheet the kernel read as source text the same way, borders kept", () => mirrored(
  '<link rel="stylesheet" href="/gh.css"><div class="b" style="width:50px;height:20px">a</div>', async ({ next, evaluate, host }) => {
    const first = [await next()];
    for (let i = 0; i < 5; i++) first.push(await next(500));
    assert(mirroredCss(first).includes("var(--w)"), "MP-10: the first document reads the lossy sheet as source text");
    await evaluate("location.href = '/b'").catch(() => {});
    for (let i = 0; i < 400 && await evaluate("location.pathname === '/b' && document.readyState === 'complete'").catch(() => false) !== true; i++) await settle(25);
    await host.request({ op: 'state' });
    let reset;
    for (let i = 0; i < 40 && reset === undefined; i++) { const packet = await next(300).catch(() => null); if (packet?.reset && packet.nodes.some(row => row[2] === 0 && row[3] === 'b')) reset = packet; }
    const packets = [reset];
    for (let i = 0; i < 5; i++) packets.push(await next(500));
    const css = mirroredCss(packets);
    assert(css.includes("var(--w)"), `MP-10: the next document's var() shorthand reaches the viewer: ${css.slice(0, 300)}`);
    assert(!/[;{]\s*(?!--)[a-z][a-z-]*:\s*;/.test(css), `MP-10: no empty longhand reaches the viewer: ${css.slice(0, 300)}`);
  }, {
    "/b": { type: "text/html", body: '<!doctype html><link rel="stylesheet" href="/gh.css"><div class="b" style="width:50px;height:20px">b</div>' },
    "/gh.css": { type: "text/css", body: ".b{--w:3px;--c:red;border:var(--w) solid;border-color:var(--c)}" },
  }));

// #1073 review: a linked sheet read as source text in one document is still read live in the next one, so a CSSOM
// edit that makes it serializable reaches the viewer.
test("MP-08/MP-10: a CSSOM edit of a linked sheet the previous document read as source text reaches the viewer", () => mirrored(
  '<link rel="stylesheet" href="/gh.css"><div class="b" style="width:50px;height:20px">a</div>', async ({ next, evaluate, host }) => {
    const first = [await next()];
    for (let i = 0; i < 5; i++) first.push(await next(500));
    assert(mirroredCss(first).includes("var(--w)"), "MP-10: the first document reads the lossy sheet as source text");
    await evaluate("location.href = '/b'").catch(() => {});
    for (let i = 0; i < 400 && await evaluate("location.pathname === '/b' && document.readyState === 'complete'").catch(() => false) !== true; i++) await settle(25);
    await host.request({ op: 'state' });
    let reset;
    for (let i = 0; i < 40 && reset === undefined; i++) { const packet = await next(300).catch(() => null); if (packet?.reset && packet.nodes.some(row => row[2] === 0 && row[3] === 'b')) reset = packet; }
    const packets = [reset];
    for (let i = 0; i < 3; i++) packets.push(await next(500));
    await evaluate("const s=document.styleSheets[0];s.deleteRule(0);s.insertRule('.b{color:rgb(1, 2, 3)}',0);true");
    for (let i = 0; i < 6 && !mirroredCss(packets).includes("rgb(1, 2, 3)"); i++) packets.push(await next(500));
    assert(mirroredCss(packets).includes("rgb(1, 2, 3)"), `MP-10: the edited rule reaches the viewer: ${mirroredCss(packets).slice(0, 300)}`);
  }, {
    "/b": { type: "text/html", body: '<!doctype html><link rel="stylesheet" href="/gh.css"><div class="b" style="width:50px;height:20px">b</div>' },
    "/gh.css": { type: "text/css", body: ".b{--w:3px;--c:red;border:var(--w) solid;border-color:var(--c)}" },
  }));

test("MP-10: an inline style attribute and a <style> element keep var() shorthand borders too", () => mirrored(
  '<style>.s{--w:3px;border:var(--w) solid;border-color:red}</style><div class="s" id="a" style="--w:3px;border:var(--w) solid;border-color:blue;width:40px">x</div>', async ({ next }) => {
    const packets = [await next()];
    for (let i = 0; i < 4; i++) packets.push(await next(500));
    const css = mirroredCss(packets), attrs = JSON.stringify(packets.flatMap(p => p.nodes ?? []));
    assert(css.includes("var(--w)"), `MP-10: the <style> source reaches the viewer: ${css.slice(0, 300)}`);
    assert(!/[;{]\s*(?!--)[a-z][a-z-]*:\s*;/.test(css), `MP-10: no empty longhand in the sheet: ${css.slice(0, 300)}`);
    assert(attrs.includes("border:var(--w) solid") || attrs.includes("border: var(--w) solid"), `MP-10: the style attribute keeps its shorthand: ${attrs.slice(0, 400)}`);
    assert(!/border-top-width: ;/.test(attrs), "MP-10: no empty longhand in the style attribute");
  }, {}));

// r61 DuckDuckGo results: result videos sit at opacity 0 (shown on hover) with no data yet. An invisible element paints
// nothing, so it must never hand the page to video (it did, "region_transform", whenever the carousel scrolled into view).
test("MP-10: a fully transparent opaque element never hands the page to video", () => mirrored(
  '<p>results</p><div style="height:20px"></div><div id="row" style="opacity:1"><video id="v" preload="none" style="opacity:0;width:189px;height:122px" src="http://localhost:PORT/none.mp4"></video><div style="opacity:0"><canvas id="c" width="80" height="40"></canvas></div><video id="h" preload="none" style="visibility:hidden;width:100px;height:60px"></video></div><div style="height:2000px"></div>', async ({ next }) => {
    const until = Date.now() + 4000; let packet = await next();
    assert.equal(packet.fallback, undefined);
    while (Date.now() < until) { packet = await next(500); assert.equal(packet.fallback, undefined, "MP-10: invisible media never hands the page to video"); }
  }, {}));

// r61 DuckDuckGo: a fade or slide distorts a region only while it runs. The page stays mirrored through a short
// distortion; only a lasting one hands it to video, and the cause is named.
test("MP-10: a transient or lasting distortion of a screenshot-crop region keeps the page mirrored (video patch, no whole-page fallback)", () => mirrored(
  '<div id="wrap"><canvas id="c" width="120" height="60"></canvas></div><script>const i=new Image();i.onload=()=>{document.getElementById("c").getContext("2d").drawImage(i,0,0);window.drawn=true};i.src="http://localhost:PORT/tile.svg"</script>', async ctx => {
    const { next, evaluate, host } = ctx;
    const marks = []; const timing = host.timing; host.timing = (name, ...rest) => { marks.push(name); return timing?.call(host, name, ...rest); };
    for (let i = 0; i < 100 && await evaluate("window.drawn === true") !== true; i++) await new Promise(resolve => setTimeout(resolve, 20));
    assert.equal((await next()).fallback, undefined);
    await evaluate("wrap.style.opacity='0.5';true");
    await stayDom(next, 1800, "MP-10: a short distortion never hands the page to video");
    await evaluate("wrap.style.opacity='';true");
    await settle(1100);
    for (let i = 0; i < 3; i++) assert.equal((await next(500)).fallback, undefined);
    await evaluate("wrap.style.filter='grayscale(1)';true");
    const packet = await stayDom(next, 5000, "MP-10: a lasting distortion never hands the page to video either");
    assert(!marks.some(m => /fallback/.test(m)), `MP-10: no fallback recorded: ${marks.filter(m => /fallback/.test(m))}`);
    assert.equal((await regionLease(ctx, packet.sequence)).rect.width, 120);
  }, { "/tile.svg": { type: "image/svg+xml", body: '<svg xmlns="http://www.w3.org/2000/svg" width="120" height="60"><rect width="120" height="60" fill="rgb(255,0,0)"/></svg>' } }));

// Review #941 @36a395b05 finding 5: rotate(180deg), scaleX(-1) and the rotate property keep the bounding size, and a
// translucent ancestor composites too: a viewport crop already holds that compositing, which the mirrored DOM applies
// again. Such regions hand the page to labelled video (it returns once the transform goes).
test("MP-10: review #941@36a395b05-5 a screenshot-crop region under a size-preserving transform or a translucent ancestor stays a DOM copy with a region lease at the on-screen rect", () => mirrored(
  '<div id="wrap"><canvas id="c" width="120" height="60"></canvas></div><script>const i=new Image();i.onload=()=>{document.getElementById("c").getContext("2d").drawImage(i,0,0);window.drawn=true};i.src="http://localhost:PORT/tile.svg"</script>', async ctx => {
    const { next, evaluate } = ctx;
    for (let i = 0; i < 100 && await evaluate("window.drawn === true") !== true; i++) await new Promise(resolve => setTimeout(resolve, 20));
    const plain = await next();
    assert.equal(plain.fallback, undefined); assert.equal(plain.tiles.length, 1, "MP-10: the tainted canvas shows as a screenshot crop");
    for (const [element, style] of [["c", "transform:rotate(180deg)"], ["wrap", "transform:scaleX(-1)"], ["c", "rotate:180deg"], ["wrap", "opacity:0.5"], ["c", "transform:rotate(90deg)"]]) {
      await evaluate(`${element}.style.cssText=${JSON.stringify(style)};true`);
      await settle(1100);
      const packet = await stayDom(next, 3000, `MP-10: ${element} ${style} never hands the page to video`);
      const lease = await regionLease(ctx, packet.sequence);
      // A quarter turn about the centre leaves the top of the viewport (clipped to the visible part).
      if (/rotate\(90deg\)/.test(style)) assert.equal(lease.rect.width, 60, `MP-10: ${element} ${style}`); else assert.equal(lease.rect.width * lease.rect.height, 120 * 60, `MP-10: ${element} ${style}: the lease covers the on-screen bounding box`);
      await evaluate(`${element}.style.cssText='';true`);
      await settle(1100);
      assert.equal((await next(1000)).fallback, undefined, `MP-10: still a DOM copy after ${style}`);
    }
  }, { "/tile.svg": { type: "image/svg+xml", body: '<svg xmlns="http://www.w3.org/2000/svg" width="120" height="60"><rect width="60" height="60" fill="rgb(255,0,0)"/><rect x="60" width="60" height="60" fill="rgb(0,0,255)"/></svg>' } }));

// Review #941 @36a395b05 finding 2: a same-origin child document is observed by its parent's world, but CSS events
// name the child's frame id: they must reach the world that mirrors that document (style and adopted sheets).
test("MP-10: review #941@36a395b05-2 same-count CSSOM edits in a same-origin child document reach the viewer (style sheet, adopted sheet)", () => mirrored(
  '<iframe id="f" src="/child" width="300" height="120"></iframe>', async ({ next, evaluate }) => {
    for (let i = 0; i < 100 && await evaluate("f.contentWindow?.sheet !== undefined && f.contentDocument.readyState === 'complete'") !== true; i++) await new Promise(resolve => setTimeout(resolve, 20));
    assert.equal((await next()).reset, true);
    const sheets = async needle => { for (let i = 0; i < 5; i++) for (const op of (await next(1000)).ops) if (op.op === "css" && op.css.includes(needle) || op.op === "adopted" && op.sheets.some(text => text.includes(needle))) return true; return false; };
    await evaluate("f.contentDocument.styleSheets[0].cssRules[0].style.color='rgb(1, 2, 3)';true");
    assert(await sheets("rgb(1, 2, 3)"), "MP-10: the child's style sheet edit reaches the viewer");
    await evaluate("f.contentWindow.sheet.cssRules[0].style.color='rgb(4, 5, 6)';true");
    assert(await sheets("rgb(4, 5, 6)"), "MP-10: the child's adopted sheet edit reaches the viewer");
  }, { "/child": { type: "text/html", body: '<!doctype html><style>#c{color:rgb(9, 9, 9)}</style><p id="c">c</p><p id="d">d</p><script>const s=new CSSStyleSheet();s.replaceSync("#d{color:rgb(7, 7, 7)}");document.adoptedStyleSheets=[s];window.sheet=s</script>' } }));

// Coordinator 2026-10-11 (Vault rule): a Vault-filled plain field is masked only while it still holds the fill (by the
// target's value_hash). Registered before the insertion, it stays masked until the fill lands; once the value is no
// longer the fill (the user edited it) it is a plain field again, for every viewer; a refill (new revision) masks it.
test("MP-11: a Vault-filled plain field is masked while it holds the fill: fill -> mask, edit -> unmask (every viewer), refill -> mask", () => mirrored('<form><input id="v"><p>after</p></form>', async ({ next, evaluate, host, subscribe }) => {
  const tab = [...host.tabs.values()][0], { connection, sessionId } = await host.browser.resolvePageTarget(tab.target_id);
  const { root } = await connection.send("DOM.getDocument", { depth: 0 }, sessionId), { nodeId } = await connection.send("DOM.querySelector", { nodeId: root.nodeId, selector: "#v" }, sessionId);
  const { node } = await connection.send("DOM.describeNode", { nodeId }, sessionId);
  const secret = "s3cret-fill-42", value_hash = createHash("sha256").update(secret).digest("hex");
  const protect = fill_revision => host.protect({ values: [secret], targets: [{ kind: "browser", target_id: tab.target_id, document_id: tab.document_id, node_ref: `backend:${node.backendNodeId}`, value_hash, fill_revision }] });
  const kinds = packet => { const out = []; const scan = rows => { let id = 0; for (const row of rows ?? []) { id += row[0]; if (row[2] === 4) out.push("mask"); else if (row[2] === "input") out.push(["input", row[4]?.form?.value]); } }; scan(packet.nodes); for (const op of packet.ops ?? []) if (op.op === "children") scan(op.nodes); return out; };
  const until = async (want, viewer = next) => { const seen = []; for (let i = 0; i < 6; i++) { const packet = await viewer(1000); seen.push(JSON.stringify(packet)); const k = kinds(packet); if (k.length && JSON.stringify(k) === JSON.stringify(want)) return seen; } assert.fail(`MP-11: expected ${JSON.stringify(want)}`); };
  await protect(1);
  assert.deepEqual(kinds(await next(0, true)), ["mask"], "MP-11: registered before the insertion: masked");
  await evaluate(`v.value=${JSON.stringify(secret)};v.dispatchEvent(new Event('input',{bubbles:true}));true`);
  const quiet = await next(1000);
  assert.deepEqual(kinds(quiet), [], "MP-11: the fill keeps the mask");
  const other = await subscribe();
  assert.deepEqual(kinds(await other.next(0, true)), ["mask"], "MP-11: both viewers are attached before the edit");
  await evaluate("v.select();v.value='user typed';v.dispatchEvent(new Event('input',{bubbles:true}));true");
  const edited = await until([["input", "user typed"]]);
  assert(![quiet, ...edited].some(text => JSON.stringify(text).includes(secret)), "MP-11: the fill never travels");
  const otherEdited = await until([["input", "user typed"]], other.next);
  assert(!otherEdited.some(text => text.includes(secret)), "MP-11: the other viewer never receives the fill");
  await evaluate(`v.value=${JSON.stringify(secret)};v.dispatchEvent(new Event('input',{bubbles:true}));true`);
  await protect(2);
  const refilled = [...await until(["mask"]), ...await until(["mask"], other.next)];
  assert(!refilled.some(text => text.includes(secret)), "MP-11: a refill masks both existing viewers again");
}));

// Round 11 drop drill (b3 hosted, DPR2): a reply lost with a dropped socket and not replayed left a gap; the viewer
// waited 500 ms, then a full snapshot (2.2 s), or the kernel's 1.5 s resend (3.1 s in the DPR2 hour). Protocol 501
// `resend`: the viewer asks for the packet after its base at once; earlier long polls end (a failed credit's orphan).
test("MP-10: protocol 501 a resend credit gets the packet after its base at once (lost reply), and ends an orphaned long poll returning the packet it issued", () => mirrored('<p id="x">one</p>', async ({ next, evaluate, host, subscription_id, generation }) => {
  const first = await next();
  const credit = (after, extra = {}) => host.request({ op: "mirror_next", subscription_id, generation, after_sequence: after, drift_nodes: [], wait_ms: 2000, ...extra });
  await evaluate("x.firstChild.data='two';true");
  const lost = await credit(first.sequence);
  assert.equal(lost.sequence, first.sequence + 1);
  let started = Date.now();
  assert.deepEqual(await credit(first.sequence, { resend: true }), lost, "MP-10: the lost packet again, byte for byte");
  assert.ok(Date.now() - started < 500, `MP-10: at once (${Date.now() - started} ms), not after the 1.5 s resend window`);
  // An orphaned long poll (its credit failed with the socket): the resend credit ends it and returns what it issued.
  const orphan = credit(lost.sequence);
  await new Promise(resolve => setTimeout(resolve, 150));
  started = Date.now();
  const resent = await credit(lost.sequence, { resend: true });
  assert.ok(Date.now() - started < 800, `MP-10: the orphaned poll ended (${Date.now() - started} ms)`);
  assert.deepEqual(resent, await orphan, "MP-10: the orphan's packet is the resend");
  assert.equal((await credit(resent.sequence, { wait_ms: 0 })).sequence, resent.sequence + 1, "MP-10: then the stream continues");
}));

// Protocol 501 (PUSH_READY r10 repro 1): setCustomValidity makes a control :invalid with no attribute or event
// footprint; the viewer's copy must match (snapshot, script, a page input handler reacting to mirrored typing).
test("MP-10: protocol 501 custom validity reaches the viewer as `state.custom_invalid` (snapshot, script, a page input handler)", () => mirrored(
  '<input id="i"><input id="v"><select id="s"><option>a</option></select><input id="j" value="abcd"><script>v.setCustomValidity("taken");j.addEventListener("input",()=>j.setCustomValidity(j.value.length<3?"short":""))</script>', async ({ next, evaluate, input }) => {
    const snapshot = await next(), ids = Object.fromEntries(elements(snapshot).filter(([, , attrs]) => attrs.id).map(([id, , attrs]) => [attrs.id, id]));
    const records = new Map(); let n = 0; for (const row of snapshot.nodes) { n += row[0]; records.set(`n${n}`, row[4] ?? {}); }
    assert.deepEqual(records.get(ids.v).state, { custom_invalid: true }, "MP-10: a control invalid at snapshot carries it");
    assert.equal(records.get(ids.i).state, undefined, "MP-10: a valid control carries nothing");
    const states = async (id, want) => { for (let i = 0; i < 6; i++) for (const op of (await next(1000)).ops) if (op.op === "state" && op.id === id && op.state.custom_invalid === want) return true; return false; };
    await evaluate("i.setCustomValidity('nope');s.setCustomValidity('pick one');true");
    assert(await states(ids.i, true), "MP-10: a script's custom validity reaches the viewer");
    await evaluate("v.setCustomValidity('');true");
    assert(await states(ids.v, false), "MP-10: cleared custom validity reaches the viewer");
    const started = Date.now();
    await input(snapshot.sequence, { kind: "focus", node_id: ids.j }); await input(snapshot.sequence, { kind: "selection", anchor_id: ids.j, anchor_offset: 0, focus_id: ids.j, focus_offset: 4 }); await input(snapshot.sequence, { kind: "text", text: "x" });
    assert(await states(ids.j, true), "MP-10: the page's input handler's verdict reaches the viewer");
    assert.ok(Date.now() - started < 1500, `MP-10: with the typing, not the 2 s sampler (${Date.now() - started} ms)`);
  }));

// Protocol 501 (PUSH_READY r10 repro 2): `<use href="/sprite.svg#icon">` showed the icon in the source and nothing in
// the mirror. The sprite file travels as a sanitized resource and the fragment as the use's href.
const spriteFile = '<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><style>.a{fill:rgb(255,0,0)} .b{background:url(http://evil.test/x.png)}</style><defs><linearGradient id="g"><stop offset="0" stop-color="rgb(0,0,255)"/></linearGradient></defs><symbol id="icon" viewBox="0 0 10 10"><rect class="a" width="10" height="10" onclick="alert(1)" style="stroke:url(http://evil.test/s.png)"/><circle r="2" fill="url(#g)"/><image href="http://evil.test/i.png" width="5" height="5"/><a href="javascript:alert(1)"><rect width="1" height="1"/></a><use xlink:href="#other"/><foreignObject><p>html</p></foreignObject><animate attributeName="href" to="javascript:alert(1)"/></symbol><symbol id="other"><rect width="2" height="2"/></symbol><script>alert(1)</script></svg>';
test("MP-10: protocol 501 an external <use> sprite travels as a sanitized resource and its fragment (snapshot, fragment change, to a local reference)", () => mirrored(
  '<svg width="40" height="40"><use id="u" href="/sprite.svg#icon" width="40" height="40"/></svg><svg id="local" width="1" height="1"><symbol id="sym"><rect width="1" height="1"/></symbol></svg>', async ({ next, evaluate }) => {
    await new Promise(resolve => setTimeout(resolve, 300));
    const packets = [await next()]; const use = rows(packets[0].nodes).find(r => r.tag === "use");
    assert.equal(use.attrs.href, "#icon", "MP-10: the fragment travels");
    assert.match(use.res ?? "", /^r[0-9]+$/, "MP-10: the file travels as a resource key");
    for (let i = 0; i < 4 && !packets.some(p => p.resources.some(r => r.key === use.res)); i++) packets.push(await next(1000));
    const text = Buffer.from(packets.flatMap(p => p.resources).find(r => r.key === use.res)?.data_base64 ?? "", "base64").toString();
    assert.match(text, /<symbol id="icon" viewBox="0 0 10 10">/, `MP-10: the symbols arrived: ${text.slice(0, 200)}`);
    assert.match(text, /fill="url\(#g\)"/, "MP-10: paint servers by fragment stay");
    assert.match(text, /\.a ?\{ ?fill: ?rgb\(255, ?0, ?0\)/, "MP-10: the sprite's own styles stay");
    for (const banned of ["script", "onclick", "evil.test", "<image", "<a ", "javascript", "foreignObject", "animate", "<p>"]) assert(!text.includes(banned), `MP-10: ${banned} never travels`);
    await evaluate("u.setAttribute('href','/sprite.svg#other');true");
    let ops = (await next(1000)).ops.filter(op => op.id === use.id);
    assert.deepEqual(ops.map(op => [op.op, op.op === "res" ? op.res : op.value]), [["attr", "#other"]], "MP-10: the same file, another symbol");
    await evaluate("u.setAttribute('href','#sym');true");
    ops = (await next(1000)).ops.filter(op => op.id === use.id);
    assert.deepEqual(ops.map(op => [op.op, op.op === "res" ? op.res : op.value]), [["res", null], ["attr", "#sym"]], "MP-10: a local reference drops the file");
  }, { "/sprite.svg": { type: "image/svg+xml", body: spriteFile } }));

// Review #941 @f6fb77245 finding 4: the compositing check walks the frame-owner chain too (a screenshot-only region
// inside a same-origin child frame whose owner element is translucent or rotated).
test("MP-10: review #941@f6fb77245-4 a screenshot-crop region inside a same-origin frame under a translucent or rotated frame owner stays a DOM copy with a region lease", () => mirrored(
  `<iframe id="f" width="200" height="100" srcdoc='<canvas id="c" width="120" height="60"></canvas><script>const i=new Image();i.onload=()=>{c.getContext("2d").drawImage(i,0,0);parent.drawn=true};i.src="http://localhost:PORT/tile.svg"</script>'></iframe>`, async ctx => {
    const { next, evaluate } = ctx;
    for (let i = 0; i < 100 && await evaluate("window.drawn === true") !== true; i++) await new Promise(resolve => setTimeout(resolve, 20));
    const plain = await next();
    assert.equal(plain.fallback, undefined); assert.equal(plain.tiles.length, 1, "MP-10: the tainted canvas in the frame shows as a crop");
    for (const style of ["opacity:0.5", "transform:rotate(180deg)", "filter:grayscale(1);transform:rotate(180deg)"]) {
      await evaluate(`f.style.cssText=${JSON.stringify(style)};true`);
      await new Promise(resolve => setTimeout(resolve, 1100));
      const packet = await stayDom(next, 3000, `MP-10: frame owner ${style} never hands the page to video`);
      // A translucent owner moves nothing; a rotated one would misplace the crop (translation-only mapping): no lease.
      if (/rotate/.test(style)) await assert.rejects(regionLease(ctx, packet.sequence), /region_target/);
      else { const lease = await regionLease(ctx, packet.sequence); assert.equal(lease.rect.width, 120); assert.equal(lease.rect.height, 60); }
      await evaluate("f.style.cssText='';true");
      await new Promise(resolve => setTimeout(resolve, 1100));
      assert.equal((await next(1000)).fallback, undefined, `MP-10: still a DOM copy after ${style}`);
    }
  }, { "/tile.svg": { type: "image/svg+xml", body: '<svg xmlns="http://www.w3.org/2000/svg" width="120" height="60"><rect width="60" height="60" fill="rgb(255,0,0)"/><rect x="60" width="60" height="60" fill="rgb(0,0,255)"/></svg>' } }));


// mirrorregion: Miguel 2026-10-19 "areas that cannot be mirrored are sent as video, and only when all fails the whole page".
const bigCanvas = '<style>body{margin:0}</style><canvas id="c" width="1280" height="600" style="display:block"></canvas><p id="t">text under the canvas</p><script>const i=new Image();i.onload=()=>{c.getContext("2d").drawImage(i,0,0);window.drawn=true};i.src="http://localhost:PORT/tile.svg"</script>';
test("MP-10 mirrorregion: an oversized opaque region (>60 % of the view) stays a DOM copy; its lease is the visible part", () => mirrored(bigCanvas, async ctx => {
  const { next, evaluate } = ctx;
  for (let i = 0; i < 100 && await evaluate("window.drawn === true") !== true; i++) await settle(20);
  const first = await next();
  assert.equal(first.fallback, undefined, "MP-10: a large region does not hand the page to video"); assert.equal(first.reset, true); assert(first.nodes.length > 3);
  const packet = await stayDom(next, 3000, "MP-10: stays DOM");
  const lease = await regionLease(ctx, packet.sequence);
  assert.deepEqual(lease.rect, { x: 0, y: 0, width: 1280, height: 600 });
}, { "/tile.svg": { type: "image/svg+xml", body: '<svg xmlns="http://www.w3.org/2000/svg" width="1280" height="600"><rect width="1280" height="600" fill="rgb(255,0,0)"/></svg>' } }));

// vimeo.com: an attached foreign frame whose owner has no box (a 0x0 iframe in a closed dialog) made regionBoxes
// throw for the whole page, so no region lease (the hero video) could ever be measured.
test("MP-10 mirrorregion: a zero-size frame owner does not stop other regions from leasing", () => mirrored(
  '<style>body{margin:0}</style><canvas id="c" width="200" height="100" style="display:block"></canvas><iframe src="http://localhost:PORT/child" style="width:0;height:0;border:0"></iframe><script>const i=new Image();i.onload=()=>{c.getContext("2d").drawImage(i,0,0);window.drawn=true};i.src="http://localhost:PORT/tile.svg"</script>', async ctx => {
    const { next, evaluate, host } = ctx;
    for (let i = 0; i < 100 && await evaluate("window.drawn === true") !== true; i++) await settle(20);
    await next();
    const packet = await stayDom(next, 3000, "MP-10: stays DOM");
    const stream = host.mirror.streams.get(ctx.subscription_id);
    assert(stream.frames.size > 0, "MP-10: the clipped foreign frame is attached");
    const lease = await regionLease(ctx, packet.sequence);
    assert.deepEqual(lease.rect, { x: 0, y: 0, width: 200, height: 100 });
  }, { "/child": { type: "text/html", body: "<!doctype html><p>child</p>" }, "/tile.svg": { type: "image/svg+xml", body: '<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><rect width="200" height="100" fill="rgb(255,0,0)"/></svg>' } }));

// Reviewer #1056 P2-3: the crop of a region inside a frame owner maps by translation only, so a rotated owner admits none.
const framedCanvas = async (transform, wrap = "") => { let count; await mirrored(
  '<style>body{margin:0}</style><canvas id="c" width="200" height="100" style="display:block"></canvas><div style="' + wrap + '"><iframe src="http://localhost:PORT/child" style="width:200px;height:100px;border:0;' + transform + '"></iframe></div>', async ctx => {
    const { next, host } = ctx;
    await next(); await stayDom(next, 3000, "MP-10: stays DOM");
    const stream = host.mirror.streams.get(ctx.subscription_id);
    assert(stream.frames.size > 0, "MP-10: the foreign frame is attached");
    count = (await host.mirror.v2.regionBoxes(stream, host.tabs.get(stream.tab_id))).length;
  }, { "/child": { type: "text/html", body: '<!doctype html><body style="margin:0"><canvas id="k" width="120" height="60" style="display:block"></canvas><script>const x=document.getElementById("k").getContext("2d");x.fillStyle="red";x.fillRect(0,0,60,60)</script>' } }); return count; };
test("MP-10 mirrorregion: a region inside a rotated frame owner is not admitted with a wrong crop", async () => {
  assert.equal(await framedCanvas(""), 2, "control: the top canvas and the frame's canvas are regions");
  assert.equal(await framedCanvas("transform:rotate(180deg)"), 1, "only the top canvas: the rotated owner's children are not boxed");
  // distorted() names one cause only: a filter or opacity nearer than the rotation must not hide it.
  assert.equal(await framedCanvas("filter:grayscale(1);transform:rotate(180deg)"), 1, "a filter on a rotated owner");
  assert.equal(await framedCanvas("", "opacity:.5;transform:rotate(90deg)"), 1, "a rotated ancestor behind a nearer opacity");
  assert.equal(await framedCanvas("", "opacity:.5"), 2, "control: opacity alone moves no pixel");
});

// DuckDuckGo results: many result videos faded in and out (opacity animation) next to a hidden carousel.
test("MP-10 mirrorregion: many hidden or fading videos and a carousel with a fading tainted canvas never hand the page to video", () => mirrored(
  '<style>body{margin:0}.f{animation:fade 600ms linear infinite alternate;width:120px;height:68px;display:inline-block}@keyframes fade{from{opacity:.05}to{opacity:1}}</style><div id="row">' +
  '<video class="f" preload="none"></video>'.repeat(12) + '<video preload="none" style="opacity:0;width:120px;height:68px"></video>'.repeat(6) +
  '<div class="f"><canvas id="c" width="120" height="60"></canvas></div></div><script>const i=new Image();i.onload=()=>{c.getContext("2d").drawImage(i,0,0);window.drawn=true};i.src="http://localhost:PORT/tile.svg"</script>', async ctx => {
    const { next, evaluate, host } = ctx;
    const marks = []; const timing = host.timing; host.timing = (name, ...rest) => { marks.push(name); return timing?.call(host, name, ...rest); };
    for (let i = 0; i < 100 && await evaluate("window.drawn === true") !== true; i++) await settle(20);
    assert.equal((await next()).fallback, undefined);
    const packet = await stayDom(next, 6000, "MP-10: fading media never hands the page to video");
    assert(!marks.some(m => /fallback/.test(m)), `MP-10: no fallback recorded: ${marks.filter(m => /fallback/.test(m))}`);
    const stream = host.mirror.streams.get(ctx.subscription_id), leases = [];
    for (const [region_id, r] of stream.regionRecords) if (r.kind === "tile") try { leases.push(await host.mirror.v2.regions.resolve({ mirror_subscription_id: ctx.subscription_id, generation: ctx.generation, document_id: stream.document_id, sequence: packet.sequence, region_id }, stream.scope)); } catch (error) { assert.match(String(error.message), /region_/, "MP-10: an unavailable region refuses with a region reason"); }
    assert(leases.length >= 13, `MP-10: the visible faded regions lease (${leases.length})`);
  }, { "/tile.svg": { type: "image/svg+xml", body: '<svg xmlns="http://www.w3.org/2000/svg" width="120" height="60"><rect width="120" height="60" fill="rgb(255,0,0)"/></svg>' } }));

// MP-08/MP-10/MP-11 review r19: closed custom roots in open roots, same-origin documents and later child lists.
test('MP-10: review r19 closed custom roots remain opaque in every mirrored tree and after insertion',()=>mirrored(
 '<x-open></x-open><iframe src="/child"></iframe><script>customElements.define("x-open",class extends HTMLElement{constructor(){super();this.attachShadow({mode:"open"}).innerHTML="<x-closed style=display:block;width:80px;height:40px></x-closed>"}});customElements.define("x-closed",class extends HTMLElement{constructor(){super();this.attachShadow({mode:"closed"}).innerHTML="<b>opaque content</b>"}})</script>',
 async({next,evaluate})=>{
  const first=await next();const tiles=first.nodes.filter(n=>n[4]?.reason==='closed_shadow');
  assert.equal(tiles.length,2,'closed hosts in the open root and same-origin iframe are opaque: '+JSON.stringify(first.nodes.filter(n=>['x-open','x-closed','iframe'].includes(n[2])||n[4]?.reason)));
  await evaluate('document.body.insertAdjacentHTML("beforeend","<x-closed style=display:block;width:80px;height:40px></x-closed>");true');
  let found=false;for(let i=0;i<8;i++){const p=await next(1000);if(p.ops.some(op=>op.op==='children'&&op.nodes.some(n=>n[4]?.reason==='closed_shadow')))found=true;}
  assert(found,'a closed custom root inserted after the snapshot becomes an opaque region through ordinary credits');
 },{'/child':{type:'text/html',body:'<!doctype html><x-closed style="display:block;width:80px;height:40px"></x-closed><script>customElements.define("x-closed",class extends HTMLElement{constructor(){super();this.attachShadow({mode:"closed"}).innerHTML="<b>frame opaque</b>"}})</script>'}}
));
test('MP-10: review r19 a newly inserted closed host becomes a tile without resetting the viewer',()=>mirrored(
 '<p>stable</p><script>customElements.define("x-closed",class extends HTMLElement{constructor(){super();this.attachShadow({mode:"closed"}).innerHTML="opaque"}})</script>',
 async({next,evaluate})=>{
  await next();await evaluate('document.body.insertAdjacentHTML("beforeend","<x-closed style=display:block;width:80px;height:40px></x-closed>");true');
  let found=false;for(let i=0;i<4;i++){const p=await next(600);assert(!p.reset,'ordinary delta, no reset');if(p.ops.some(op=>op.op==='children'&&op.nodes.some(n=>n[4]?.reason==='closed_shadow')))found=true;}
  assert(found,'late closed host is an opaque region rather than an empty mirrored host');
 }
));

// MP-08/MP-10/MP-11: #941 c956f84c7 review F1/F2. No reset may hide a lost descriptor.
for (const child of [false, true]) test(`MP-10: closed-host discovery retains a simultaneously loaded image in ${child ? 'an attached child' : 'the parent'}`,()=>mirrored(
 child ? '<iframe src="http://localhost:PORT/child" style="width:300px;height:160px"></iframe>' : '<p>stable</p>',
 async({next,evaluate,host})=>{
  if(child)await new Promise(r=>setTimeout(r,400));
  await next();
  const mutate='customElements.define("x-resource",class extends HTMLElement{constructor(){super();this.attachShadow({mode:"closed"}).innerHTML="opaque"}});document.body.insertAdjacentHTML("beforeend",\'<x-resource style="display:block;width:40px;height:30px"></x-resource><img width="40" height="40" src="/discovery.svg">\');true';
  if(child){
   const connection=await host.browser.ensureConnection();const {targetInfos}=await connection.send('Target.getTargets');
   const frame=targetInfos.find(t=>t.type==='iframe'&&t.url.includes('/child'));assert(frame,'MP-10: real attached child');
   const sessionId=await host.browser.ensureTargetSession(connection,frame.targetId);
   const result=await connection.send('Runtime.evaluate',{expression:`(async()=>{${mutate};await document.querySelector('img').decode();return true})()`,returnByValue:true,awaitPromise:true},sessionId);assert(!result.exceptionDetails);
  }else await evaluate(`(async()=>{${mutate};await document.querySelector('img').decode();return true})()`);
  let delivered=false;
  for(let i=0;i<6;i++){const p=await next(1000);assert(!p.reset,'MP-10: ordinary credit');if(p.resources.some(r=>Buffer.from(r.data_base64,'base64').toString().includes('#1234ab')))delivered=true;if(delivered)break;}
  assert(delivered,'MP-10: resource descriptor survives discovery and its bytes arrive without reset');
 },{'/child':{type:'text/html',body:'<!doctype html><p>child</p>'},'/discovery.svg':{type:'image/svg+xml',body:svg('#1234ab')}}
));
for(const placement of ['parent','open-root','same-origin-frame','attached-child'])test(`MP-10: an already-defined ${placement} host gets a closed root after the snapshot`,()=>mirrored(
 placement==='parent'?'<x-late id="late" style="display:block;width:80px;height:40px"></x-late><script>customElements.define("x-late",class extends HTMLElement{})</script>':placement==='open-root'?'<x-open></x-open><script>customElements.define("x-open",class extends HTMLElement{constructor(){super();this.attachShadow({mode:"open"}).innerHTML="<x-late id=late style=display:block;width:80px;height:40px></x-late>"}});customElements.define("x-late",class extends HTMLElement{})</script>':`<iframe src="http://${placement==='attached-child'?'localhost':'127.0.0.1'}:PORT/late" style="width:300px;height:160px"></iframe>`,
 async({next,evaluate,host})=>{
  if(placement.includes('frame')||placement==='attached-child')await new Promise(r=>setTimeout(r,400));
  const initial=await next();assert(!initial.nodes.some(n=>n[4]?.reason==='closed_shadow'),'MP-10: initially rootless');
  const attach='document.getElementById("late").attachShadow({mode:"closed"}).innerHTML="late opaque";true';
  if(placement==='attached-child'){
   const connection=await host.browser.ensureConnection();const {targetInfos}=await connection.send('Target.getTargets');const frame=targetInfos.find(t=>t.type==='iframe'&&t.url.includes('/late'));assert(frame);
   const sessionId=await host.browser.ensureTargetSession(connection,frame.targetId);assert(!(await connection.send('Runtime.evaluate',{expression:attach,returnByValue:true},sessionId)).exceptionDetails);
  }else await evaluate(placement==='open-root'?'document.querySelector("x-open").shadowRoot.getElementById("late").attachShadow({mode:"closed"}).innerHTML="late opaque";true':placement==='same-origin-frame'?'document.querySelector("iframe").contentDocument.getElementById("late").attachShadow({mode:"closed"}).innerHTML="late opaque";true':attach);
  let found=false;
  for(let i=0;i<5;i++){const p=await next(1000);assert(!p.reset,'MP-10: no reset');if(p.ops.some(op=>op.op==='children'&&op.nodes.some(n=>n[4]?.reason==='closed_shadow'))){found=true;break;}}
  assert(found,'MP-10: delayed closed root becomes an opaque region through ordinary credits');
 },{'/late':{type:'text/html',body:'<!doctype html><x-late id="late" style="display:block;width:80px;height:40px"></x-late><script>customElements.define("x-late",class extends HTMLElement{})</script>'}}
));

// MP-08/MP-10/MP-11: review a44f1afce F2, waiting child resources need their own load receipt.
test("MP-10: delayed same-origin child image becomes deliverable without a parent load or reset",()=>mirrored(
 '<iframe src="/idle-child" style="width:320px;height:140px"></iframe>',async({next,evaluate,host,subscription_id})=>{
  await next();await next(0);
  assert([...host.mirror.streams.get(subscription_id).resources.values()].some(r=>r.state==='waiting'),'MP-10: unused child author CSS first leaves the resource waiting');
  await evaluate("(()=>{const d=document.querySelector('iframe').contentDocument;const i=d.createElement('img');i.id='slow';i.width=100;i.height=60;i.src='/slow-child.svg';d.body.append(i);return true})()");
  for(let i=0;i<100&&!await evaluate("document.querySelector('iframe').contentDocument.getElementById('slow').complete");i++)await new Promise(r=>setTimeout(r,30));
  let delivered=false;for(let i=0;i<5;i++){const p=await next(800);assert(!p.reset);if(p.resources.some(r=>Buffer.from(r.data_base64,'base64').toString().includes('#6543ab'))){delivered=true;break;}}
  assert(delivered,'MP-10: the idle parent observer admits the completed child image');
 },{'/idle-child':{type:'text/html',body:'<!doctype html><style>.lazy{background-image:url(/slow-child.svg)}</style><p>idle</p>'},'/slow-child.svg':{type:'image/svg+xml',body:svg('#6543ab'),delay_ms:1200}}));

test('MP-08/MP-10/MP-11: opaque leases remeasure actual canvas bounds, protection and navigation', () => mirrored(
  '<style>body{margin:0}canvas{width:200px;height:100px}</style><canvas id="c"></canvas>',
  async ({host,next,evaluate,subscription_id,generation}) => {
    const packet=await next(),stream=host.mirror.streams.get(subscription_id);
    const region_id=[...stream.regionRecords].find(([,r])=>r.kind==='tile')[0];
    const target={mirror_subscription_id:subscription_id,generation,document_id:stream.document_id,sequence:packet.sequence,region_id};
    const lease=await host.mirror.v2.regions.resolve(target,stream.scope);
    assert.deepEqual(lease.rect,{x:0,y:0,width:200,height:100}); assert.equal(await lease.isCurrent(),true);
    for(const proof of (await lease.attestationState()).media){assert.equal(proof.id,region_id);assert.deepEqual(proof.box,[0,0,200,100]);assert(proof.backend_node_id>0)}
    await evaluate('document.querySelector("canvas").style.marginLeft="10px"');
    assert.equal(await lease.isCurrent(),false,'live layout changes retire before a new mirror packet');
    const replacement=await host.mirror.v2.regions.resolve(target,stream.scope);
    assert.equal(replacement.rect.x,10);
    await evaluate('document.querySelector("canvas").style.transform="rotate(180deg)"');
    assert.equal(await replacement.isCurrent(),false);
    const rotated=await host.mirror.v2.regions.resolve(target,stream.scope);
    assert.deepEqual(rotated.rect,{x:10,y:0,width:200,height:100},'a rotated region leases its on-screen rect');
    for(const proof of (await rotated.attestationState()).media){assert.equal(proof.id,region_id);assert.deepEqual(proof.box,[10,0,200,100],'a composited region is excluded from the native attestation by its on-screen box')}
  }
));
test('MP-11: a child opaque lease clips to the actual cross-origin frame viewport', () => mirrored(
  '<style>body{margin:0}iframe{width:100px;height:80px;border:0}</style><iframe src="http://localhost:PORT/child"></iframe>',
  async({host,next,subscription_id,generation})=>{
    const packet=await next(),stream=host.mirror.streams.get(subscription_id);
    const region_id=[...stream.regionRecords].find(([,r])=>r.kind==='tile')[0];
    const lease=await host.mirror.v2.regions.resolve({mirror_subscription_id:subscription_id,generation,document_id:stream.document_id,sequence:packet.sequence,region_id},stream.scope);
    assert.deepEqual(lease.rect,{x:0,y:40,width:100,height:40});
    for(const proof of (await lease.attestationState()).media){assert.equal(proof.id,region_id);assert.deepEqual(proof.box,[0,40,100,40]);assert(proof.backend_node_id>0)}
  },{'/child':{type:'text/html',body:'<style>body{margin:0}canvas{display:block;margin-top:40px;width:200px;height:100px}</style><canvas></canvas>'}}
));

// MP-08/MP-10/MP-11 M2: a real public spec has >337k elements. This regression
// is supplementary; the hosted WHATWG drill is the user-facing acceptance.
test("MP-10 M2: large documents paint a bounded prefix then continue on existing children ops", () => mirrored(
  '<h1>Viewport first</h1><main>' + '<p>later text</p>'.repeat(210000) + '</main>', async ({ next, host, evaluate }) => {
    const first = await next();
    assert.equal(first.fallback, undefined, 'MP-10 M2: large DOM must remain a DOM mirror');
    assert(first.nodes.length < 4096, 'MP-10 M2: first packet cannot serialize the entire document');
    assert([...texts(first).keys()].includes('Viewport first'), 'MP-10 M2: viewport content in the first packet');
    const stream = [...host.mirror.streams.values()][0];
    assert(stream, 'MP-10 M2: ordinary mirror subscription');
    const second = await next();
    assert.equal(second.reset, undefined);
    assert(second.ops.some(op => op.op === 'children'), 'MP-10 M2: ordinary children continuation');
    assert(second.ops.reduce((n, op) => n + (op.nodes?.length ?? 0), 0) < 8192, 'MP-10 M2: bounded continuation');
    // A different real tab still takes the ordinary path while hydration is pending.
    const other = await host.request({ op: 'open', url: 'about:blank' });
    assert(other.tab_id);
    const otherMirror = await host.request({op:'mirror_subscribe',tab_id:other.tab_id,generation:other.generation,device_scale_factor:1,wire:2});
    await host.request({op:'mirror_next',subscription_id:otherMirror.subscription_id,generation:other.generation,after_sequence:0,drift_nodes:[],wait_ms:0});
    const resumed = await next();assert(resumed.ops.reduce((n,op)=>n+(op.nodes?.length??0),0)<8192,'another small document cannot change this viewer continuation budget');
    assert.equal(await evaluate('document.querySelector("h1").textContent'), 'Viewport first');
  }));

test('MP-10 M2: a busy first task includes the visible viewport after a detailed header', () => mirrored(
  '<header>'+'<span>item </span>'.repeat(150)+'</header><p>Visible viewport tail</p><main>'+'<p>Later content</p>'.repeat(10001)+'</main>', async ({next,host,evaluate}) => {
    assert(await evaluate("document.querySelector('header+p').getBoundingClientRect().bottom < innerHeight"),'the native viewport includes the paragraph');
    const mirror=host.mirror.v2,original=mirror.evaluate.bind(mirror);let world;
    mirror.evaluate=async(w,expression,...args)=>{
      if(expression.endsWith('.snapshot()')){world=w;await original(w,"Object.defineProperty(performance,'now',{configurable:true,value:(()=>{let t=performance.now();return()=>t+=20})()});true");}
      return original(w,expression,...args);
    };
    try{const first=await next();assert(!first.fallback);assert(texts(first).has('Visible viewport tail'),'a detailed header must not leave the visible viewport unfinished');assert(first.nodes.length<4096,'first paint remains bounded');}
    finally{mirror.evaluate=original;if(world)await original(world,'delete performance.now;true');}
  }));

test('MP-10 M2: continuations finish the full DOM with bounded ordinary children packets', () => mirrored(
  '<h1>Viewport first</h1><main>' + '<p>middle</p>'.repeat(10001) + '<p>Last paragraph</p></main>', async ({next,host,subscription_id}) => {
    const first=await next();assert(!first.fallback);assert(first.nodes.length<4096);
    const seen=new Set(),add=(rows,parent)=>{let id=0;for(let i=0;i<rows.length;i++){const row=rows[i];id+=row[0];const at=`n${id}`,p=row[1]?rows[i-row[1]]._id:parent;assert(p===null||seen.has(p),'parent precedes the continuation record');assert(!seen.has(at),'a node is introduced once');seen.add(at);row._id=at;}};
    add(first.nodes,null);let tail=false;
    for(let n=0;n<200&&!tail;n++){const packet=await next();assert(!packet.reset);assert(!packet.fallback);let count=0;for(const op of packet.ops){if(op.op!=='children')continue;add(op.nodes,op.id);count+=op.nodes.length;tail||=op.nodes.some(row=>row[2]===0&&row[3]==='Last paragraph');}assert(count<8192);}
    assert(tail,'the last paragraph eventually arrives');assert(seen.size>20000,'full document, not just the viewport prefix');
    const world=host.mirror.streams.get(subscription_id).world;
    assert.equal(await host.mirror.v2.evaluate(world,`${world.ref}.pending()`),false,'the continuation settles');
  }));

// BROWSER-UX-09 / GATE-bytes (browserux r20, typed github.com): the page hydrates while the bounded
// snapshot continues. Each restart re-sent the whole head CSS (3 x 362 KB on an 8 Mbit/s link).
test('MP-10/MP-11 M2: mutations during a continuation travel as deltas, never a restarted snapshot', () => mirrored(
  '<style>h1:not(.marker-once){color:#123456}' + '.pad{margin:1px}'.repeat(6000) + '</style><h1 class="pad">Viewport first</h1><main id="m"><p>kept</p><p id="gone">removed later</p>' + '<p>middle</p>'.repeat(10001) + '<p>Last paragraph</p></main>'
  + '<script>let n=0;window.tick=setInterval(()=>{document.querySelector("h1").append(" t"+(n++))},15)</script>', async ({next,evaluate,host,subscription_id}) => {
    const first=await next();assert(!first.fallback);assert(first.nodes.length<4096);
    const [main]=elements(first).find(([,,attrs])=>attrs.id==='m'),[gone]=elements(first).find(([,,attrs])=>attrs.id==='gone');
    // Hydration shape: script/style churn in head, a removal and an insertion under the parent being walked.
    await evaluate(`document.head.append(Object.assign(document.createElement('script'),{textContent:'1'}),Object.assign(document.createElement('style'),{textContent:'i{}'}));document.querySelector('#gone').remove();document.querySelector('#m').append(Object.assign(document.createElement('section'),{textContent:'Appended paragraph'}));true`);
    const seen=new Set(first.nodes.map((_,i,rows)=>rows.slice(0,i+1).reduce((n,row)=>n+row[0],0)));
    let bytes=JSON.stringify(first).length,tail=false,appended=false,mainKids=null,ticks=false;
    const marker=JSON.stringify(first).split('marker-once').length-1;assert.equal(marker,1,'the head CSS is in the first packet');
    for(let n=0;n<300&&!(tail&&appended&&mainKids&&!mainKids.includes(gone)&&ticks);n++){
      const packet=await next(200);assert.notEqual(packet.reset,true,'a mutation never restarts the snapshot');assert(!packet.fallback);
      const json=JSON.stringify(packet);bytes+=json.length;assert(!json.includes('marker-once'),'the head CSS crosses once');
      for(const op of packet.ops){
        if(op.op!=='children')continue;
        let id=0;for(const row of op.nodes){id+=row[0];assert(!seen.has(id),'a node is introduced once');seen.add(id);tail||=row[2]===0&&row[3]==='Last paragraph';appended||=row[2]===0&&row[3]==='Appended paragraph';ticks||=row[2]===0&&/^ t\d+$/.test(row[3]);}
        if(op.id===main)mainKids=op.children;
      }
    }
    assert(tail,'the continuation reaches the last paragraph');assert(appended,'an insertion under the walked parent arrives');assert(ticks,'changes to finished nodes arrive while it continues');
    assert(mainKids&&!mainKids.includes(gone),'a child removed from the walked parent leaves the viewer');
    assert(bytes<2*JSON.stringify(first).length+1024*1024,`bounded total bytes: ${bytes}`);
    await evaluate('clearInterval(window.tick);true');
    const world=host.mirror.streams.get(subscription_id).world;
    for(let n=0;n<20&&await host.mirror.v2.evaluate(world,`${world.ref}.pending()`);n++)await next(100);
    assert.equal(await host.mirror.v2.evaluate(world,`${world.ref}.pending()`),false,'the continuation settles');
  }));

test('MP-10 M2: more than 100000 records on nodes the continuation has not reached do not restart it', () => mirrored(
  '<h1>Viewport first</h1><main>' + '<p>middle</p>'.repeat(10001) + '<p>Last paragraph</p></main>', async ({next,evaluate,host,subscription_id}) => {
    const first=await next();assert(!first.fallback);
    // A parsing or hydrating document: 110011 records, almost all about nodes not yet sent.
    await evaluate(`const ps=[...document.querySelectorAll('main p')];for(let i=0;i<11;i++)for(const p of ps)p.append('.');true`);
    let tail=false;
    for(let n=0;n<200&&!tail;n++){const packet=await next(200);assert.notEqual(packet.reset,true,'no overflow restart');for(const op of packet.ops)if(op.op==='children')tail||=op.nodes.some(row=>row[2]===0&&row[3]==='Last paragraph');}
    assert(tail,'the continuation reaches the last paragraph');
    const world=host.mirror.streams.get(subscription_id).world;
    for(let n=0;n<20&&await host.mirror.v2.evaluate(world,`${world.ref}.pending()`);n++)await next(100);
    assert.equal(await host.mirror.v2.evaluate(world,`${world.ref}.pending()`),false,'the continuation settles');
  }));

test('MP-10 M2: children appended to a parent the continuation is walking arrive in bounded continuation packets', () => mirrored(
  '<h1>Viewport first</h1><main>' + '<p>middle</p>'.repeat(10001) + '</main>', async ({next,evaluate}) => {
    const first=await next();assert(!first.fallback);
    // A parser still appending: the walk must reach them, not one synchronous diff of the whole parent.
    await evaluate(`const m=document.querySelector('main');for(let i=0;i<20000;i++)m.append(Object.assign(document.createElement('p'),{textContent:'late'}));m.append(Object.assign(document.createElement('p'),{textContent:'Last appended'}));true`);
    let tail=false;
    for(let n=0;n<300&&!tail;n++){const packet=await next(200);assert.notEqual(packet.reset,true);assert(!packet.fallback);let count=0;for(const op of packet.ops){if(op.op!=='children')continue;count+=op.nodes.length;tail||=op.nodes.some(row=>row[2]===0&&row[3]==='Last appended');}assert(count<8192,`bounded packet: ${count} nodes`);}
    assert(tail,'appended children arrive');
  }));


test('MP-08/MP-10 M2: native form and text echoes take priority over continuing the document', () => mirrored(
  '<input id="q"><h1>Viewport first</h1><main>' + '<p>middle</p>'.repeat(10001) + '</main>', async ({next,evaluate}) => {
    assert(!(await next()).fallback);
    await evaluate(`const q=document.querySelector('#q');q.value='typed';q.dispatchEvent(new Event('input',{bubbles:true}));document.querySelector('h1').firstChild.data='Changed text';true`);
    const echo=await next();assert(!echo.reset);assert(!echo.fallback);
    assert(echo.ops.some(op=>op.op==='form'&&op.form.value==='typed'));
    assert(echo.ops.some(op=>op.op==='text'&&op.text==='Changed text'));
    assert(!echo.ops.some(op=>op.op==='children'),'an echo never waits behind continuation records');
    assert((await next()).ops.some(op=>op.op==='children'),'hydration resumes on the next credit');
  }));


test('MP-10/MP-11 M2: a closed host discovered in a partial subtree discards all unpublished light descendants', () => mirrored(
  '<x-closed>' + '<p>Unpublished light content</p>'.repeat(10001) + '</x-closed><h1>After closed host</h1><script>customElements.define("x-closed",class extends HTMLElement{connectedCallback(){this.attachShadow({mode:"closed"}).innerHTML="<p>Closed pixels</p>"}})</script>', async ({next}) => {
    const first=await next();assert(!first.fallback);assert(first.nodes.some(row=>row[2]===5));
    assert(!JSON.stringify(first.nodes).includes('Unpublished light content'));
    let after=texts(first).has('After closed host');
    for(let n=0;n<10&&!after;n++){const packet=await next();assert(!packet.fallback);assert(!packet.reset);assert(!JSON.stringify(packet.ops).includes('Unpublished light content'));after=packet.ops.some(op=>op.op==='children'&&op.nodes.some(row=>row[2]===0&&row[3]==='After closed host'));}
    assert(after,'continuation skips the now-opaque subtree and reaches its next sibling');
  }));

test('MP-10 M2: a document that grows after an early small snapshot switches to a bounded prefix', () => mirrored(
  '<h1>Early paint</h1><main></main>', async ({next,evaluate}) => {
    assert(!(await next()).fallback);
    await evaluate(`document.querySelector('main').innerHTML='<p>Growing document</p>'.repeat(10001);true`);
    const grown=await next();assert(grown.reset);assert(!grown.fallback);assert(grown.nodes.length<4096);
    assert(texts(grown).has('Early paint'));assert((await next()).ops.some(op=>op.op==='children'));
  }));

test('MP-10/MP-11 M2: snapshot preparation fences old inputs before replacing the observer tree',()=>mirrored(
  '<div style="height:3000px">scrollable</div>',async({next,input,host})=>{
    const first=await next(),mirror=host.mirror.v2,original=mirror.evaluate.bind(mirror);let enter,release;
    const entered=new Promise(r=>enter=r),gate=new Promise(r=>release=r);let held=true;
    mirror.evaluate=async(w,expression,...args)=>{if(held&&expression.endsWith('.snapshot()')){held=false;enter();await gate}return original(w,expression,...args)};
    const reset=next(0,true);
    try{await entered;await assert.rejects(input(first.sequence,{kind:'scroll_to',node_id:null,x:0,y:10}),/stale mirror input epoch/);}
    finally{release();await reset;mirror.evaluate=original}
  }));

test('MP-10/MP-11 M2: a cancelled snapshot stays fenced and the next credit rebuilds its full base',()=>mirrored(
  '<div style="height:3000px">scrollable</div>',async({next,input,host,subscription_id,generation})=>{
    const first=await next(),mirror=host.mirror.v2,original=mirror.evaluate.bind(mirror),controller=new AbortController();let enter,release;
    const entered=new Promise(r=>enter=r),gate=new Promise(r=>release=r);let held=true;
    mirror.evaluate=async(w,expression,...args)=>{if(held&&expression.endsWith('.snapshot()')){held=false;enter();await gate}return original(w,expression,...args)};
    const reset=host.request({op:'mirror_next',subscription_id,generation,after_sequence:0,drift_nodes:[],wait_ms:0},{signal:controller.signal});
    try{await entered;controller.abort();release();await assert.rejects(reset,/cancel/i);mirror.evaluate=original;
      await assert.rejects(input(first.sequence,{kind:'scroll_to',node_id:null,x:0,y:10}),/stale mirror input epoch/);
      const recovered=await next();assert.equal(recovered.reset,true,'a private cancelled snapshot never becomes a delta base');assert(!recovered.fallback);
      await input(recovered.sequence,{kind:'scroll_to',node_id:null,x:0,y:10});
    }finally{release();await reset.catch(()=>{});mirror.evaluate=original}
  }));

test('MP-10 M2: repeated full resnapshots of a large document stay mirrored',()=>mirrored(
  '<h1>Viewport</h1>'+('<section>'+'<p>Content</p>'.repeat(100)+'</section>').repeat(1000)+'<p>Final paragraph</p>',async({next})=>{
    for(let epoch=0;epoch<6;epoch++){
      const first=await next(0,true);assert(!first.fallback);let tail=texts(first).has('Final paragraph');
      for(let n=0;n<2500&&!tail;n++){const packet=await next();assert(!packet.fallback,`full snapshot ${epoch+1} stays DOM mirrored`);tail=packet.ops.some(op=>op.op==='children'&&op.nodes.some(row=>row[2]===0&&row[3]==='Final paragraph'));}
      assert(tail,`full snapshot ${epoch+1} reaches the document end`);
    }
  }));

// MP-08/MP-10/MP-11 review #1001: repeated resets must not retain retired control ids.
test('MP-11: full snapshots retire the old control-shape cache',()=>mirrored('<input id="field"><select><option>one</option></select>',async({host,next,subscription_id,evaluate})=>{
 // Observe the real isolated-world Map calls; every snapshot allocates fresh ids.
 // Install before the subscription observer is created by its first credit.
 const tab=[...host.tabs.values()][0];const {connection,sessionId}=await host.browser.resolvePageTarget(tab.target_id);
 const {frameTree}=await connection.send('Page.getFrameTree',{},sessionId);
 const world=await host.browser.ensureFocusWorld(connection,sessionId,tab.target_id,frameTree.frame);
 const instrument=await connection.send('Runtime.evaluate',{contextId:world.contextId,expression:`(()=>{const NativeMap=Map;globalThis.__shapeMaps=[];globalThis.Map=class extends NativeMap{set(k,v){if(typeof k==='string'&&/^n[0-9]+$/.test(k)&&(v==='text'||v===false)&&!__shapeMaps.includes(this))__shapeMaps.push(this);return super.set(k,v)}};return true})()`,returnByValue:true},sessionId);assert(!instrument.exceptionDetails);
 for(let i=0;i<4;i++)await next(0,i>0);
 const result=await connection.send('Runtime.evaluate',{contextId:world.contextId,expression:"__shapeMaps.filter(m=>[...m.values()].every(v=>v==='text'||v===false)).map(m=>m.size)",returnByValue:true},sessionId);
 assert(!result.exceptionDetails);assert.deepEqual(result.result.value,[2],'only the current input and select ids remain');
}));

// r9 hosted drill (GitHub): the site's head style sheets filled the first 128 KiB, so the reset reached the
// viewer without a body and painted a white page until later credits. The head precedes the viewport.
test('MP-08/MP-10 M2: large head style sheets do not push the visible viewport out of the reset',()=>mirrored(
 '<head><style>'+'.c{color:#123456}\n'.repeat(12000)+'</style><style>'+'.d{margin:1px}\n'.repeat(8000)+'</style></head><body><h1 class="c d">Visible heading</h1>'+'<p>content</p>'.repeat(200)+'</body>',async({next})=>{
 const first=await next();assert(!first.fallback);
 assert(JSON.stringify(first).length>128*1024,'the fixture head alone exceeds the prefix bound');
 assert(texts(first).has('Visible heading'),'the viewport content travels with the head');
}));

test('MP-08/MP-10 M2: byte-heavy documents below the element threshold still continue in bounded packets',()=>mirrored(
 '<h1>Visible heading</h1><main>'+'<p>'+('payload '.repeat(1000))+'</p>'+('<p>'+('content '.repeat(1000))+'</p>').repeat(1500)+'</main>',async({next})=>{
 const first=await next();assert(!first.fallback);
 assert(texts(first).has('Visible heading'),'viewport heading precedes continuation');
 assert(JSON.stringify(first).length<9*1024*1024,'an ordinary reset stops at 8 MiB plus at most one bounded record');
 const second=await next();assert(!second.reset);assert(second.ops.some(op=>op.op==='children'));
}));

// MP-08/MP-10/MP-11: ordinary GitHub-sized navigation is one complete reset.
test('MP-10 M2: ordinary navigation includes the form after a byte-heavy body header atomically',()=>mirrored(
 '<head><style>'+'.c{color:#123456}\n'.repeat(12000)+'</style></head><body><header hidden>'+('<span>'+('template '.repeat(1000))+'</span>').repeat(80)+'</header><form><h1>Sign in to the destination</h1><input id="login"></form></body>',async({next,host,subscription_id})=>{
 const first=await next();assert(!first.fallback);
 assert(texts(first).has('Sign in to the destination'),'the complete ordinary document reaches the viewer before replacing its prior paint');
 const world=host.mirror.streams.get(subscription_id).world;
 assert.equal(await host.mirror.v2.evaluate(world,`${world.ref}.pending()`),false,'ordinary navigation leaves no unfinished body');
 assert(JSON.stringify(first).length<3*1024*1024,'ordinary resets retain a size cap');
}));

// MP-08/MP-10/MP-11: hosted GitHub's later CSS exceeds the initial 2MiB cap.
test('MP-10 M2: an ordinary document between two and four MiB fits the eight MiB reset cap',()=>mirrored(
 '<head><style>body{margin:0}</style></head><body><header hidden>'+('<span>'+('template '.repeat(1000))+'</span>').repeat(300)+'</header><form><h1>Complete destination form</h1><input id="login"></form></body>',async({next,host,subscription_id})=>{
 const first=await next();assert(!first.fallback);
 assert(texts(first).has('Complete destination form'),'the ordinary form cannot be deferred behind a partial reset');
 const world=host.mirror.streams.get(subscription_id).world;
 assert.equal(await host.mirror.v2.evaluate(world,`${world.ref}.pending()`),false,'ordinary navigation fits the eight MiB cap');
 assert(JSON.stringify(first).length<5*1024*1024,'record/message backstop remains bounded');
}));

// MP-08/MP-10/MP-11: current GitHub styles now exhaust 4 MiB before the file list.
test('MP-10 M2: an ordinary document between four and eight MiB includes its destination in the reset',()=>mirrored(
 '<body><header hidden>'+('<span>'+('template '.repeat(1000))+'</span>').repeat(600)+'</header><main><h1>Complete destination after current site head</h1><input id="destination"></main></body>',async({next,host,subscription_id})=>{
 const first=await next();assert(!first.fallback);
 assert(texts(first).has('Complete destination after current site head'),'a complete native page cannot be replaced by its header prefix');
 const world=host.mirror.streams.get(subscription_id).world;
 assert.equal(await host.mirror.v2.evaluate(world,`${world.ref}.pending()`),false);
 assert(JSON.stringify(first).length<9*1024*1024,'the ordinary reset retains a bounded byte cap');
}));

// MP-08/MP-10/MP-11: exercise the actual controller encoder, not only the observer.
test('MP-10 protocol539 completion follows the final initial continuation and compacts unchanged headers', () => mirrored(
  '<header hidden>'+('<span>hidden prefix</span>').repeat(12000)+'</header><p>Final viewport</p>', async ({ next }) => {
    const fixture = JSON.parse(await readFile(new URL('../../src/local/api/tests/protocol_shapes/browser-mirror2-completion-550.json', import.meta.url), 'utf8'))
    const first = await next(); assert.equal(first.reset, true)
    assert.equal(first.snapshot_complete, fixture.reset_header.snapshot_complete)
    let complete = first.snapshot_complete, drains = 0
    while (!complete) {
      const packet = await next(); assert(!packet.fallback)
      if (Object.hasOwn(packet, 'snapshot_complete')) complete = packet.snapshot_complete
      assert(++drains < 300, 'initial serialization must settle')
    }
    assert(drains > 0)
    const idle = await next()
    assert.equal(Object.hasOwn(idle, 'snapshot_complete'), false, 'unchanged completion inherits its base')
  }))

// GATE-bytes / BROWSER-UX-09 (typed github.com: 363 KB of deflated CSS in the first paint): a first paint carries the
// rules the document uses; each large sheet's full text follows on a quiet packet (readable and cross-origin sheets).
const bulkCss = prefix => `p.used{color:rgb(1, 2, 3)}@media (min-width: 1px){p.used{margin:0}.${prefix}-unused-0{color:red}}` + Array.from({ length: 2000 }, (_, i) => `.${prefix}-unused-${i}{color:rgb(${i % 250}, 1, 2);padding:${i}px}`).join("");
test("GATE-bytes: a first paint sends only the used rules of large sheets; their full text follows", () => mirrored(
  '<link rel="stylesheet" href="/a.css"><link rel="stylesheet" href="http://localhost:PORT/b.css"><p class="used">text</p>', async ({ next }) => {
    await new Promise(resolve => setTimeout(resolve, 300));
    const reset = await next(0, true), first = JSON.stringify(reset);
    assert.equal(reset.reset, true);
    assert(first.includes("rgb(1, 2, 3)") && first.includes("@media"), "the used rule and its group reach the first paint");
    assert(!first.includes("a-unused-1999") && !first.includes("b-unused-1999"), "unused rules wait");
    assert(first.length < 16 * 1024, `first paint ${first.length} bytes`);
    let later = "";
    for (let i = 0; i < 8 && !(later.includes("a-unused-1999") && later.includes("b-unused-1999")); i++) later += JSON.stringify(await next(1000));
    assert(later.includes("a-unused-1999") && later.includes("b-unused-1999"), "both full sheets follow");
  }, { "/a.css": { type: "text/css", body: bulkCss("a") }, "/b.css": { type: "text/css", body: bulkCss("b") } }));

// browserperf r6 (CPU): pruning a large sheet checks a selector against the few elements its rightmost compound names,
// not with a whole-document query per selector (most of the in-page sheet cost of every navigation's first paint).
const narrowCss = "p.used{color:rgb(1, 2, 3)}" + Array.from({ length: 1500 }, (_, i) => `.absent-${i % 7} .c${i % 300}{padding:${i}px}`).join("");
test("MP-08/MP-10 CPU: a first paint prunes a large sheet without a document query per selector", () => mirrored(
  '<link rel="stylesheet" href="/n.css"><p class="used">text</p>' + Array.from({ length: 300 }, (_, i) => `<span class="c${i}">${i}</span>`).join(""), async ({ next, evaluate, host, subscription_id }) => {
    await next();
    const world = host.mirror.streams.get(subscription_id).world;
    await host.mirror.v2.evaluate(world, "(()=>{const query=Document.prototype.querySelector;globalThis.queries=0;Document.prototype.querySelector=function(...args){globalThis.queries++;return query.apply(this,args)};return true})()");
    // A mutation retires the pruning index of the first packet, so the reset prunes afresh.
    await evaluate("document.body.append(document.createElement('i'));true"); await next(300);
    const first = JSON.stringify(await next(0, true));
    assert(first.includes("rgb(1, 2, 3)"), "the used rule reaches the first paint");
    assert(!first.includes(".absent-0 .c0"), "rules whose ancestor the document lacks wait");
    const queries = await host.mirror.v2.evaluate(world, "globalThis.queries");
    assert(queries < 50, `${queries} document queries for 1500 rules`);
  }, { "/n.css": { type: "text/css", body: narrowCss } }));

// Review #1016 @c5dfd2e31 item 3: a published node moved into a parent the continuation has not reached yet.
test('MP-10/MP-11 M2: a published subtree moved into an unvisited parent during a continuation stays mirrored', () => mirrored(
  '<main id="m"><p id="moved">Move me</p>' + '<p>middle</p>'.repeat(10000) + '<section id="destination"></section></main>', async ({ next, evaluate }) => {
    const first = await next(); assert(!first.fallback); assert(first.nodes.length < 4096);
    const moved = elements(first).find(([, , attrs]) => attrs.id === 'moved')?.[0]; assert(moved, 'the paragraph is in the prefix');
    await evaluate("document.querySelector('#destination').append(document.querySelector('#moved'));true");
    let section = null, restored = false;
    for (let n = 0; n < 300 && !restored; n++) {
      const packet = await next(200); assert.notEqual(packet.reset, true);
      for (const op of packet.ops) {
        if (op.op !== 'children') continue;
        let id = 0; for (const row of op.nodes) { id += row[0]; if (row[2] === 'section' && row[3]?.id === 'destination') section = `n${id}`; }
        if (section && op.id === section && (op.children.includes(moved) || op.nodes.some(row => row[2] === 0 && row[3] === 'Move me'))) restored = true;
      }
    }
    assert(section, 'the continuation reaches the section'); assert(restored, 'the moved paragraph is mirrored under its new parent');
  }));

// maskleak r3 (MP-11): a Vault fill into a password field that the page then flips to type=text must never put the
// value on the mirror wire: every packet from the first snapshot on is scanned for the secret (plain, UTF-16, base64).
const secretForms = secret => { const utf16 = Buffer.from(secret, "utf16le"); return [secret, utf16.toString("latin1"), Buffer.from(secret).toString("base64").replace(/=+$/, ""), utf16.toString("base64").replace(/=+$/, ""), Buffer.from(secret).toString("hex")]; };
for (const [name, html, fillAndFlip] of [
  ["the page flips the field after the fill", '<form><input id="f" type="password"><p>after</p></form>', async (fill, evaluate) => { await fill(); await evaluate("f.type='text';true"); }],
  ["the page flips the field in the fill's own task", '<form><input id="f" type="password"><p>after</p></form>', async (fill, evaluate, secret) => { await evaluate(`f.value=${JSON.stringify(secret)};f.dispatchEvent(new Event('input',{bubbles:true}));f.type='text';true`); }],
]) test(`MP-11 maskleak: Vault fill into a password field, then ${name}: the value never travels`, () => mirrored(html, async ({ next, evaluate, host, subscribe }) => {
  const tab = [...host.tabs.values()][0], { connection, sessionId } = await host.browser.resolvePageTarget(tab.target_id);
  const { root } = await connection.send("DOM.getDocument", { depth: 0 }, sessionId), { nodeId } = await connection.send("DOM.querySelector", { nodeId: root.nodeId, selector: "#f" }, sessionId);
  const { node } = await connection.send("DOM.describeNode", { nodeId }, sessionId), node_ref = `backend:${node.backendNodeId}`;
  const secret = "magenta-secret-" + Math.random().toString(36).slice(2, 8), value_hash = createHash("sha256").update(secret).digest("hex");
  await host.protect({ values: [secret], targets: [{ kind: "browser", target_id: tab.target_id, document_id: tab.document_id, node_ref, value_hash, fill_revision: 1 }] });
  const seen = [JSON.stringify(await next(0, true))];
  // The Vault paste (host.secret) types the value into the field with physical keys when the kernel owns a desktop; a
  // headless run types it with one input event per character.
  const fill = async () => process.env.CHARIOX_KERNEL_BROWSER_DISPLAY === "1"
    ? await host.handle({ id: 7, method: "host.secret", params: { tab_id: tab.tab_id, generation: (await host.request({ op: "state" })).generation, document_id: tab.document_id, node_ref, observed_by: "terminal:owner", action: { kind: "fill", text: secret, expected_document_url: tab.url } } }).then(reply => assert.ok(reply.ok, `the Vault paste lands: ${JSON.stringify(reply).slice(0, 200)}`))
    : evaluate(`f.focus();for(const c of ${JSON.stringify(secret)}){f.value+=c;f.dispatchEvent(new Event('input',{bubbles:true}))}true`);
  await fillAndFlip(fill, evaluate, secret);
  assert.equal(await evaluate("document.querySelector('input').value"), secret, "the page holds the fill");
  for (let i = 0; i < 6; i++) seen.push(JSON.stringify(await next(250)));
  seen.push(JSON.stringify(await next(0, true)));
  // A viewer that attaches after the flip (reconnect, second tab of the page) gets a snapshot from the same serializer.
  const late = await subscribe();
  seen.push(JSON.stringify(await late.next(0, true)));
  for (const form of secretForms(secret)) assert(!seen.some(text => text.includes(form)), `MP-11: the fill never travels (${form.slice(0, 6)}…)`);
  assert(seen.some(text => JSON.parse(text).nodes?.some?.(row => row[2] === 4) || JSON.parse(text).ops?.some?.(op => op.op === "children" && op.nodes.some(row => row[2] === 4))) || JSON.parse(seen.at(-1)).nodes.some(row => row[2] === 4), "MP-11: the flipped field ends as a mask");
}));

// MP-08/MP-10/MP-11: a viewport crop includes the page's overlaid controls. The pointer belongs to the person.
test("MP-08/MP-10 mirrorregion: pointer over a media patch reaches the real overlaid control", () => mirrored(
  '<style>body{margin:0}canvas{position:absolute;left:100px;top:100px;width:400px;height:200px}button{position:absolute;left:200px;top:150px;width:120px;height:50px}</style><canvas id="media" width="400" height="200"></canvas><button onclick="this.textContent=\'clicked\'">cookie choice</button>',
  async ({next,input,evaluate,host,subscription_id}) => {
    const packet=await next(),node_id=[...host.mirror.streams.get(subscription_id).regionRecords].find(([,r])=>r.kind==='tile')[0];
    await input(packet.sequence,{kind:'click',node_id,x:150,y:70});
    assert.equal(await evaluate('document.querySelector("button").textContent'),'clicked','the real pointer hits the overlaid control, not the video underneath');
  }
));

test("MP-08/MP-10 mirrorregion: pointer beside media reaches an overlaid control on an ordinary node", () => mirrored(
  '<style>body{margin:0}canvas{position:absolute;left:100px;top:100px;width:400px;height:200px}#under{position:absolute;left:100px;top:310px;width:400px;height:100px}button{position:absolute;left:200px;top:330px;width:120px;height:50px}</style><canvas width="400" height="200"></canvas><div id="under">page copy</div><button onclick="this.textContent=\'clicked\'">cookie choice</button>',
  async ({next,input,evaluate,host,subscription_id}) => {
    const packet=await next(),stream=host.mirror.streams.get(subscription_id),world=stream.world;
    const node_id=await host.mirror.v2.evaluate(world,`${world.ref}.idOfNode.call(document.getElementById("under"))`);
    assert.equal(stream.regionRecords.get(node_id).kind,'element');
    await input(packet.sequence,{kind:'click',node_id,x:150,y:40});
    assert.equal(await evaluate('document.querySelector("button").textContent'),'clicked');
  }
));
test('MP-08/MP-10/MP-11: reopening a desktop mirror restores the captured viewport', () => mirrored(
  '<p>visible viewport</p><div style="position:fixed;right:0;bottom:0;width:20px;height:20px;background:rgb(101,102,103)"></div>', async ({host, tab_id, generation, subscription_id, subscribe}) => {
    const tab=host.tabs.get(tab_id),{connection,sessionId}=await host.browser.resolvePageTarget(tab.target_id);
    const {windowId}=await connection.send('Browser.getWindowForTarget',{targetId:tab.target_id});
    await connection.send('Browser.setWindowBounds',{windowId,bounds:{width:1000,height:700}});
    await host.request({op:'mirror_close',subscription_id,generation});
    await host.releaseTabScale(tab_id);
    const desktop=await subscribe(),frame=await desktop.next();
    const shot=await host.request({op:'screenshot',tab_id,generation,bound_frames:true});
    const png=Buffer.from(shot.data_base64,'base64');
    assert.deepEqual([png.readUInt32BE(16),png.readUInt32BE(20),shot.width,shot.height],
      [frame.css_width*frame.device_scale_factor,frame.css_height*frame.device_scale_factor,frame.css_width*frame.device_scale_factor,frame.css_height*frame.device_scale_factor],
      'the reopened mirror and the CDP visible capture describe the same viewport');
    const phone=await subscribe({layout_width:400,layout_height:250});await phone.next();
    const small=await host.request({op:'screenshot',tab_id,generation,bound_frames:true});
    const smallPng=Buffer.from(small.data_base64,'base64');
    assert.deepEqual([small.width,small.height,smallPng.readUInt32BE(16),smallPng.readUInt32BE(20)],[400,250,400,250]);
    const pixels=decodePng(small.data_base64).pixels,corner=((small.height-2)*small.width+small.width-2)*4;
    assert.deepEqual([...pixels.subarray(corner,corner+3)],[101,102,103],'the logical bottom-right marker retains its native capture projection');
  }));

test("protocol 560/168: a layout change resets the Tab's existing mirror streams and the failed registration releases it", () => mirrored(
  "<p>page</p>", async ({ subscribe, host, evaluate, tab_id, generation }) => {
    const innerWidth = () => evaluate("innerWidth");
    const a = await subscribe({ layout_width: 400, layout_height: 700 }), first = await a.next();
    assert.deepEqual([first.reset, first.css_width, await innerWidth()], [true, 400, 400]);
    const b = await subscribe({ layout_width: 600, layout_height: 700 });
    assert.equal((await b.next()).css_width, 600);
    const moved = await a.next();
    assert.deepEqual([moved.reset, moved.css_width], [true, 600], "the owner changed: the other viewer re-bases at the new page size");
    await host.request({ op: "mirror_close", subscription_id: b.subscription_id, generation });
    const restored = await a.next();
    assert.deepEqual([restored.reset, restored.css_width, await innerWidth()], [true, 1280, 1280], "releasing the layout re-bases the remaining viewer");
    assert.notEqual((await a.next()).reset, true, "a steady page keeps its stream");
    // A registration that fails after the metrics override leaves no resized page behind.
    const hold = host.browser.inputCapture.hold;
    host.browser.inputCapture.hold = async () => { throw new Error("registration failed"); };
    await assert.rejects(subscribe({ layout_width: 400, layout_height: 700 }), /registration failed/);
    host.browser.inputCapture.hold = hold;
    assert.equal(await innerWidth(), 1280, "the failed registration did not leave the Tab resized");
  }));

test("protocol 560/168: a protected capture is the size of the laid-out page and the canonical page again after the layout is released", () => mirrored(
  "<p>page</p>", async ({ subscribe, host, tab_id, generation }) => {
    const scale = host.scales.get(tab_id) ?? host.viewport().device_scale_factor;
    const capture = async (width, height) => {
      const shot = await host.request({ op: "screenshot", tab_id, generation, bound_frames: true, _capture_protection: true, _capture_viewport: { width: width * scale, height: height * scale } });
      const png = Buffer.from(shot.data_base64, "base64");
      return [png.readUInt32BE(16), png.readUInt32BE(20)];
    };
    const narrow = await subscribe({ layout_width: 400, layout_height: 700 });
    await narrow.next();
    assert.deepEqual(await capture(400, 700), [400 * scale, 700 * scale], "the capture matches the mirror frame at the laid-out size");
    const wide = await subscribe({ layout_width: 1440, layout_height: 900 });
    await wide.next();
    assert.deepEqual(await capture(1280, 800), [1280 * scale, 800 * scale], "the newest viewer asks for the canonical page");
    const again = await subscribe({ layout_width: 400, layout_height: 700 });
    await again.next();
    assert.deepEqual(await capture(400, 700), [400 * scale, 700 * scale], "400 -> 1440 -> 400 captures at the laid-out size");
    await host.request({ op: "mirror_close", subscription_id: again.subscription_id, generation });
    await host.request({ op: "mirror_close", subscription_id: narrow.subscription_id, generation });
    assert.deepEqual(await capture(1280, 800), [1280 * scale, 800 * scale], "closing the layout owners restores the canonical capture");
  }));

test("protocol 560/168: a layout released while its override is still in flight does not leave the Tab resized", () => mirrored(
  "<p>page</p>", async ({ subscribe, host, evaluate }) => {
    for (let i = 0; i < 5; i++) {
      const [narrow, wide] = await Promise.all([subscribe({ layout_width: 400, layout_height: 700 }), subscribe({ layout_width: 1440, layout_height: 900 })]);
      await new Promise(resolve => setTimeout(resolve, 150));
      const owner = host.layouts.size ? "narrow" : "canonical", width = await evaluate("innerWidth");
      assert.equal(width, owner === "narrow" ? 400 : 1280, `round ${i}: the page width follows the layout owner (${owner})`);
      for (const s of [narrow, wide]) await host.request({ op: "mirror_close", subscription_id: s.subscription_id, generation: (await host.request({ op: "state" })).generation });
      assert.equal(await evaluate("innerWidth"), 1280, `round ${i}: closing restores the page`);
    }
  }));

test("protocol 560/168: a layout changed and restored is a different layout key", () => mirrored(
  "<p>page</p>", async ({ subscribe, host }) => {
    const a = await subscribe({ layout_width: 400, layout_height: 700 }), v2 = host.mirror.v2, tabId = [...host.tabs.keys()][0], key = v2.layoutKey(tabId);
    await subscribe({ layout_width: 600, layout_height: 700 });
    await subscribe({ layout_width: 400, layout_height: 700 });
    assert.notEqual(v2.layoutKey(tabId), key, "400 -> 600 -> 400 is not the layout the credit started with");
    await a.next();
  }));
test("protocol 629: a layout change while observed retries the credit with a reset", () => mirrored(
  "<p id='p'>page</p>", async ({ subscribe, host, evaluate }) => {
    const a = await subscribe({ layout_width: 400, layout_height: 700 });
    const first = await a.next();
    // Another viewer changes layout after the credit reads its size and before publication.
    const v2 = host.mirror.v2, real = v2.layoutKey.bind(v2), tiles = v2.tiles.bind(v2);
    let moved = false;
    v2.layoutKey = tabId => (moved ? "600x700" : real(tabId));
    v2.tiles = (...args) => { moved = true; return tiles(...args); };
    await evaluate("document.getElementById('p').textContent='changed'");
    try {
      const next = await a.next(2000);
      assert.equal(moved, true, "the observation crossed the injected layout change");
      assert.equal(next.reset, true, "the same credit retries with a fresh snapshot");
      assert.equal(next.sequence, first.sequence + 1, "the retired observation publishes no sequence");
      assert.match(JSON.stringify(next), /changed/, "the consumed change reaches the viewer in the reset");
    } finally { v2.layoutKey = real; v2.tiles = tiles; }
  }));

test('MP-08/MP-10/MP-11: typing already sent survives a same-document mirror reset', () => mirrored(
  '<input id="q" autofocus onkeydown="if(event.key===\'Enter\')window.submitted=this.value"><p id="status">ready</p>',
  async ({ next, input, evaluate }) => {
    const first = await next();
    await evaluate("document.querySelector('#q').focus()");
    await input(first.sequence, { kind: 'key', key: 'a' });
    await evaluate("document.querySelector('#status').textContent='reconstructed'");
    const replacement = await next(0, true);
    assert(replacement.reset && replacement.sequence > first.sequence);
    // These keys left the viewer before the reset packet arrived. No pointer,
    // selection or field state is being re-addressed: native focus is current.
    await input(first.sequence, { kind: 'key', key: 'b' });
    await input(first.sequence, { kind: 'key', key: 'c' });
    await input(first.sequence, { kind: 'key', key: 'Enter' });
    assert.equal(await evaluate('window.submitted'), 'abc');
  }));

// browserperf r9: resource keys follow URLs, so a site's shared sheet is the same text in each of its documents (the
// viewer's retained copy can serve it) even when each document registers other resources first.
test('MP-08/MP-10: a shared stylesheet is the same mirrored text in two documents of a site', () => mirrored(
  '<img src="/x.svg"><img src="/y.svg"><link rel="stylesheet" href="/s.css"><p>a</p>',
  async ({ next, evaluate, host }) => {
    const sheetOf = packet => { const extra = packet.nodes.find(row => row[2] === 'style')?.[4]; return extra ? extra.css ?? packet.sheets?.[extra.css_ref] : undefined; };
    const first = await next();
    const before = sheetOf(first);
    assert.match(before ?? '', /url\("mr:r[0-9]+"\)/, 'the sheet names its images by resource key');
    await evaluate("location.href = '/b'").catch(() => {});
    for (let i = 0; i < 400 && await evaluate("location.pathname === '/b' && document.readyState === 'complete'").catch(() => false) !== true; i++) await settle(25);
    await host.request({ op: 'state' });
    let after;
    for (let i = 0; i < 40 && after === undefined; i++) { const packet = await next(300).catch(() => null); if (packet?.reset && packet.nodes.some(row => row[2] === 0 && row[3] === 'b')) after = sheetOf(packet); }
    assert.equal(after, before, 'the second document mirrors the shared sheet as the same text');
  }, {
    '/b': { type: 'text/html', body: '<!doctype html><img src="/z.svg"><img src="/w.svg"><img src="/v.svg"><link rel="stylesheet" href="/s.css"><p>b</p>' },
    '/s.css': { type: 'text/css', body: `.i{background:url(/i1.svg)}.j{background:url(/i2.svg)}${'.k{color:rgb(1, 2, 3)}'.repeat(150)}` },
    ...Object.fromEntries(['x', 'y', 'z', 'w', 'v', 'i1', 'i2'].map(name => [`/${name}.svg`, { type: 'image/svg+xml', body: '<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>' }])),
  }));

// browserperf r9: pruning a large sheet for first paint costs the page a rule-by-rule match; a sheet the viewer already
// retains (it advertised the digest) arrives whole by reference instead, and nothing is left to send later.
test('MP-08/MP-10: a large sheet the viewer retains reaches the next document whole by reference, not pruned', () => mirrored(
  '<link rel="stylesheet" href="/big.css"><p class="k">a</p>',
  async ({ next, evaluate, host }) => {
    const digest = text => createHash('sha256').update(text).digest('hex').slice(0, 24);
    const style = packet => packet.nodes.find(row => row[2] === 'style');
    const textOf = (packet, extra) => extra?.css ?? packet.sheets?.[extra?.css_ref];
    const first = await next(), pruned = textOf(first, style(first)[4]);
    let full;
    for (let i = 0; i < 40 && full === undefined; i++) { const packet = await next(300); const op = packet.ops.find(op => op.op === 'css'); if (op) full = textOf(packet, op); }
    assert(full && pruned.length < full.length, 'the first document gets the pruned sheet first, its full text later');
    await evaluate("location.href = '/b'").catch(() => {});
    for (let i = 0; i < 400 && await evaluate("location.pathname === '/b' && document.readyState === 'complete'").catch(() => false) !== true; i++) await settle(25);
    await host.request({ op: 'state' });
    let reset;
    for (let i = 0; i < 40 && reset === undefined; i++) { const packet = await next(300, false, { known_sheets: [digest(full)] }).catch(() => null); if (packet?.reset && packet.nodes.some(row => row[2] === 0 && row[3] === 'b')) reset = packet; }
    assert.equal(style(reset)?.[4]?.css_ref, digest(full), 'the retained sheet is referenced whole');
    assert.equal(reset.sheets?.[digest(full)], undefined, 'its text does not travel again');
    for (let i = 0; i < 6; i++) assert(!(await next(300)).ops.some(op => op.op === 'css'), 'no full text follows later');
  }, {
    '/b': { type: 'text/html', body: '<!doctype html><link rel="stylesheet" href="/big.css"><p class="k">b</p>' },
    '/big.css': { type: 'text/css', body: `.k{color:rgb(1, 2, 3)}${Array.from({ length: 2500 }, (_, i) => `.unused${i}{color:rgb(4, 5, 6)}`).join('')}` },
  }));

// Protocol 629/199 viewport-first snapshot. Compact rows: [idDelta, parentBack, tag | kind, attrs | text | 0, extra].
const viewportRows = rows => { let id = 0; return rows.map(row => { id += row[0]; return { id: `n${id}`, element: typeof row[2] === "string", attrs: typeof row[3] === "object" && row[3] ? row[3] : {}, extra: row[4] ?? {} }; }); };
const keyed = (count, body) => `<style>body{margin:8px;font:16px/20px sans-serif}p{margin:0 0 12px}</style>${body}`.replace(/<(\w+)(?=[\s>])/g, (tag, name) => `${tag} data-k="${name}${++count.n}"`);
// The viewer's view of a viewport-first snapshot, emulated on the page itself: deferred children absent, each kept box
// pinned to its CSS height. `placed` is the viewer's check (every kept box where the page had it); `same` is rule C
// (every element on screen where it was, and the same scrollbar).
const placeholderExpression = deferred => `(() => {
  const keys = ${JSON.stringify(deferred)}, onScreen = () => [...document.querySelectorAll('[data-k]')].map(e => [e.dataset.k, e.getBoundingClientRect()]).filter(([, r]) => r.bottom > 0 && r.top < innerHeight && r.width > 0).map(([k, r]) => [k, Math.round(r.x * 4), Math.round(r.y * 4), Math.round(r.width * 4), Math.round(r.height * 4)].join(':'));
  const before = onScreen(), width = document.documentElement.clientWidth;
  for (const [k, box] of keys) { const e = document.querySelector('[data-k="' + k + '"]'); e.replaceChildren(); if (box[4] !== null) { e.style.setProperty('height', box[4] + 'px', 'important'); e.style.setProperty('margin-top', box[5] + 'px', 'important'); e.style.setProperty('margin-bottom', box[6] + 'px', 'important'); } }
  const placed = keys.every(([k, box]) => { if (box[4] === null) return true; const r = document.querySelector('[data-k="' + k + '"]').getBoundingClientRect(); return Math.abs(r.x + scrollX - box[0]) <= 1 && Math.abs(r.y + scrollY - box[1]) <= 1 && Math.abs(r.width - box[2]) <= 1 && Math.abs(r.height - box[3]) <= 1; });
  const after = onScreen();
  return { placed, same: JSON.stringify(before) === JSON.stringify(after) && width === document.documentElement.clientWidth, shown: before.length };
})()`;
const article = keyed({ n: 0 }, `<h1>Title</h1>${Array.from({ length: 80 }, (_, i) => `<p>Paragraph ${i} <b>bold</b> text that wraps across the line a few times so the article is long enough to scroll far below the fold of the viewport.</p>`).join("")}<div id="dock"><div id="banner" style="position:fixed;bottom:0;left:0;right:0;height:30px;background:#ccc">cookie banner</div></div><div id="hidden" style="display:none"><p>hidden</p></div><div id="sheet"><style>b{color:#a00}</style><p>styled</p></div>`);

test("MP-08/MP-10 protocol 629: a viewport-first snapshot defers off-screen blocks with their boxes and fills them later", () => mirrored(article, async ({ subscribe, evaluate }) => {
  const viewer = await subscribe({ viewport_first: true });
  const first = await viewer.next();
  assert.equal(first.reset, true);
  assert.equal(first.snapshot_viewport_complete, true, "the viewport part is in the first packet");
  assert.equal(first.snapshot_complete, false, "the off-screen part follows");
  const rows = viewportRows(first.nodes), deferred = rows.filter(row => row.extra.deferred).map(row => [row.attrs["data-k"], row.extra.deferred]);
  assert(deferred.length >= 40, `off-screen paragraphs are deferred (${deferred.length})`);
  const named = name => rows.find(row => row.attrs.id === name);
  assert(named("dock") && !named("dock").extra.deferred, "the fixed banner's wrapper is walked");
  assert(named("banner"), "the fixed banner travels in the first packet");
  assert.deepEqual(named("hidden")?.extra.deferred, [0, 0, 0, 0, null, 0, 0], "a display:none block is deferred without a box");
  assert(named("sheet") && !named("sheet").extra.deferred, "a block holding a sheet is walked");
  assert(!rows.some(row => !row.attrs["data-k"] && row.extra.deferred), "only page blocks are deferred (never the head)");
  const native = await evaluate(`JSON.stringify(${JSON.stringify(deferred)}.map(([k]) => { const r = document.querySelector('[data-k="' + k + '"]').getBoundingClientRect(); return [k, r.top < innerHeight * 1.25 && r.bottom > 0] }))`);
  assert(JSON.parse(native).every(([, near]) => !near), "nothing on screen is deferred");
  for (const [k, box] of deferred) if (box[4] !== null) { const r = JSON.parse(await evaluate(`JSON.stringify(document.querySelector('[data-k="${k}"]').getBoundingClientRect())`)); assert(Math.abs(r.y + 0 - box[1]) < 0.01 && Math.abs(r.height - box[3]) < 0.01 && box[4] === box[3], `${k} keeps its native box`); }
  // The deferred subtrees fill in later credits; every element arrives once, and the snapshot then completes.
  const filled = new Set(); let packet = first, guard = 0, elements = rows.filter(row => row.element).length;
  while (!packet.snapshot_complete && guard++ < 200) { packet = await viewer.next(300); for (const op of packet.ops) if (op.op === "children") { filled.add(op.id); elements += viewportRows(op.nodes).filter(row => row.element).length; } }
  assert.equal(packet.snapshot_complete, true);
  const ids = new Map(rows.map(row => [row.attrs["data-k"], row.id]));
  assert(deferred.every(([k]) => filled.has(ids.get(k))), "every deferred element gets its children");
  assert.equal(elements, await evaluate("document.getElementsByTagName('*').length"), "the same elements as a whole snapshot");
  const placeholders = await evaluate(placeholderExpression(deferred));
  assert.deepEqual({ placed: placeholders.placed, same: placeholders.same }, { placed: true, same: true }, "kept boxes hold the article's layout");
}));

test("MP-08/MP-10 protocol 629: a kept box the viewer cannot place is caught before a handover (tables, grids, shrink-to-fit, scrollbar)", async () => {
  const cases = {
    table: `<table><tr><td>a</td><td>b</td></tr>${Array.from({ length: 60 }, (_, i) => `<tr><td><div>cell ${i}</div></td><td><div>${i % 7 ? "x" : "a much longer cell text that widens the auto column"}</div></td></tr>`).join("")}</table>`,
    grid: `<div style="display:grid;grid-template-columns:auto auto">${Array.from({ length: 60 }, (_, i) => `<div><p>${i % 9 ? "g" : "grid auto track content that is wide"}</p></div><div><p>right ${i}</p></div>`).join("")}</div>`,
    shrink: `<div style="float:left;border:1px solid"><p>short</p>${Array.from({ length: 60 }, (_, i) => `<div><p>${i === 59 ? "the widest line of this floated shrink-to-fit box sits at the end" : "row " + i}</p></div>`).join("")}</div><p>after</p>`,
    scrollbar: `<div style="height:calc(100vh - 60px)">top</div><div><p>just</p><p>below</p><p>the fold</p></div>`,
    margins: `${Array.from({ length: 40 }, (_, i) => `<div><p style="margin:${i % 3 ? 0 : 30}px 0">collapsing ${i}</p></div>`).join("")}`,
  };
  for (const [name, body] of Object.entries(cases)) await mirrored(keyed({ n: 0 }, body), async ({ subscribe, evaluate }) => {
    const viewer = await subscribe({ viewport_first: true }), first = await viewer.next();
    const deferred = viewportRows(first.nodes).filter(row => row.extra.deferred).map(row => [row.attrs["data-k"], row.extra.deferred]);
    const { placed, same, shown } = await evaluate(placeholderExpression(deferred));
    assert(shown > 0, name);
    assert(!placed || same, `${name}: the viewer's check passes only when the screen is unchanged (${deferred.length} deferred)`);
    // Collapsing child margins travel with the kept box: the common heading wrapper still hands over early.
    if (name === "margins") assert(placed && deferred.length > 0, "collapsed margins are kept");
  });
});

test("MP-08/MP-10 protocol 629: an on-screen image travels while the off-screen rest still streams", () => mirrored(`<img src="/top.svg" width="40" height="40">${Array.from({ length: 3000 }, (_, i) => `<p>Paragraph ${i} <b>bold</b></p>`).join("")}`, async ({ subscribe }) => {
  const viewer = await subscribe({ viewport_first: true }), wanted = r => Buffer.from(r.data_base64, "base64").toString().includes("#0a0b0c");
  let packet = await viewer.next(), early = false, guard = 0;
  assert.equal(packet.snapshot_complete, false, "the off-screen part follows");
  while (!packet.snapshot_complete && guard++ < 400) { packet = await viewer.next(300); if (!packet.snapshot_complete && packet.resources.some(wanted)) early = true; }
  assert.equal(packet.snapshot_complete, true);
  assert(early, "the image the viewer's handover waits for does not queue behind every off-screen chunk");
}, { "/top.svg": { type: "image/svg+xml", body: svg("#0a0b0c") } }));

test("MP-08/MP-10 protocol 629: a followed link's new page gets its on-screen image while its off-screen rest streams", () => mirrored('<a href="/next">Go</a>', async ({ subscribe }) => {
  const viewer = await subscribe({ viewport_first: true }), wanted = r => Buffer.from(r.data_base64, "base64").toString().includes("#0d0e0f");
  const first = await viewer.next(), go = texts(first).get("Go");
  await viewer.input(first.sequence, { kind: "click", node_id: `n${Number(go.slice(1)) - 1}`, x: 5, y: 5 });
  let packet = await viewer.next(2000), early = false, guard = 0;
  while (!(packet.reset && packet.snapshot_viewport_complete) && guard++ < 100) packet = await viewer.next(300);
  assert.equal(packet.snapshot_complete, false, "the new page's off-screen part follows");
  while (!packet.snapshot_complete && guard++ < 400) { packet = await viewer.next(300); if (!packet.snapshot_complete && packet.resources.some(wanted)) early = true; }
  assert.equal(packet.snapshot_complete, true);
  assert(early, "the click on the previous page does not hold the new page's on-screen image behind every off-screen chunk");
}, { "/next": { type: "text/html", body: `<!doctype html><img src="/top.svg" width="40" height="40">${Array.from({ length: 3000 }, (_, i) => `<p>Paragraph ${i} <b>bold</b></p>`).join("")}` }, "/top.svg": { type: "image/svg+xml", body: svg("#0d0e0f") } }));
test("MP-08/MP-10 protocol 629: a subscription that did not ask gets a whole snapshot and no viewport flag", () => mirrored(article, async ({ next }) => {
  const first = await next();
  assert.equal(first.snapshot_complete, true);
  assert.equal("snapshot_viewport_complete" in first, false);
  assert.equal(viewportRows(first.nodes).some(row => row.extra.deferred), false);
}));
const visitedPage = `<style>a{color:rgb(0,0,255)} a:visited{color:rgb(255,0,0)}</style><p><a id="b" href="/b">B</a> <a id="other" href="/other">other</a> <a id="part" href="#section">part</a></p><h2 id="section">Part</h2>`;
const visitedRoutes = { "/b": { type: "text/html", body: "<!doctype html><p>B</p>" } };
const linkHrefs = packet => Object.fromEntries(viewportRows(packet.nodes).filter(row => ["b", "other", "part"].includes(row.attrs.id)).map(row => [row.attrs.id, [row.id, row.attrs.href]]));
// Follow A's link to B and come back, through the page itself (a renderer-started navigation, as a user's click).
const visitB = async evaluate => {
  const at = async (path, tries = 400) => { for (let i = 0; i < tries; i++) { if (await evaluate(`location.pathname === ${JSON.stringify(path)} && document.readyState === 'complete'`).catch(() => false) === true) return; await new Promise(resolve => setTimeout(resolve, 25)); } assert.fail(`MP-10: the page did not reach ${path}`); };
  await evaluate("document.getElementById('b').click(), true"); await at("/b");
  await evaluate("history.back(), true"); await at("/");
};
test("MP-08/MP-10 protocol 629: a link the kernel saw visited from this origin is sent as an empty href to a subscription that asked", () => mirrored(visitedPage, async ({ subscribe, evaluate }) => {
  await visitB(evaluate);
  const first = await (await subscribe({ visited_links: true })).next();
  const hrefs = linkHrefs(first);
  assert.equal(hrefs.b?.[1], "", "A's link to the visited B draws :visited in the viewer");
  assert.equal(hrefs.other?.[1], "#", "an unvisited link keeps :link");
  assert.equal(hrefs.part?.[1], "#", "an unvisited fragment link keeps :link");
  const old = await (await subscribe()).next();
  assert.equal(linkHrefs(old).b?.[1], "#", "a subscription that did not ask never receives an empty href");
}, visitedRoutes));

test("MP-08/MP-10 protocol 629: a link visited after the snapshot re-marks without a reset", () => mirrored(visitedPage, async ({ subscribe, evaluate }) => {
  const viewer = await subscribe({ visited_links: true });
  const first = await viewer.next(), part = linkHrefs(first).part;
  assert.equal(part?.[1], "#");
  await evaluate("document.getElementById('part').click(), true");
  let marked = null;
  for (let i = 0; i < 20 && !marked; i++) {
    const packet = await viewer.next(500);
    assert.notEqual(packet.reset, true, "re-marking needs no reset");
    marked = packet.ops.find(op => op.op === "attr" && op.id === part[0] && op.name === "href");
  }
  assert.deepEqual(marked, { op: "attr", id: part[0], name: "href", value: "" });
}, visitedRoutes));


for (const kind of ['click','scroll']) test(`MP-08/MP-10 resized iframe opaque region ${kind} reaches the real canvas`,()=>mirrored(
  '<style>body{margin:0}</style><iframe src="http://localhost:PORT/region" style="position:absolute;left:1400px;top:820px;width:100px;height:60px;border:0"></iframe><script>window.regionEvents=[];addEventListener("message",e=>regionEvents.push(e.data))</script>',
  async ({host,tab_id,generation,next,subscription_id,evaluate})=>{
    await host.request({op:'resize',tab_id,generation,width:1610,height:900});
    let packet,region;
    for(let n=0;n<12&&!region;n++){
      packet=await next(300,n===0);
      region=[...(host.mirror.streams.get(subscription_id).regionRecords??[])].find(([id,record])=>record.kind==='tile'&&Number(id.slice(1))>=1e9)?.[0];
    }
    assert(region,'the opaque canvas belongs to an attached cross-origin frame');
    const tab=host.tabs.get(tab_id),scope=host.mirror.streams.get(subscription_id).scope,lease=await host.mirror.v2.regions.resolve({mirror_subscription_id:subscription_id,generation,document_id:tab.document_id,sequence:packet.sequence,region_id:region},scope);
    try{
      const resolved=await lease.resolveInput({kind,sequence:packet.sequence,x:4,y:8,...(kind==='scroll'?{delta_x:0,delta_y:120}:{})},scope);
      assert.deepEqual([resolved.input.x,resolved.input.y],[1404,828]);
      await resolved.guard();
      await host.request({op:'input',tab_id,generation,document_id:tab.document_id,input:resolved.input});
      for(let n=0;n<40&&!(await evaluate('regionEvents.length'));n++)await settle(25);
      assert.deepEqual(await evaluate('regionEvents'),[kind]);
    }finally{await lease.close();}
  },{'/region':{type:'text/html',body:'<!doctype html><style>body{margin:0}</style><canvas width="100" height="60"></canvas><script>const c=document.querySelector("canvas");c.getContext("2d").fillRect(0,0,100,60);c.onclick=()=>parent.postMessage("click","*");c.onwheel=e=>{e.preventDefault();parent.postMessage("scroll","*")}</script>'}}
));
