// MP-08/MP-10: credential-free Docker API fixture, isolated from the host daemon.
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { once } from "node:events"
import { join } from "node:path"

export async function missingBrokerDockerFixture(context, root, container) {
  const socket = join(root, "docker.sock")
  const child = spawn(process.execPath, ["--input-type=module", "-e", `
import http from "node:http"
const server = http.createServer((request, response) => {
  response.setHeader("API-Version", "1.47")
  const path = request.url.replace(/^\\/v[0-9.]+/, "")
  if (path === "/_ping") return response.end("OK")
  const kind = path === "/containers/" + process.argv[2] + "/json" ? "container"
    : path === "/volumes/" + process.argv[2] + "-home" ? "volume" : null
  response.writeHead(kind ? 404 : 500, { "Content-Type": "application/json" })
  response.end(JSON.stringify({ message: kind
    ? "No such " + kind + ": " + process.argv[2] + (kind === "volume" ? "-home" : "")
    : "unexpected synthetic Docker request" }))
})
server.listen(process.argv[1], () => process.stdout.write("ready\\n"))
`, socket, container], { stdio: ["ignore", "pipe", "pipe"] })
  context.after(async () => {
    if (child.exitCode === null && child.signalCode === null) {
      const closed = once(child, "close")
      child.kill("SIGTERM")
      await closed
    }
  })
  const [ready] = await once(child.stdout, "data", { signal: AbortSignal.timeout(3000) })
  assert.equal(ready.toString(), "ready\n")
  return `unix://${socket}`
}
