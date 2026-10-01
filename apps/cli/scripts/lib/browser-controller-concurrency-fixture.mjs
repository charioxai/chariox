// MP-08/MP-10: deterministic page gates; no mocked controller or provider.
import http from "node:http";

export async function startBrowserConcurrencyFixture({ host = "0.0.0.0", port = 0 } = {}) {
  const streams = new Set();
  let epoch = 0;
  let held = { same: false, other: false };
  let readsReleased = false;
  const probes = [];
  const effects = [];
  const state = () => ({ epoch, held, readsReleased, probes, effects });
  const publish = () => {
    for (const response of streams) response.write(`data: ${JSON.stringify(state())}\n\n`);
  };
  const server = http.createServer((request, response) => {
    const url = new URL(request.url, "http://fixture.test");
    if (url.pathname === "/events") {
      response.writeHead(200, { "Content-Type": "text/event-stream", "Cache-Control": "no-cache" });
      streams.add(response); response.on("close", () => streams.delete(response)); publish(); return;
    }
    if (url.pathname === "/state") {
      response.setHeader("Content-Type", "application/json"); response.end(JSON.stringify(state())); return;
    }
    if (url.pathname === "/control") {
      epoch += 1;
      held = { same: url.searchParams.get("same") === "hold", other: url.searchParams.get("other") === "hold" };
      readsReleased = url.searchParams.get("reads") === "release";
      publish(); response.end(JSON.stringify(state())); return;
    }
    if (url.pathname === "/probe" || url.pathname === "/effect") {
      const event = { ...Object.fromEntries(url.searchParams), atMs: Date.now() };
      (url.pathname === "/probe" ? probes : effects).push(event);
      response.end("ok"); return;
    }
    const tab = url.pathname.slice(1);
    if (!["same", "other"].includes(tab)) { response.writeHead(404).end(); return; }
    response.setHeader("Content-Type", "text/html");
    response.end(`<title>MP-08/MP-10 ${tab}</title><h1>Concurrency ${tab}</h1>
      <button id="button" onclick="fetch('/effect?tab=${tab}&kind=click'); count.textContent=++window.clicks">Click ${tab}</button>
      <input id="note" aria-label="Note ${tab}" oninput="fetch('/effect?tab=${tab}&kind=type&text='+encodeURIComponent(this.value))">
      <output id="count">0</output><script>
      window.clicks=0; let epoch=-1, probes=new Set();
      const report=(kind,id)=>{const key=epoch+':'+kind+':'+id;if(probes.has(key))return;probes.add(key);fetch('/probe?tab=${tab}&kind='+kind+'&id='+encodeURIComponent(id)+'&epoch='+epoch)};
      const query=Document.prototype.querySelector;
      Document.prototype.querySelector=function(selector){if(selector.startsWith('#read-'))report('read',selector);return query.call(this,selector)};
      const rect=Element.prototype.getBoundingClientRect;
      Element.prototype.getBoundingClientRect=function(){if(this.id==='button'||this.id==='note')report('action',this.id);return rect.call(this)};
      const events=new EventSource('/events');
      events.onmessage=event=>{const state=JSON.parse(event.data);epoch=state.epoch;
        button.disabled=state.held.${tab};note.disabled=state.held.${tab};
        for(const id of ['read-a','read-b','read-c']){let node=document.getElementById(id);
          if(state.readsReleased&&!node){node=document.createElement('div');node.id=id;node.textContent=id;document.body.append(node)}
          if(!state.readsReleased&&node)node.remove()}
        fetch('/probe?tab=${tab}&kind=ready&id=page&epoch='+epoch);
      };
      </script>`);
  });
  await new Promise((resolve, reject) => { server.once("error", reject); server.listen(port, host, resolve); });
  return { port: server.address().port, state,
    async close() { server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); } };
}

if (process.argv[1]?.endsWith("browser-controller-concurrency-fixture.mjs")) {
  const fixture = await startBrowserConcurrencyFixture({ port: Number(process.env.CONCURRENCY_FIXTURE_PORT ?? 60222) });
  const browser = "http://127.0.0.1:9222";
  for (const tab of ["same", "other"]) {
    await fetch(`${browser}/json/new?${encodeURIComponent(`http://127.0.0.1:${fixture.port}/${tab}`)}`, { method: "PUT" });
  }
  console.log(JSON.stringify({ mp_items: ["MP-08", "MP-10"], port: fixture.port }));
  for (const signal of ["SIGINT", "SIGTERM"]) process.once(signal, () => { void fixture.close().then(() => process.exit()); });
}
