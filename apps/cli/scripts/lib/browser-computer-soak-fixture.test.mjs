import assert from "node:assert/strict"
import http from "node:http"
import test from "node:test"
import { startActiveSoakFixture, waitForFixtureMarker } from "./browser-computer-soak-fixture.mjs"

// MP-08 / MP-10: a fill completes before its input handler's async request.
test("soak waits for delayed fixture acknowledgement without replaying the mutation", async () => {
  const fixture = await startActiveSoakFixture()
  let markRequests = 0
  let releaseMark
  const markGate = new Promise((resolve) => { releaseMark = resolve })
  const proxy = http.createServer(async (request, response) => {
    if (request.url.startsWith("/mark?")) {
      markRequests += 1
      await markGate
    }
    const upstream = await fetch(new URL(request.url, fixture.url))
    response.writeHead(upstream.status)
    response.end(await upstream.text())
  })
  await new Promise((resolve) => proxy.listen(0, "127.0.0.1", resolve))
  const url = `http://127.0.0.1:${proxy.address().port}/`
  let acknowledgement
  let releaseTimer
  try {
    acknowledgement = fetch(`${url}mark?value=SOAK-00000536`)
    // Hold the acknowledgement across the first health read, deterministically.
    releaseTimer = setTimeout(releaseMark, 200)
    const proof = await waitForFixtureMarker(url, "SOAK-00000536")
    assert.ok(proof.attempts > 1)
    assert.ok(proof.elapsedMs >= 150)
    await acknowledgement
    assert.equal(markRequests, 1)
  } finally {
    clearTimeout(releaseTimer)
    releaseMark()
    await acknowledgement
    await new Promise((resolve) => proxy.close(resolve))
    await fixture.close()
  }
})

test("soak fails within its deadline when the mutation never arrives", async () => {
  const fixture = await startActiveSoakFixture()
  try {
    await assert.rejects(
      waitForFixtureMarker(fixture.url, "SOAK-00000536", { timeoutMs: 120 }),
      /Chromium mutation did not reach the active fixture.*expected SOAK-00000536.*observed SOAK-00000000/,
    )
    assert.equal(await fetch(`${fixture.url}health`).then((r) => r.text()), "SOAK-00000000")
  } finally {
    await fixture.close()
  }
})

test("soak accepts an acknowledged marker on the first read", async () => {
  const fixture = await startActiveSoakFixture()
  try {
    await fetch(`${fixture.url}mark?value=SOAK-00000536`)
    const proof = await waitForFixtureMarker(fixture.url, "SOAK-00000536")
    assert.equal(proof.attempts, 1)
  } finally {
    await fixture.close()
  }
})

test("soak rejects a failed health response even when its body matches", async () => {
  const server = http.createServer((_request, response) => {
    response.writeHead(503)
    response.end("SOAK-00000536")
  })
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve))
  try {
    await assert.rejects(waitForFixtureMarker(`http://127.0.0.1:${server.address().port}/`, "SOAK-00000536"), /HTTP 503/)
  } finally {
    await new Promise((resolve) => server.close(resolve))
  }
})

test("soak bounds an unresponsive health request by the acknowledgement deadline", async () => {
  const server = http.createServer(() => {})
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve))
  try {
    await assert.rejects(
      waitForFixtureMarker(`http://127.0.0.1:${server.address().port}/`, "SOAK-00000536", { timeoutMs: 100 }),
      /Chromium mutation did not reach the active fixture within 100ms.*observed \[unobserved\]/,
    )
  } finally {
    server.closeAllConnections()
    await new Promise((resolve) => server.close(resolve))
  }
})
