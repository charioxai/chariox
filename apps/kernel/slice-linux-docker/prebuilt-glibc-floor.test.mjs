import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { readFile } from "node:fs/promises"
import test from "node:test"
import { fileURLToPath } from "node:url"

const checker = fileURLToPath(new URL("./docker/check-prebuilt-glibc.sh", import.meta.url))
const dockerfilePath = fileURLToPath(new URL("./docker/Dockerfile", import.meta.url))
// Any dynamically linked host binary references glibc symbol versions.
const hostBinary = process.execPath

const check = (floor) => spawnSync("bash", [checker, hostBinary], {
  encoding: "utf8",
  env: { ...process.env, CHARIOX_GLIBC_FLOOR: floor },
})

test("prebuilt binaries needing a newer glibc than the slice image are rejected", () => {
  const result = check("2.0")
  assert.equal(result.status, 1, result.stderr)
  assert.match(result.stderr, /needs glibc \d+\.\d+\S* but the slice image has glibc 2\.0/)
  assert.match(result.stderr, /--target managed-release-artifacts/)
})

test("prebuilt binaries within the slice image glibc are accepted", () => {
  const result = check("99.0")
  assert.equal(result.status, 0, result.stderr)
})

test("the prebuilt runtime install checks the glibc floor before installing", async () => {
  const dockerfile = await readFile(dockerfilePath, "utf8")
  const check = dockerfile.indexOf("bash apps/kernel/slice-linux-docker/docker/check-prebuilt-glibc.sh /opt/chariox-prebuilt/chariox-kernel /opt/chariox-prebuilt/chariox-relay")
  assert.ok(check > 0, "Dockerfile does not check prebuilt binaries")
  assert.ok(check < dockerfile.indexOf("install -m 0755 /opt/chariox-prebuilt/chariox-kernel"))
})
