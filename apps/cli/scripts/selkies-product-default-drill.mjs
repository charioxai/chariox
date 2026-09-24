import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"

import {
  createSliceRequest,
} from "../../../packages/kernel-client/dist/ipc-slice-requests.js"

const headed = createSliceRequest({ name: "default-headed", displayMode: "headed" })
assert.equal(headed.CreateSlice.display_backend, "selkies")

const rollback = createSliceRequest({
  name: "rollback-headed",
  displayMode: "headed",
  displayBackend: "novnc",
})
assert.equal(rollback.CreateSlice.display_backend, "novnc")

const headless = createSliceRequest({ name: "headless", displayMode: "headless" })
assert.equal(Object.hasOwn(headless.CreateSlice, "display_backend"), false)

const [screen, provisioner] = await Promise.all([
  readFile(new URL("../../kernel/slice-linux-docker/docker/slice-screen.sh", import.meta.url), "utf8"),
  readFile(new URL("../../kernel/slice-linux-docker/provision-linux-docker-slice.sh", import.meta.url), "utf8"),
])
assert.match(screen, /CHARIOX_SLICE_VIEWER_BACKEND:-selkies/)
assert.doesNotMatch(provisioner, /CHARIOX_SLICE_VIEWER_BACKEND:-novnc/)
assert.match(screen, /novnc\|selkies/)

process.stdout.write("Selkies product-default protocol drill passed\n")
