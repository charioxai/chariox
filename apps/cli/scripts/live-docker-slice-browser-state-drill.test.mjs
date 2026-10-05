import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"
import { cleanupBrowserStateDockerResources } from "./lib/browser-state-drill-cleanup.mjs"

const script = await readFile(new URL("./live-docker-slice-browser-state-drill.mjs", import.meta.url), "utf8")

function loadRemoveContainerAndHomeVolume(docker) {
  const match = script.match(/^async function removeContainerAndHomeVolume\(\) \{[\s\S]*?^\}/m)
  assert.ok(match, "browser-state drill must define its container and home-volume removal operation")
  const labels = { "io.chariox.slice.id": "slice", "io.chariox.slice.owner-kernel-id": "kernel", "io.chariox.slice.owner-machine-id": "machine", "io.chariox.slice.runtime-name": "chariox-slice-drill-test" }
  const container = { Id: "a".repeat(64), Config: { Labels: labels } }
  const volume = { Name: "chariox-slice-drill-test-home", CreatedAt: "created", Driver: "local", Mountpoint: "/fixture", Labels: labels }
  return new Function("docker", "containerName", "homeVolume", "dockerOwnership", "inspectDrillDockerObject", "cleanupBrowserStateDockerResources", `return ${match[0]}`)(
    docker, "chariox-slice-drill-test", volume.Name,
    { runId: "drill-test", containerId: container.Id, labels, volume },
    async kind => kind === "container" ? container : volume, cleanupBrowserStateDockerResources,
  )
}

function loadParseFixturePort() {
  const match = script.match(/^function parseFixturePort\(args = process\.argv\.slice\(2\)\) \{[\s\S]*?^\}/m)
  assert.ok(match, "browser-state drill must parse an explicit fixture port")
  return new Function(`${match[0]}; return parseFixturePort`)()
}

const expectedCalls = [
  ["rm", "-f", "a".repeat(64)],
  ["volume", "rm", "-f", "chariox-slice-drill-test-home"],
]

for (const failedIndex of [1, 2]) {
  test(`restore fails closed when Docker removal ${failedIndex} fails`, async () => {
    const calls = []
    const remove = loadRemoveContainerAndHomeVolume(async args => {
      calls.push(args)
      if (calls.length === failedIndex) throw new Error("simulated removal failure")
      return { code: 0 }
    })

    await assert.rejects(remove(), /simulated removal failure/)
    assert.deepEqual(calls, expectedCalls.slice(0, failedIndex))
  })
}

test("fixture port defaults compatibly and accepts an explicit TCP port", () => {
  const parseFixturePort = loadParseFixturePort()

  assert.equal(parseFixturePort([]), 4321)
  assert.equal(parseFixturePort(["--fixture-port", "1"]), 1)
  assert.equal(parseFixturePort(["--fixture-port", "15432"]), 15432)
  assert.equal(parseFixturePort(["--fixture-port", "65535"]), 65535)
})

test("fixture port rejects malformed values and ports outside the TCP range", () => {
  const parseFixturePort = loadParseFixturePort()

  for (const value of ["", "0", "-1", "65536", "1.5", " 4321", "4321 ", "04321", "port"]) {
    assert.throws(() => parseFixturePort(["--fixture-port", value]), /fixture port/i, value || "empty port")
  }
  assert.throws(() => parseFixturePort(["--fixture-port"]), /fixture port/i)
  assert.throws(() => parseFixturePort(["--other-option", "4321"]), /fixture port/i)
})

test("selected fixture port reaches the fixture, slice proxy, cleanup, and evidence", () => {
  assert.match(script, /const fixturePort = parseFixturePort\(\)/)
  assert.match(script, /runId, port: fixturePort, account: email, password,/)
  assert.match(script, /host: "0\.0\.0\.0",\s*port: fixturePort,/)
  assert.match(script, /port: fixturePort, upstreamHost, upstreamPort: fixturePort,/)
  assert.match(script, /kernelPort \+ 3, fixturePort\]/)
  assert.match(script, /const result = \{\s+fixturePort,\s+dockerAvailable,/)
  assert.match(script, /homeVolume,\s*fixturePort,\s*markers,\s*screenshots,\s*cleanup: cleanupResult,/)
  assert.match(script, /async function writeManifest\(ok, error = null\)[\s\S]*?\s+fixturePort,/)
})
