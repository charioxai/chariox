import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"
import { fileURLToPath } from "node:url"

const provisioner = fileURLToPath(new URL("./provision-linux-docker-slice.sh", import.meta.url))

test("a configured slice memory limit does not acquire extra swap", async () => {
  const source = await readFile(provisioner, "utf8")

  assert.match(source, /--memory \"\$SLICE_DOCKER_MEMORY\"/)
  assert.match(source, /--memory-swap \"\$SLICE_DOCKER_MEMORY\"/)
})
