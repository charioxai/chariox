import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const script = await readFile(new URL("./live-docker-slice-browser-state-drill.mjs", import.meta.url), "utf8")

function loadRemoveContainerAndHomeVolume(docker) {
  const match = script.match(/^async function removeContainerAndHomeVolume\(\) \{[\s\S]*?^\}/m)
  assert.ok(match, "browser-state drill must define its container and home-volume removal operation")
  return new Function("docker", "containerName", "homeVolume", `return ${match[0]}`)(
    docker,
    "chariox-slice-drill-test",
    "chariox-slice-drill-test-home",
  )
}

const expectedCalls = [
  ["rm", "-f", "chariox-slice-drill-test"],
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
    assert.deepEqual(calls, expectedCalls)
  })
}
