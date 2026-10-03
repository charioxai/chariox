import assert from "node:assert/strict"
import test from "node:test"

import { roomSharedBrowserStateFixtureGateway } from "./room-shared-browser-state-proxy.mjs"

test("loopback secure-context fixture requires one routable slice network", () => {
  assert.equal(roomSharedBrowserStateFixtureGateway({ bridge: { Gateway: "172.19.0.1" } }), "172.19.0.1")
  for (const networks of [null, {}, { first: { Gateway: "127.0.0.1" } },
    { first: { Gateway: "0.0.0.0" } }, { first: { Gateway: "invalid" } },
    { first: { Gateway: "172.19.0.1" }, second: { Gateway: "172.20.0.1" } }]) {
    assert.throws(() => roomSharedBrowserStateFixtureGateway(networks), /one slice Docker network|usable gateway/)
  }
})
