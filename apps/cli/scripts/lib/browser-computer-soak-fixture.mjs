import http from "node:http"
import { performance } from "node:perf_hooks"
import { setTimeout as sleep } from "node:timers/promises"

export async function startActiveSoakFixture() {
  let marker = "SOAK-00000000"
  let bytes = 0
  const server = http.createServer((request, response) => {
    bytes += Buffer.byteLength(`${request.method ?? ""} ${request.url ?? ""}`)
    if (request.url === "/health") {
      response.writeHead(200, { "content-type": "text/plain", "cache-control": "no-store" })
      bytes += Buffer.byteLength(marker)
      return response.end(marker)
    }
    if (request.url?.startsWith("/mark?")) {
      marker = new URL(request.url, "http://127.0.0.1").searchParams.get("value") ?? marker
      response.writeHead(204, { "cache-control": "no-store" })
      return response.end()
    }
    response.writeHead(200, { "content-type": "text/html", "cache-control": "no-store" })
    const body = `<!doctype html><title>Chariox active soak</title><style>body{font:24px sans-serif;background:#14213d;color:#fff}main{padding:60px}input{font-size:28px;width:520px}.pulse{width:120px;height:120px;background:#fca311;animation:pulse 1s infinite alternate}@keyframes pulse{to{transform:translateX(320px);background:#2ec4b6}}</style><main><label>Soak marker <input id="marker" value="${marker}"></label><p id="echo">${marker}</p><div class="pulse"></div></main><script>const field=document.querySelector('#marker');field.addEventListener('input',()=>{document.querySelector('#echo').textContent=field.value;document.title=field.value;fetch('/mark?value='+encodeURIComponent(field.value)).catch(()=>{})})</script>`
    bytes += Buffer.byteLength(body)
    response.end(body)
  })
  await new Promise((resolve, reject) => server.once("error", reject).listen(0, "127.0.0.1", resolve))
  const port = server.address().port
  const url = `http://127.0.0.1:${port}/`
  return {
    url,
    port,
    metrics: () => ({ bytes }),
    close: () => new Promise((resolve, reject) => server.close((error) => error ? reject(error) : resolve())),
  }
}


export async function waitForFixtureMarker(url, marker, { timeoutMs = 2_000 } = {}) {
  const startedAt = performance.now()
  const deadline = startedAt + timeoutMs
  let attempts = 0
  let observedMarker = "[unobserved]"
  // Fill dispatches input; it does not await the page's asynchronous fetch.
  // Poll only the acknowledgement. Never repeat a completed browser mutation.
  while (performance.now() < deadline) {
    attempts += 1
    try {
      const response = await fetch(`${url}health`, {
        signal: AbortSignal.timeout(Math.max(1, Math.ceil(deadline - performance.now()))),
      })
      if (!response.ok) throw new Error(`Active fixture health read failed: HTTP ${response.status}`)
      observedMarker = await response.text()
    } catch (error) {
      if (error.name === "TimeoutError") break
      throw error
    }
    if (observedMarker === marker && performance.now() <= deadline) {
      return { attempts, elapsedMs: performance.now() - startedAt }
    }
    await sleep(Math.max(0, Math.min(50, deadline - performance.now())))
  }
  throw new Error(`Chromium mutation did not reach the active fixture within ${timeoutMs}ms: expected ${marker}, observed ${observedMarker.slice(0, 64)} (${attempts} reads)`)
}
