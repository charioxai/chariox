// MP-08/MP-10/MP-11: opt-in native Chromium; no mocked DOM or Input dispatch.
import test from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { mkdtemp, rm } from "node:fs/promises";
import path from "node:path";
import { KernelBrowserHost } from "./kernel-browser-host.mjs";

assert(process.env.CHARIOX_MDACCESS_DRILL_ROOT, "MP-10: explicit disposable drill root required");

async function native(markup, operation) {
  const root = await mkdtemp(path.join(process.env.CHARIOX_MDACCESS_DRILL_ROOT, "native-input-"));
  const server = createServer((_request, response) => {
    response.setHeader("Content-Type", "text/html");
    response.end(`<!doctype html>${markup}<p id="result">submitted=0 releases=0</p>
      <script>
      let submitted=0, releases=0;
      const status=()=>document.querySelector('#result').textContent=
        'submitted='+submitted+' releases='+releases+' focus='+document.activeElement.id;
      document.addEventListener('submit',e=>{e.preventDefault();submitted++;status()});
      document.addEventListener('keyup',e=>{if(e.key==='Tab')releases++;status()});
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
    await operation({ host, bound, input, status, events });
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

test("MP-08 P1: retained Enter can activate a classified ordinary submit", () => native(`<form>${field}<button>Search</button></form>`, async ({ input, status }) => {
  await input({ kind: "key", key: "Enter" }, { _retained_agent: true });
  assert.match(await status(), /^submitted=1/);
}));

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
