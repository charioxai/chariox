import assert from "node:assert/strict"
import test from "node:test"
import { mkdir, mkdtemp, realpath, rm, stat, symlink } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import {
  captureProviderAfter,
  captureProviderBefore,
  writeCaptureOutput,
} from "./path1-provider-rebuild-capture.mjs"

const TOKEN = "fixture-only-token"
const REQUESTED_AT = "2026-09-26T05:00:00.000Z"
const CAPTURED_AT = new Date("2026-09-26T05:00:04.000Z")

function server(overrides = {}) {
  return {
    id: 123,
    image: { id: 456, description: "DO-NOT-RETAIN-IMAGE-BODY" },
    server_type: { id: 4, name: "cx-example" },
    location: { id: 5, name: "fsn1" },
    datacenter: { id: 6, name: "fsn1-dc1" },
    ...overrides,
  }
}

function action(overrides = {}) {
  return {
    id: 88,
    command: "rebuild",
    status: "success",
    started: "2026-09-26T05:00:01Z",
    finished: "2026-09-26T05:00:03Z",
    resources: [{ id: 123, type: "server" }],
    error: null,
    ...overrides,
  }
}

function actionPage(page, actions, lastPage = 1) {
  return {
    actions,
    meta: { pagination: {
      page,
      per_page: 50,
      previous_page: page === 1 ? null : page - 1,
      next_page: page < lastPage ? page + 1 : null,
      last_page: lastPage,
      total_entries: actions.length,
    } },
  }
}

function jsonResponse(value) {
  return new Response(JSON.stringify(value), {
    status: 200,
    headers: { "content-type": "application/json" },
  })
}

function fixture({ serverViews = [server(), server()], pages = [actionPage(1, [action()])] } = {}) {
  const requests = []
  let serverIndex = 0
  const fetchImpl = async (url, init) => {
    requests.push({ url: new URL(url), init })
    const parsed = new URL(url)
    if (parsed.pathname === "/v1/servers/123") {
      const value = serverViews[Math.min(serverIndex, serverViews.length - 1)]
      serverIndex += 1
      return jsonResponse({ server: value })
    }
    if (parsed.pathname === "/v1/servers/123/actions") {
      const page = Number(parsed.searchParams.get("page"))
      return jsonResponse(pages[page - 1] ?? actionPage(page, [], page))
    }
    throw new Error("fixture received an unexpected URL")
  }
  return { requests, fetchImpl, serverViews, pages }
}

function afterInput(api, overrides = {}) {
  return {
    serverId: "123",
    imageId: "456",
    actionId: "88",
    requestedAt: REQUESTED_AT,
    token: TOKEN,
    fetchImpl: api.fetchImpl,
    now: () => CAPTURED_AT,
    ...overrides,
  }
}

function assertRequestsAreReadOnly(requests) {
  for (const { url, init } of requests) {
    assert.equal(url.origin, "https://api.hetzner.cloud")
    assert.equal(init.method, "GET")
    assert.equal(init.redirect, "error")
    assert.equal(init.headers.authorization, `Bearer ${TOKEN}`)
    assert.equal(init.headers.accept, "application/json")
  }
}

test("before capture derives server, image, type, location, and datacenter IDs", async () => {
  const api = fixture()
  const capture = await captureProviderBefore({
    serverId: "123", token: TOKEN, fetchImpl: api.fetchImpl, now: () => CAPTURED_AT,
  })
  assert.deepEqual(capture, {
    schema: "chariox.path1-provider-rebuild-capture/v1",
    provider: "hetzner-cloud",
    kind: "before",
    capturedAt: "2026-09-26T05:00:04.000Z",
    server: { serverId: "123", imageId: "456", serverTypeId: "4", locationId: "5", datacenterId: "6" },
  })
  assertRequestsAreReadOnly(api.requests)
  assert.equal(api.requests.length, 1)
})

test("after capture retains exact successful rebuild fields and rechecks server identity", async () => {
  const api = fixture()
  const capture = await captureProviderAfter(afterInput(api))
  assert.deepEqual(capture.action, {
    actionId: "88", command: "rebuild", status: "success", error: null,
    started: "2026-09-26T05:00:01Z", finished: "2026-09-26T05:00:03Z",
  })
  assert.deepEqual(capture.serverBefore, capture.serverAfter)
  assert.equal(capture.requestedAt, REQUESTED_AT)
  assert.equal(capture.ok, undefined)
  assert.equal(capture.verdict, undefined)
  assertRequestsAreReadOnly(api.requests)
  assert.deepEqual(api.requests.map(({ url }) => url.pathname), [
    "/v1/servers/123", "/v1/servers/123/actions", "/v1/servers/123",
  ])
  assert.ok(!JSON.stringify(capture).includes("DO-NOT-RETAIN"))
})

for (const [name, mutate, message] of [
  ["wrong server ID", (api) => { api.serverViews[0].id = 124 }, /wrong server/],
  ["wrong expected image", (_api, input) => { input.imageId = "999" }, /wrong image/],
  ["wrong action ID", (api) => { api.pages[0] = actionPage(1, [action({ id: 89 })]) }, /not found/],
  ["wrong action server binding", (api) => { api.pages[0] = actionPage(1, [action({ resources: [{ id: 999, type: "server" }] })]) }, /bound to the requested server/],
  ["wrong command", (api) => { api.pages[0] = actionPage(1, [action({ command: "reboot" })]) }, /not a rebuild/],
  ["stale start time", (api) => { api.pages[0] = actionPage(1, [action({ started: "2026-09-26T04:59:59Z" })]) }, /predates requestedAt/],
  ["finish before start", (api) => { api.pages[0] = actionPage(1, [action({ finished: "2026-09-26T05:00:00Z" })]) }, /times are inconsistent/],
  ["future finish time", (api) => { api.pages[0] = actionPage(1, [action({ finished: "2026-09-26T05:00:05Z" })]) }, /after capture time/],
]) {
  test(`rejects ${name}`, async () => {
    const api = fixture()
    const input = afterInput(api)
    mutate(api, input)
    await assert.rejects(captureProviderAfter(input), message)
  })
}

