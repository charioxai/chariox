// MP-08/MP-10/MP-11: DOM mirror v2 observer and service on real Chromium
// (credential-free local fixtures, the real host controller and CDP).
import test from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { mkdtemp, rm } from "node:fs/promises";
import path from "node:path";
import { KernelBrowserHost } from "./kernel-browser-host.mjs";

assert(process.env.CHARIOX_MDACCESS_DRILL_ROOT, "MP-11: explicit disposable drill root required");

async function mirrored(html, run, routes = {}) {
  const root = await mkdtemp(path.join(process.env.CHARIOX_MDACCESS_DRILL_ROOT, "mirror2-"));
  const server = createServer((request, response) => {
    const port = String(server.address().port), route = routes[request.url];
    if (route) { response.setHeader("Content-Type", route.type); response.end(route.body.replaceAll("PORT", port)); return; }
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
      let applied = 0;
      const next = async (wait_ms = 0, reset = false) => { const packet = await host.request({ op: "mirror_next", subscription_id, generation: opened.generation, after_sequence: reset ? 0 : applied, drift_nodes: [], wait_ms }); applied = packet.sequence; return packet; };
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
