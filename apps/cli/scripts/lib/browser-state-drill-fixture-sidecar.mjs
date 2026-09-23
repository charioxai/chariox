import assert from "node:assert/strict"
import { randomUUID } from "node:crypto"
import { readFile } from "node:fs/promises"
import http from "node:http"
import { basename } from "node:path"
import { fileURLToPath } from "node:url"

import { startBrowserComputerFixture } from "./browser-computer-fixture.mjs"

const sidecarDir = "/tmp/chariox-m20-fixture"
const controlPort = 4322
const fixtureSources = [
  new URL("./browser-state-drill-fixture-sidecar.mjs", import.meta.url),
  new URL("./browser-computer-fixture.mjs", import.meta.url),
  new URL("./browser-computer-fixture-mail.mjs", import.meta.url),
]

export function browserStateFixtureSidecarArgs({ name, image, runId }) {
  assert.match(name, /^m20-fixture-[a-z0-9-]+$/)
  assert.match(runId, /^m20-docker-state-[A-Za-z0-9-]+$/)
  assert.ok(image && !/\s/.test(image), "sidecar image must be explicit")
  return [
    "run", "-d", "--rm", "--name", name,
    "--label", `io.chariox.drill-run=${runId}`,
    "--memory", "256m", "--memory-swap", "256m", "--cpus", "0.5",
    "--pids-limit", "64", "--security-opt", "no-new-privileges",
    "--cap-drop", "ALL", "--network", "host", "--entrypoint", "/bin/sleep",
    image, "infinity",
  ]
}

export async function startBrowserStateFixtureSidecar({
  docker, dockerText, runCommand, image, runId, port, account, password,
}) {
  assert.equal(port, 4321, "rootless M20 fixture uses the fixed slice loopback port")
  assert.ok(account && password, "sidecar fixture requires synthetic test credentials")
  const name = `m20-fixture-${process.pid}-${randomUUID().slice(0, 8)}`
  const imageId = JSON.parse(await dockerText(["image", "inspect", image, "--format", "{{json .Id}}"])).trim()
  assert.match(imageId, /^sha256:[a-f0-9]{64}$/)
  let containerId = null
  let closed = false

  async function control(method, pathname) {
    const result = await dockerText([
      "exec", "-u", "slice", name, "curl", "--fail", "--silent", "--show-error",
      "--max-time", "2", "--request", method,
      `http://127.0.0.1:${controlPort}${pathname}`,
    ])
    return result.trim()
  }

  async function cleanup() {
    if (!containerId) return
    const inspected = await runCommand("docker", ["container", "inspect", containerId], { timeoutMs: 10_000 })
    if (inspected.code !== 0) return
    const current = JSON.parse(inspected.stdout)[0]
    assert.equal(current?.Id, containerId, "fixture sidecar identity changed before cleanup")
    assert.equal(current?.Name, `/${name}`, "fixture sidecar name changed before cleanup")
    assert.equal(current?.Image, imageId, "fixture sidecar image changed before cleanup")
    assert.equal(current?.Config?.Labels?.["io.chariox.drill-run"], runId,
      "fixture sidecar ownership label changed before cleanup")
    await docker(["rm", "-f", containerId])
    const after = await runCommand("docker", ["container", "inspect", containerId], { timeoutMs: 10_000 })
    assert.notEqual(after.code, 0, "fixture sidecar survived cleanup")
  }

  try {
    containerId = (await dockerText(browserStateFixtureSidecarArgs({ name, image, runId }))).trim()
    assert.match(containerId, /^[a-f0-9]{64}$/, "Docker did not return a sidecar identity")
    await docker(["exec", "-u", "slice", name, "mkdir", "-m", "700", sidecarDir])
    for (const source of fixtureSources) {
      await docker(["cp", fileURLToPath(source), `${name}:${sidecarDir}/${basename(source.pathname)}`])
    }
    await dockerText(["exec", "-i", "-u", "slice", name, "sh", "-c",
      `umask 077; tee ${sidecarDir}/config.json >/dev/null`], {
      stdin: JSON.stringify({ port, account, password }),
    })
    await docker(["exec", "-d", "-u", "slice", "-e", "M20_FIXTURE_SIDECAR_CHILD=1", name,
      "sh", "-c", `node ${sidecarDir}/browser-state-drill-fixture-sidecar.mjs >${sidecarDir}/server.log 2>&1`])
    let ready = false
    for (let attempt = 0; attempt < 30; attempt += 1) {
      const result = await runCommand("docker", [
        "exec", "-u", "slice", name, "curl", "--fail", "--silent", "--max-time", "1",
        "--output", "/dev/null", `http://127.0.0.1:${controlPort}/health`,
      ], { timeoutMs: 3_000 })
      if (result.code === 0) { ready = true; break }
      await new Promise(resolve => setTimeout(resolve, 200))
    }
    assert.ok(ready, "rootless fixture sidecar did not become healthy")
    return {
      origin: `http://127.0.0.1:${port}`,
      health: async () => { await control("GET", "/health") },
      readMessages: async () => JSON.parse(await control("GET", "/messages")),
      invalidateSessions: async () => JSON.parse(await control("POST", "/invalidate")).count,
      close: async () => {
        if (closed) return
        await control("POST", "/close")
        closed = true
      },
      cleanup,
      containerId,
      imageId,
    }
  } catch (error) {
    try { await cleanup() } catch (cleanupError) { throw new AggregateError([error, cleanupError]) }
    throw error
  }
}

async function runSidecarServer() {
  const { port, account, password } = JSON.parse(await readFile(`${sidecarDir}/config.json`, "utf8"))
  const fixture = await startBrowserComputerFixture({ host: "0.0.0.0", port, account, password })
  let closed = false
  const control = http.createServer(async (request, response) => {
    const key = `${request.method} ${request.url}`
    let value
    if (key === "GET /health") {
      if (closed) { response.statusCode = 503; response.end(); return }
      response.statusCode = 204
      response.end()
      return
    }
    if (key === "GET /messages") value = fixture.messages
    else if (key === "POST /invalidate") value = { count: fixture.invalidateSessions() }
    else if (key === "POST /close") {
      if (!closed) { await fixture.close(); closed = true }
      value = { closed: true }
    } else {
      response.statusCode = 404
      response.end()
      return
    }
    response.setHeader("content-type", "application/json")
    response.end(JSON.stringify(value))
  })
  control.maxConnections = 4
  await new Promise((resolve, reject) => {
    control.once("error", reject)
    control.listen(controlPort, "127.0.0.1", resolve)
  })
}

if (process.env.M20_FIXTURE_SIDECAR_CHILD === "1") await runSidecarServer()
