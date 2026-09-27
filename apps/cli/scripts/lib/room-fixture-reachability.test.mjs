import assert from "node:assert/strict"
import { createServer } from "node:http"
import { spawnSync } from "node:child_process"
import test from "node:test"
import { probeRoomFixture, roomFixtureProbeCommand } from "./room-fixture-reachability.mjs"

test("fixture probe distinguishes HTTP access from the expected page without returning page contents", async () => {
  const server = createServer((_request, response) => response.end("READY synthetic-private-value"))
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve))
  try {
    const url = `http://127.0.0.1:${server.address().port}`
    assert.deepEqual(await probeRoomFixture(url, "READY"), { reachable: true, status: 200, markerPresent: true })
    assert.deepEqual(await probeRoomFixture(url, "MISSING"), { reachable: true, status: 200, markerPresent: false })
  } finally {
    await new Promise(resolve => server.close(resolve))
  }
})

test("serialized in-slice probe reports a refused connection without leaking the URL", () => {
  const [command, ...args] = roomFixtureProbeCommand("http://127.0.0.1:1/private-value", "READY")
  const result = spawnSync(command, args, { encoding: "utf8", timeout: 10_000 })
  assert.equal(result.status, 0, result.stderr)
  assert.equal(JSON.parse(result.stdout).reachable, false)
  assert.doesNotMatch(result.stdout, /private-value|127\.0\.0\.1/)
})
