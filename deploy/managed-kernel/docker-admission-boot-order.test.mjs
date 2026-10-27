import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

// A tmpfiles D /tmp rule removes existing lock files during --remove --boot.
// Provisioning must follow that cleanup, before ordinary kernels can flock.
test("Docker admission boot provisioning follows tmpfiles cleanup", async () => {
  const unit = await readFile(new URL("chariox-docker-admission-locks.service", import.meta.url), "utf8")
  const dependencies = (key) => [...unit.matchAll(new RegExp(`^${key}=(.*)$`, "gm"))]
    .flatMap((match) => match[1].trim().split(/\s+/))
  assert.ok(dependencies("After").includes("systemd-tmpfiles-setup.service"))
  assert.ok(dependencies("After").includes("local-fs.target"))
  assert.ok(dependencies("Before").includes("basic.target"))
  assert.ok(dependencies("Before").includes("chariox-managed-bootstrap.service"))
})