test("rejects a noncanonical requestedAt before making a provider request", async () => {
  const api = fixture()
  await assert.rejects(captureProviderAfter(afterInput(api, { requestedAt: "2026-09-26T05:00:00Z" })), /canonical/)
  assert.deepEqual(api.requests, [])
})

test("rejects a running rebuild action", async () => {
  const api = fixture({ pages: [actionPage(1, [action({ status: "running", finished: null })])] })
  await assert.rejects(captureProviderAfter(afterInput(api)), /incomplete/)
})

test("rejects an errored rebuild action without retaining its error body", async () => {
  const api = fixture({ pages: [actionPage(1, [action({ status: "error", error: { code: "private", message: "DO-NOT-RETAIN-ERROR" } })])] })
  await assert.rejects(captureProviderAfter(afterInput(api)), /failed/)
})

test("rejects a nominally successful action with incomplete evidence fields", async () => {
  const incomplete = action()
  delete incomplete.error
  const api = fixture({ pages: [actionPage(1, [incomplete])] })
  await assert.rejects(captureProviderAfter(afterInput(api)), /has an error/)
})

test("rejects a missing action and duplicate action IDs in server history", async () => {
  const missing = fixture({ pages: [actionPage(1, [])] })
  await assert.rejects(captureProviderAfter(afterInput(missing)), /not found/)
  const duplicate = fixture({ pages: [actionPage(1, [action(), action()])] })
  await assert.rejects(captureProviderAfter(afterInput(duplicate)), /duplicate action/)
})

test("finds the action on a later bounded history page", async () => {
  const api = fixture({ pages: [actionPage(1, [], 2), actionPage(2, [action()], 2)] })
  const capture = await captureProviderAfter(afterInput(api))
  assert.equal(capture.action.actionId, "88")
  assert.deepEqual(api.requests.filter(({ url }) => url.pathname.endsWith("/actions"))
    .map(({ url }) => url.search), ["?page=1&per_page=50", "?page=2&per_page=50"])
})

test("rejects history pagination beyond the fixed page limit", async () => {
  const api = fixture({ pages: [actionPage(1, [], 21)] })
  await assert.rejects(captureProviderAfter(afterInput(api)), /pagination is incomplete or outside the limit/)
  assert.equal(api.requests.length, 2)
})

test("rejects a server whose identity or image changes during the action read", async () => {
  const api = fixture({ serverViews: [server(), server({ location: { id: 7 } })] })
  await assert.rejects(captureProviderAfter(afterInput(api)), /identity or image changed/)
})

test("rejects a server image change after the action history read", async () => {
  const api = fixture({ serverViews: [server(), server({ image: { id: 999 } })] })
  await assert.rejects(captureProviderAfter(afterInput(api)), /identity or image changed/)
})

test("redirects fail without following or retaining provider response text", async () => {
  let seen
  const fetchImpl = async (_url, init) => {
    seen = init
    throw new TypeError("DO-NOT-RETAIN-REDIRECT-DETAILS")
  }
  await assert.rejects(captureProviderBefore({ serverId: "123", token: TOKEN, fetchImpl }), /request failed/)
  assert.equal(seen.redirect, "error")
  assert.equal(seen.method, "GET")
})

test("provider requests have a bounded timeout", async () => {
  const fetchImpl = () => new Promise(() => {})
  await assert.rejects(captureProviderBefore({ serverId: "123", token: TOKEN, fetchImpl, timeoutMs: 10 }), /timed out/)
})

test("oversized provider response bodies are rejected", async () => {
  const fetchImpl = async () => new Response("x".repeat(1024 * 1024 + 1), { status: 200 })
  await assert.rejects(captureProviderBefore({ serverId: "123", token: TOKEN, fetchImpl }), /size limit/)
})

test("external evidence output is private, exclusive, and rejects symlink and repository paths", async (t) => {
  const root = await realpath(await mkdtemp(join(tmpdir(), "chariox-path1-provider-capture-")))
  t.after(() => rm(root, { recursive: true, force: true }))
  const repository = join(root, "repo")
  const evidence = join(root, "evidence")
  await mkdir(repository)
  await mkdir(evidence)
  const output = join(evidence, "capture.json")
  const capture = { schema: "fixture" }

  await assert.rejects(writeCaptureOutput(join(repository, "capture.json"), capture, repository), /outside the repository/)
  await writeCaptureOutput(output, capture, repository)
  assert.equal((await stat(output)).mode & 0o777, 0o600)
  await assert.rejects(writeCaptureOutput(output, capture, repository), /new file/)

  const fileLink = join(evidence, "capture-link.json")
  await symlink(output, fileLink)
  await assert.rejects(writeCaptureOutput(fileLink, capture, repository), /new file/)
  const parentLink = join(evidence, "repo-alias")
  await symlink(repository, parentLink)
  await assert.rejects(writeCaptureOutput(join(parentLink, "capture.json"), capture, repository), /symlink/)
})
