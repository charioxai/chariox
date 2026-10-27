import assert from "node:assert/strict"
import { createServer } from "node:http"
import { once } from "node:events"
import test from "node:test"
import { browserStateFixtureGateway, startBrowserStateFixtureProxy } from "./browser-state-fixture-proxy.mjs"

test("browser fixture reaches the Docker host through the slice gateway", () => {
  assert.equal(browserStateFixtureGateway({ bridge: { Gateway: "172.17.0.1" } }), "172.17.0.1")
  assert.throws(() => browserStateFixtureGateway({ bridge: { Gateway: "" } }), /gateway/)
  assert.throws(() => browserStateFixtureGateway({
    bridge: { Gateway: "172.17.0.1" },
    other: { Gateway: "172.18.0.1" },
  }), /one Docker network/)
})

test("loopback fixture preserves authenticated requests and response bytes", async () => {
  const fixture = createServer((request, response) => {
    assert.equal(request.url, "/mail/inbox")
    assert.equal(request.headers.cookie, "fixture-session=accepted")
    response.setHeader("set-cookie", "fixture-refresh=next; HttpOnly; SameSite=Lax")
    response.end("Fixture Grüße\n")
  })
  fixture.listen(0, "127.0.0.1")
  await once(fixture, "listening")
  let proxy
  try {
    proxy = await startBrowserStateFixtureProxy({ port: 0, upstreamHost: "127.0.0.1", upstreamPort: fixture.address().port })
    assert.equal(proxy.address.address, "127.0.0.1")
    const response = await fetch(`http://127.0.0.1:${proxy.address.port}/mail/inbox`, {
      headers: { cookie: "fixture-session=accepted" },
    })
    assert.equal(response.headers.get("set-cookie"), "fixture-refresh=next; HttpOnly; SameSite=Lax")
    assert.equal(await response.text(), "Fixture Grüße\n")
  } finally {
    await proxy?.close()
    await new Promise(resolve => fixture.close(resolve))
  }
})
