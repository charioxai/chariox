import assert from "node:assert/strict"
import test from "node:test"

import { createRoomSharedBrowserStateFixture } from "./room-shared-browser-state-fixture.mjs"

test("local authenticated Browser fixture changes from one-time seed to read-only verification", () => {
  const fixture = createRoomSharedBrowserStateFixture({ generation: "room-state-test-1" })
  const seededPage = fixture.pageScript()
  assert.equal(fixture.mode, "seed")
  assert.doesNotThrow(() => new Function(seededPage), "generated Browser seed script must parse")
  for (const api of ["fetch(config.cookieUrl", "localStorage.setItem", "indexedDB.open", "caches.open",
    "serviceWorker.register"]) assert.ok(seededPage.includes(api), `fixture seed omitted ${api}`)
  assert.ok(seededPage.includes("ROOM_BROWSER_AUTH=PASS"))
  assert.ok(seededPage.includes("await fetch(config.seedCompleteUrl"),
    "the fixture must acknowledge read-only verification mode before reporting the seed")
  assert.equal([...seededPage.matchAll(/render\(\);/g)].length, 1,
    "the fixture must render only after its async reads finish")

  const denied = makeResponse()
  fixture.handleRequest({ method: "GET", url: "/room-state-auth?generation=room-state-test-1", headers: {} }, denied)
  assert.equal(denied.status, 204)
  const cookie = denied.headers["set-cookie"]
  assert.match(cookie, /HttpOnly/)

  const authenticated = makeResponse()
  fixture.handleRequest({ method: "GET", url: "/room-state-auth?generation=room-state-test-1",
    headers: { cookie: cookie.split(";")[0] } }, authenticated)
  assert.equal(authenticated.body, "ROOM_BROWSER_AUTH=PASS")

  const seeded = makeResponse()
  fixture.handleRequest({ method: "POST", url: "/room-state-seeded?generation=room-state-test-1",
    headers: { cookie: cookie.split(";")[0] } }, seeded)
  assert.equal(seeded.status, 204)
  assert.equal(fixture.mode, "verify")

  const verifyPage = fixture.pageScript()
  for (const write of ["localStorage.setItem", ".put(config.databaseValue", ".put(config.cacheKey",
    "serviceWorker.register"]) assert.ok(!verifyPage.includes(write), `verify script reseeds ${write}`)
  assert.ok(verifyPage.includes("getRegistration"))
  assert.ok(verifyPage.includes("request.transaction.abort()"),
    "the verifier must not create a missing IndexedDB during its read")
  assert.equal([...verifyPage.matchAll(/render\(\);/g)].length, 1,
    "the restored verifier must render only after its async reads finish")
  const worker = makeResponse()
  fixture.handleRequest({ method: "GET", url: "/room-state-service-worker.js?generation=room-state-test-1", headers: {} }, worker)
  assert.doesNotThrow(() => new Function(worker.body), "generated service worker must parse")
  assert.ok(!worker.body.includes("storage.put(cacheKey"), "verification service worker must not recreate its marker")
})

test("fixture refuses invalid seed completion and never seeds an absent cookie after verification starts", () => {
  const fixture = createRoomSharedBrowserStateFixture({ generation: "room-state-test-2" })
  const rejected = makeResponse()
  fixture.handleRequest({ method: "POST", url: "/room-state-seeded?generation=wrong", headers: {} }, rejected)
  assert.equal(rejected.status, 403)
  assert.equal(fixture.mode, "seed")

  const seeded = makeResponse()
  fixture.handleRequest({ method: "POST", url: "/room-state-seeded?generation=room-state-test-2",
    headers: { cookie: "chariox_room_room-state-test-2=room-state-test-2-authenticated-fixture" } }, seeded)
  assert.equal(seeded.status, 204)
  const absentCookie = makeResponse()
  fixture.handleRequest({ method: "GET", url: "/room-state-auth?generation=room-state-test-2", headers: {} }, absentCookie)
  assert.equal(absentCookie.status, 401)
  assert.equal(absentCookie.headers["set-cookie"], undefined)
})

function makeResponse() {
  return {
    status: 0,
    headers: {},
    body: "",
    writeHead(status, headers = {}) { this.status = status; this.headers = headers; return this },
    end(body = "") { this.body = body; return this },
  }
}
