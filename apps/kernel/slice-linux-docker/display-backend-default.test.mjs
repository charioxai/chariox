import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const screen = new URL("./docker/slice-screen.sh", import.meta.url)
const provisioner = new URL("./provision-linux-docker-slice.sh", import.meta.url)
const lifecycleDrill = new URL("../../cli/scripts/live-slice-lifecycle-drill.mjs", import.meta.url)

test("slice runtime defaults omitted viewer backend to Selkies", async () => {
  const [screenSource, provisionerSource] = await Promise.all([
    readFile(screen, "utf8"),
    readFile(provisioner, "utf8"),
  ])

  assert.match(screenSource, /VIEWER_BACKEND="\$\{CHARIOX_SLICE_VIEWER_BACKEND:-selkies\}"/)
  assert.doesNotMatch(provisionerSource, /CHARIOX_SLICE_VIEWER_BACKEND:-novnc/)
  assert.match(provisionerSource, /CHARIOX_SLICE_VIEWER_BACKEND:-selkies/)
})

test("slice runtime retains explicit noVNC rollback handling", async () => {
  const screenSource = await readFile(screen, "utf8")
  assert.match(screenSource, /novnc\|selkies/)
  assert.match(screenSource, /nohup x11vnc/)
})

test("live slice lifecycle verifies the intended Selkies backend", async () => {
  const source = await readFile(lifecycleDrill, "utf8")
  assert.match(source, /async function assertScreenEndpointReady\(url, expectedBackend\)/)
  assert.match(source, /if \(expectedBackend === 'novnc'\)/)
  assert.match(source, /else if \(expectedBackend === 'selkies'\)/)
  assert.match(source, /assert\(endpoint && endpoint\.kind === 'selkies'/)
  assert.match(source, /assertScreenEndpointReady\(endpoint\.url, endpoint\.kind\)/)
})
