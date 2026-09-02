import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"
import { fileURLToPath } from "node:url"

const kernelManifest = fileURLToPath(new URL("../Cargo.toml", import.meta.url))
const dockerfilePath = fileURLToPath(new URL("./docker/Dockerfile", import.meta.url))
const provisionerPath = fileURLToPath(new URL("./provision-linux-docker-slice.sh", import.meta.url))
const selkiesLifecycleTestPath = fileURLToPath(new URL("./docker/test_slice_selkies.py", import.meta.url))

test("slice images copy and fingerprint every kernel package path dependency", async () => {
  const [manifest, dockerfile, provisioner] = await Promise.all([
    readFile(kernelManifest, "utf8"),
    readFile(dockerfilePath, "utf8"),
    readFile(provisionerPath, "utf8"),
  ])
  const dependencies = [...manifest.matchAll(/path\s*=\s*"\.\.\/\.\.\/packages\/([^"/]+)"/g)]
    .map((match) => match[1])
  assert.ok(dependencies.length > 0, "fixture should discover at least one local package dependency")
  for (const dependency of dependencies) {
    assert.match(dockerfile, new RegExp(`packages/${dependency}\\s+(?:/opt/chariox-source/)?packages/${dependency}`))
    assert.match(provisioner, new RegExp(`packages/${dependency}`))
  }
})

test("slice image build runs the discoverable Selkies lifecycle tests", async () => {
  const [dockerfile, lifecycleTest] = await Promise.all([
    readFile(dockerfilePath, "utf8"),
    readFile(selkiesLifecycleTestPath, "utf8"),
  ])

  assert.match(lifecycleTest, /class SelkiesStopTests/)
  assert.match(dockerfile, /python[^\n]*-m unittest discover\s*\\?\s*-s [^\n]* -p 'test_\*\.py'/)
})
