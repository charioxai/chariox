import http from "node:http"
import { createDrillEBarrier } from "./drill-e-barrier.mjs"

// MP-08/MP-10: fixture traffic only. Provider tools still use the normal kernel MCP path.
export async function startDrillESynchronizedFixture({ actors }) {
  const phases = new Map()
  let phase = null
  let armed = false
  let mutationTimerStarted = false
  const deliveries = []
  const streams = new Set()
  const held = () => phase !== null && phases.get(phase)?.evidence().pageReleaseReason === null
  const publish = () => {
    for (const stream of streams) stream.write(`data: ${JSON.stringify({ held: held() })}\n\n`)
  }
  const server = http.createServer(async (request, response) => {
    try {
      const url = new URL(request.url, "http://fixture.invalid")
      const parts = url.pathname.split("/").filter(Boolean)
      if (parts[0] === "ready") {
        const gate = phases.get(parts[1])
        if (!gate) throw new Error("phase not prepared")
        const waiting = gate.arrive(parts[2])
        if (gate.evidence().ready.length === 3) armed = true
        await waiting
        if (gate.evidence().releaseReason !== "all_actors_ready") {
          response.writeHead(409).end("MP-08/MP-10 ABORTED: do not call a Browser tool")
          return
        }
        // Role scheduling only; no provider adapter or runtime policy changes.
        const delay = phase === "reads"
          ? (parts[2] === actors[2] ? 5_000 : 0)
          : (parts[2] === actors[1] ? 0 : 3_000)
        if (delay) await new Promise(resolve => setTimeout(resolve, delay))
        deliveries.push({ phase: parts[1], actor: parts[2], atMs: Date.now() })
        response.end("MP-08/MP-10 READY: call the requested Chariox Browser tool now")
        return
      }
      if (parts[0] === "state") {
        response.setHeader("Content-Type", "application/json")
        response.end(JSON.stringify({ held: held() }))
        return
      }
      if (parts[0] === "events") {
        response.writeHead(200, { "Content-Type": "text/event-stream", "Cache-Control": "no-cache" })
        streams.add(response)
        response.on("close", () => streams.delete(response))
        publish()
        return
      }
      if (parts[0] === "page" && ["same", "other"].includes(parts[1])) {
        if (armed && phase === "reads") await phases.get(phase).hold(parts[1])
        response.setHeader("Content-Type", "text/html")
        // The production CDP snapshot captures AX/DOM before compacting to its
        // unchanged 5000-node result bound. A large fixture makes that read
        // observable after admission, without blocking preflight reconciliation.
        const probes = parts[1] === "same"
          ? Array.from({ length: 30_000 }, (_, index) => `<button>Drill E probe ${index}</button>`).join("")
          : "";
        response.end(`<title>MP-08/MP-10 Drill E ${parts[1]}</title><h1>Drill E probe</h1>
          <button id="held-mutation" onclick="document.getElementById('count').textContent=++window.clicks">Drill E held mutation</button>
          <output id="count">0</output>${probes}<script>
          window.clicks=0;
          const events=new EventSource('/events');
          events.onmessage=event=>{document.getElementById('held-mutation').disabled=JSON.parse(event.data).held};
          </script>`)
        return
      }
      response.writeHead(404).end()
    } catch {
      response.writeHead(409).end("MP-08/MP-10 invalid fixture barrier request")
    }
  })
  await new Promise((resolve, reject) => {
    server.once("error", reject)
    server.listen(0, "0.0.0.0", resolve)
  })
  const port = server.address().port
  return {
    port,
    browserOrigin: `http://host.docker.internal:${port}`,
    beforePhase(next) {
      if (phases.has(next)) throw new Error("MP-08/MP-10 duplicate fixture phase")
      phase = next
      armed = false
      mutationTimerStarted = false
      phases.set(next, createDrillEBarrier({ actors }))
      publish()
    },
    tick(environment) {
      if (phase === null || mutationTimerStarted) return
      if (!environment.actions.some(action => action.kind === "click" && action.state === "running")) return
      mutationTimerStarted = true
      void phases.get(phase).hold("actionability").then(publish)
    },
    promptPrefix(next, actor) {
      const endpoint = `http://127.0.0.1:${port}/ready/${next}/${actor}`
      const code = `import urllib.request; print(urllib.request.urlopen(${JSON.stringify(endpoint)}, timeout=70).read().decode())`
      // Only generated safe identities enter this command; URL is passed as Python text.
      const command = "python3 -c '" + code.replaceAll("'", "'\\''") + "'"
      return `MP-08/MP-10 controlled concurrency fixture. FIRST run exactly this shell command and wait for READY: ${command}. If it errors or says ABORTED, stop and report the failure. Only after READY, immediately use the requested Chariox tool without any other tool or commentary. `
    },
    observed(next, actions) {
      phases.get(next).releasePages("observed_overlap", actions.map(action => action.action_id))
      armed = false
      publish()
    },
    evidence() { return Object.fromEntries([...phases].map(([name, gate]) => [name,
      { ...gate.evidence(), deliveries: deliveries.filter(item => item.phase === name) }])) },
    close() {
      armed = false
      for (const gate of phases.values()) gate.close()
      publish()
    },
    async stop() {
      this.close()
      server.closeAllConnections()
      await new Promise(resolve => server.close(resolve))
    },
  }
}
