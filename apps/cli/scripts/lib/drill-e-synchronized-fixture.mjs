import http from "node:http"
import { createDrillEBarrier } from "./drill-e-barrier.mjs"

// MP-08/MP-10: fixture traffic only. Provider tools still use the normal kernel MCP path.
export async function startDrillESynchronizedFixture({ actors }) {
  const phases = new Map()
  let phase = null
  let armed = false
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
        response.end("MP-08/MP-10 READY: call the requested Chariox Browser tool now")
        return
      }
      if (parts[0] === "read-hold") {
        if (armed && phase === "reads") await phases.get(phase).hold("read-evaluation")
        response.end("released")
        return
      }
      if (parts[0] === "page" && ["same", "other"].includes(parts[1])) {
        if (armed) await phases.get(phase).hold(parts[1])
        response.setHeader("Content-Type", "text/html")
        response.end(`<title>MP-08/MP-10 Drill E ${parts[1]}</title><h1>Drill E probe</h1>
          <script>
          const nativeVisibility = Object.getOwnPropertyDescriptor(Document.prototype, 'visibilityState');
          Object.defineProperty(document, 'visibilityState', {get() {
            const request = new XMLHttpRequest();
            request.open('GET', '/read-hold', false); request.send();
            return nativeVisibility.get.call(document);
          }});
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
      phases.set(next, createDrillEBarrier({ actors }))
    },
    promptPrefix(next, actor) {
      const endpoint = `http://127.0.0.1:${port}/ready/${next}/${actor}`
      const code = `import urllib.request; print(urllib.request.urlopen(${JSON.stringify(endpoint)}, timeout=70).read().decode())`
      // Only generated safe identities enter this command; URL is passed as Python text.
      const command = "python3 -c '" + code.replaceAll("'", "'\\''") + "'"
      return `MP-08/MP-10 controlled concurrency fixture. FIRST run exactly this shell command and wait for READY: ${command}. Immediately after READY, use the requested Chariox tool without any other tool or commentary. `
    },
    observed(next, actions) {
      phases.get(next).releasePages("observed_overlap", actions.map(action => action.action_id))
      armed = false
    },
    evidence() { return Object.fromEntries([...phases].map(([name, gate]) => [name, gate.evidence()])) },
    close() {
      armed = false
      for (const gate of phases.values()) gate.close()
    },
    async stop() {
      this.close()
      server.closeAllConnections()
      await new Promise(resolve => server.close(resolve))
    },
  }
}
