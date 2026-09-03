import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"

const directory = path.dirname(fileURLToPath(import.meta.url))
const screenHelper = await readFile(
  path.join(directory, "docker", "slice-screen.sh"),
  "utf8",
)

test("physical input watchdog stays in the kernel-owned process group", () => {
  assert.match(
    screenHelper,
    /timeout --foreground 10s xdotool "\$@"/,
    "GNU timeout must not isolate xdotool from kernel process-group cancellation",
  )
  assert.doesNotMatch(screenHelper, /timeout 10s xdotool "\$@"/)
})
